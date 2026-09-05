//! The clauses `trace.rs` did not recompute: lock and pool **exclusion**,
//! `Skipped`, `terminal`, inner-scope settling, and the non-empty `why`.
//!
//! `dev-docs/Review-Stage1-Semantics.md` § 5.1 found each of these unchecked,
//! which is how F-01 survived a 200 000-case walk: the walk *reached* two pools
//! with a waiter on each, and had no rule that could fail on the answer. The
//! "exclusive contention" and "pool wait" coverage floors were counted from the
//! machine's own `why`, so they measured that the machine *said* it waited, not
//! that the arbitration was right.
//!
//! Everything here is computed from the [`PlanView`] and the [`Trace`] alone,
//! and everything is per **copy** ([`Occ`]): two instances of one template
//! contending for a key they both import is one pair to check, while two
//! instances each holding *their own* copy of a lock is no pair at all. A rule
//! that compared paths would get both backwards.

use super::occ::{is_static, label, occ_of, occ_of_scope, occ_of_target, occurrences, Occ};
use super::{violation, Violation};
use crate::WhyAt;
use sdax::{Kind, NodePath, PlanView, Trace, TraceKind};

/// A body of `path`, attempt `k`, is inside its lock/pool grant from its
/// `Start` until whichever of these ends the attempt.
fn ends_grant(k: &TraceKind) -> bool {
    matches!(
        k,
        TraceKind::Ready
            | TraceKind::Fail(_, _)
            | TraceKind::Interrupted { .. }
            | TraceKind::Ambiguous
            | TraceKind::Abandoned
            | TraceKind::Skipped { .. }
    )
}

/// The resources a node names, by the paths the view resolved them to. A lock
/// on an imported resource names the parent's node, so two scopes contending
/// for one resource are comparable.
fn locks(view: &PlanView, n: &sdax::NodeView, attr: &str) -> Vec<NodePath> {
    n.attr(attr)
        .map(|s| s.split(", ").map(|x| path_of(view, x)).collect())
        .unwrap_or_default()
}

/// A rendered path, back as the [`NodePath`] it renders.
///
/// Looked up among the plan's own nodes rather than split on `/`: a node's
/// *name* may contain a slash, so `"A/B/C"` can be two segments or three and
/// only the plan knows which. Splitting made two instances' own copies of one
/// resource look like one shared lock and `MUTEX` fired on a run that was
/// perfectly arbitrated (`monte_carlo_big` seed 12339652235566683353).
fn path_of(view: &PlanView, s: &str) -> NodePath {
    if let Some(n) = view.nodes.iter().find(|n| n.path.to_string() == s) {
        return n.path.clone();
    }
    let mut segs = s.split('/');
    let mut p = match segs.next() {
        Some(first) => NodePath::root(first),
        None => NodePath::default(),
    };
    for s in segs {
        p = p.child(s);
    }
    p
}

/// The scope a node lives in: its path without the last segment. The root
/// scope is the empty path.
fn scope_of(p: &NodePath) -> NodePath {
    let segs = p.segments();
    let mut out = NodePath::default();
    for s in &segs[..segs.len().saturating_sub(1)] {
        out = out.child(s);
    }
    out
}

/// `[start, end)` trace-index intervals one **copy** of a node held a grant,
/// one per attempt. A service keeps its grant past `Ready` (contract § 1, T1),
/// until its stop.
fn grant_spans(trace: &Trace, path: &NodePath, occ: &Occ, service: bool) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut open: Option<usize> = None;
    for (i, e) in trace.events.iter().enumerate() {
        if e.node.as_ref() != Some(path) || occ_of(e) != *occ {
            continue;
        }
        match &e.kind {
            TraceKind::Start(_) => open = Some(i),
            k if ends_grant(k) => {
                // A service's grant outlives its `Ready`.
                if service && matches!(k, TraceKind::Ready) {
                    continue;
                }
                if let Some(s) = open.take() {
                    out.push((s, i));
                }
            }
            TraceKind::Stopped | TraceKind::ReleaseFail(_) | TraceKind::Fail(_, _) if service => {
                if let Some(s) = open.take() {
                    out.push((s, i));
                }
            }
            _ => {}
        }
    }
    if let Some(s) = open {
        out.push((s, trace.events.len()));
    }
    out
}

fn overlaps(a: (usize, usize), b: (usize, usize)) -> bool {
    a.0 < b.1 && b.0 < a.1
}

/// `MUTEX` — INV-1's lock clause: two bodies that name the same resource never
/// hold it at once unless both named it `shared`.
///
/// `POOL` — INV-1's pool clause: no more than `limit` bodies of a pool are
/// inside it at any point of the trace.
pub fn check_arbitration(trace: &Trace, view: &PlanView) -> Vec<Violation> {
    let mut out = Vec::new();
    let is_service = |p: &NodePath| {
        view.nodes
            .iter()
            .any(|n| n.path == *p && n.kind == Kind::Service)
    };
    // One entry per live copy that names a lock: the copy, and the *resolved*
    // copy of each resource it names, exclusively or shared.
    struct User {
        path: NodePath,
        occ: Occ,
        exclusive: Vec<(NodePath, Occ)>,
        shared: Vec<(NodePath, Occ)>,
    }
    let mut users: Vec<User> = Vec::new();
    for (path, occ) in occurrences(trace, view) {
        let Some(n) = view.nodes.iter().find(|n| n.path == path) else {
            continue;
        };
        let resolve = |rs: Vec<NodePath>| -> Vec<(NodePath, Occ)> {
            rs.into_iter()
                .map(|r| {
                    let o = occ_of_target(&path, &occ, &r);
                    (r, o)
                })
                .collect()
        };
        let exclusive = resolve(locks(view, n, "exclusive"));
        let shared = resolve(locks(view, n, "shared"));
        if exclusive.is_empty() && shared.is_empty() {
            continue;
        }
        users.push(User {
            path,
            occ,
            exclusive,
            shared,
        });
    }

    // MUTEX: every pair of copies naming one copy of a resource. Two instances
    // of one template are a pair when the resource is one they both import,
    // and are not a pair when each holds its own.
    for i in 0..users.len() {
        for j in i + 1..users.len() {
            let (a, b) = (&users[i], &users[j]);
            let conflict = a
                .exclusive
                .iter()
                .any(|r| b.exclusive.contains(r) || b.shared.contains(r))
                || a.shared.iter().any(|r| b.exclusive.contains(r));
            if !conflict {
                continue;
            }
            let sa = grant_spans(trace, &a.path, &a.occ, is_service(&a.path));
            let sb = grant_spans(trace, &b.path, &b.occ, is_service(&b.path));
            for x in &sa {
                for y in &sb {
                    if overlaps(*x, *y) {
                        out.push(violation(
                            "MUTEX",
                            format!(
                                "{} and {} hold the same resource at once ({:?} and {:?})",
                                label(&a.path, &a.occ),
                                label(&b.path, &b.occ),
                                x,
                                y
                            ),
                        ));
                    }
                }
            }
        }
    }

    // POOL: every pool of every *copy* of every scope, its own users only. A
    // pool declared in a template's plan is one pool per instance.
    for pool in &view.pools {
        let depth = pool.scope.segments().len();
        let mut spans: Vec<(usize, usize, String, Occ)> = Vec::new();
        for u in &pool.users {
            for (path, occ) in occurrences(trace, view) {
                if path != *u {
                    continue;
                }
                let key = occ_of_scope(&occ, depth);
                for (s, e) in grant_spans(trace, &path, &occ, is_service(&path)) {
                    spans.push((s, e, label(&path, &occ), key.clone()));
                }
            }
        }
        let mut keys: Vec<Occ> = Vec::new();
        for (_, _, _, k) in &spans {
            if !keys.contains(k) {
                keys.push(k.clone());
            }
        }
        for k in keys {
            for at in 0..trace.events.len() {
                let inside: Vec<&str> = spans
                    .iter()
                    .filter(|(s, e, _, key)| *key == k && *s <= at && at < *e)
                    .map(|(_, _, u, _)| u.as_str())
                    .collect();
                if inside.len() > pool.limit {
                    out.push(violation(
                        "POOL",
                        format!(
                            "pool {} (limit {}) holds {} at trace index {at}: {:?}",
                            pool.name,
                            pool.limit,
                            inside.len(),
                            inside
                        ),
                    ));
                    break;
                }
            }
        }
    }
    out
}

/// `SKIPPED` — T4: the node a `Skipped{because}` names really did end badly,
/// and a skipped node never started.
///
/// `TERMINAL` — a `terminal` service that finished with no stop request ends
/// its scope: the root's `Settling` follows at the same instant. Without this
/// rule `OD-BACKSTOP`'s shutdown at 20–25 s ended the run instead and the case
/// passed.
///
/// `T5-INNER` — nothing starts inside a component after that component's own
/// attempt ended. `trace.rs` checks T5 against the **root**'s `Settling` only,
/// so an inner scope that settled for a reason of its own was unchecked. The
/// instance form of the same rule is `T5-INSTANCE`, in `containment.rs`.
pub fn check_scopes(trace: &Trace, view: &PlanView) -> Vec<Violation> {
    let mut out = Vec::new();
    let evs = &trace.events;
    let ended_badly = |p: &NodePath, o: &Occ| {
        evs.iter().any(|e| {
            e.node.as_ref() == Some(p)
                && occ_of(e) == *o
                && matches!(
                    e.kind,
                    TraceKind::Fail(_, _)
                        | TraceKind::Skipped { .. }
                        | TraceKind::Interrupted { .. }
                        | TraceKind::Ambiguous
                        | TraceKind::Abandoned
                )
        })
    };
    for (i, e) in evs.iter().enumerate() {
        let Some(path) = &e.node else { continue };
        let occ = occ_of(e);
        match &e.kind {
            TraceKind::Skipped { because } => {
                if let Some(b) = because {
                    let bocc = occ_of_target(path, &occ, b);
                    if !ended_badly(b, &bocc) {
                        out.push(violation(
                            "SKIPPED",
                            format!(
                                "{} was skipped because of {}, which did not end badly",
                                label(path, &occ),
                                label(b, &bocc)
                            ),
                        ));
                    }
                }
                // T4: a skipped node has no attempt in flight. One whose
                // earlier attempt failed and is waiting for its next may be
                // skipped — the trace numbers both attempts the same, so the
                // check is "the last `Start` before the skip already ended".
                let last_start = evs[..i].iter().rposition(|x| {
                    x.node.as_ref() == Some(path) && occ_of(x) == occ && is_start(&x.kind)
                });
                if let Some(sp) = last_start {
                    let ended = evs[sp + 1..i].iter().any(|x| {
                        x.node.as_ref() == Some(path) && occ_of(x) == occ && ends_grant(&x.kind)
                    });
                    if !ended {
                        out.push(violation(
                            "SKIPPED",
                            format!(
                                "{} was skipped with attempt #{sp} still in flight",
                                label(path, &occ)
                            ),
                        ));
                    }
                }
            }
            TraceKind::Stopped => {
                let Some(n) = view.nodes.iter().find(|n| n.path == *path) else {
                    continue;
                };
                if n.kind != Kind::Service || n.attr("terminal") != Some("yes") {
                    continue;
                }
                let requested = evs[..i].iter().any(|x| {
                    x.node.as_ref() == Some(path)
                        && occ_of(x) == occ
                        && matches!(x.kind, TraceKind::StopRequested)
                });
                if requested {
                    continue;
                }
                // The scope it ends is its own, and the trace says so
                // differently at each level. The root emits `Settling`. An
                // inner scope — a component's or an instance's — emits none,
                // and its release cannot open until INV-5 lets it, so the
                // observable consequence there is T5: nothing of that scope
                // starts afterwards.
                let scope = scope_of(path);
                if scope.segments().is_empty() && is_static(&occ) {
                    let settled = evs
                        .iter()
                        .any(|x| matches!(x.kind, TraceKind::Settling) && x.at <= e.at);
                    if !settled {
                        out.push(violation(
                            "TERMINAL",
                            format!("terminal service {path} finished and the run did not settle"),
                        ));
                    }
                } else {
                    for (j, x) in evs.iter().enumerate().skip(i + 1) {
                        let Some(q) = &x.node else { continue };
                        let qocc = occ_of(x);
                        if is_start(&x.kind)
                            && scope_of(q) == scope
                            && occ_of_scope(&qocc, scope.segments().len())
                                == occ_of_scope(&occ, scope.segments().len())
                        {
                            out.push(violation(
                                "TERMINAL",
                                format!(
                                    "{} started at #{j}, after terminal service {} ended its scope",
                                    label(q, &qocc),
                                    label(path, &occ)
                                ),
                            ));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // T5-INNER, per copy of the component.
    for (path, occ) in occurrences(trace, view) {
        let Some(n) = view.nodes.iter().find(|n| n.path == path) else {
            continue;
        };
        if n.kind != Kind::Component {
            continue;
        }
        let Some(end) = evs.iter().position(|e| {
            e.node.as_ref() == Some(&path)
                && occ_of(e) == occ
                && matches!(
                    e.kind,
                    TraceKind::Fail(_, _)
                        | TraceKind::Interrupted { .. }
                        | TraceKind::Skipped { .. }
                        | TraceKind::ReleaseStart
                )
        }) else {
            continue;
        };
        let depth = path.segments().len();
        for (i, e) in evs.iter().enumerate().skip(end + 1) {
            let Some(p) = &e.node else { continue };
            let po = occ_of(e);
            if is_start(&e.kind)
                && p.segments().len() > depth
                && p.segments().starts_with(path.segments())
                && occ_of_scope(&po, depth) == occ_of_scope(&occ, depth)
            {
                out.push(violation(
                    "T5-INNER",
                    format!(
                        "{} started at #{i}, after component {} ended at #{end}",
                        label(p, &po),
                        label(&path, &occ)
                    ),
                ));
            }
        }
    }
    out
}

fn is_start(k: &TraceKind) -> bool {
    matches!(k, TraceKind::Start(_))
}

/// `WHY` — contract § 2: a `Waiting` node's `on` "lists each reason". An empty
/// one is a delay the run cannot explain, which is how F-01 was silent.
pub fn check_whys(whys: &[WhyAt]) -> Vec<Violation> {
    let mut out = Vec::new();
    for (at, node, on) in whys {
        if on.is_empty() {
            out.push(violation(
                "WHY",
                format!("{node} is Waiting at t={at} and names no reason"),
            ));
            break;
        }
    }
    out
}
