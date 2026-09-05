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
//! Everything here is computed from the [`PlanView`] and the [`Trace`] alone.

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
fn locks(n: &sdax::NodeView, attr: &str) -> Vec<String> {
    n.attr(attr)
        .map(|s| s.split(", ").map(|x| x.to_string()).collect())
        .unwrap_or_default()
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

/// `[start, end)` trace-index intervals a node held a grant, one per attempt.
/// A service keeps its grant past `Ready` (contract § 1, T1), until its stop.
fn grant_spans(trace: &Trace, path: &NodePath, service: bool) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut open: Option<usize> = None;
    for (i, e) in trace.events.iter().enumerate() {
        if e.node.as_ref() != Some(path) {
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

    // MUTEX: every pair of nodes naming one resource, in any scope — the view
    // resolves an imported lock to the parent's path, so a child node and a
    // parent node contending for one resource are one pair.
    for (i, a) in view.nodes.iter().enumerate() {
        let (ax, ash) = (locks(a, "exclusive"), locks(a, "shared"));
        if ax.is_empty() && ash.is_empty() {
            continue;
        }
        for b in view.nodes.iter().skip(i + 1) {
            let (bx, bsh) = (locks(b, "exclusive"), locks(b, "shared"));
            // shared/shared is not a conflict; every other pairing is.
            let conflict = ax.iter().any(|r| bx.contains(r) || bsh.contains(r))
                || ash.iter().any(|r| bx.contains(r));
            if !conflict {
                continue;
            }
            let sa = grant_spans(trace, &a.path, is_service(&a.path));
            let sb = grant_spans(trace, &b.path, is_service(&b.path));
            for x in &sa {
                for y in &sb {
                    if overlaps(*x, *y) {
                        out.push(violation(
                            "MUTEX",
                            format!(
                                "{} and {} hold the same resource at once ({:?} and {:?})",
                                a.path, b.path, x, y
                            ),
                        ));
                    }
                }
            }
        }
    }

    // POOL: every pool of every scope, its own users only.
    for pool in &view.pools {
        let mut spans: Vec<(usize, usize, &NodePath)> = Vec::new();
        for u in &pool.users {
            for (s, e) in grant_spans(trace, u, is_service(u)) {
                spans.push((s, e, u));
            }
        }
        for at in 0..trace.events.len() {
            let inside: Vec<&NodePath> = spans
                .iter()
                .filter(|(s, e, _)| *s <= at && at < *e)
                .map(|(_, _, u)| *u)
                .collect();
            if inside.len() > pool.limit {
                out.push(violation(
                    "POOL",
                    format!(
                        "pool {} (limit {}) holds {} at trace index {at}: {:?}",
                        pool.name,
                        pool.limit,
                        inside.len(),
                        inside.iter().map(|p| p.to_string()).collect::<Vec<_>>()
                    ),
                ));
                break;
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
/// so an inner scope that settled for a reason of its own was unchecked.
pub fn check_scopes(trace: &Trace, view: &PlanView) -> Vec<Violation> {
    let mut out = Vec::new();
    let evs = &trace.events;
    let ended_badly = |p: &NodePath| {
        evs.iter().any(|e| {
            e.node.as_ref() == Some(p)
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
        match &e.kind {
            TraceKind::Skipped { because } => {
                if let Some(b) = because {
                    if !ended_badly(b) {
                        out.push(violation(
                            "SKIPPED",
                            format!("{path} was skipped because of {b}, which did not end badly"),
                        ));
                    }
                }
                // T4: a skipped node has no attempt in flight. One whose
                // earlier attempt failed and is waiting for its next may be
                // skipped — the trace numbers both attempts the same, so the
                // check is "the last `Start` before the skip already ended".
                let last_start = evs[..i]
                    .iter()
                    .rposition(|x| x.node.as_ref() == Some(path) && is_start(&x.kind));
                if let Some(sp) = last_start {
                    let ended = evs[sp + 1..i]
                        .iter()
                        .any(|x| x.node.as_ref() == Some(path) && ends_grant(&x.kind));
                    if !ended {
                        out.push(violation(
                            "SKIPPED",
                            format!("{path} was skipped with attempt #{sp} still in flight"),
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
                    x.node.as_ref() == Some(path) && matches!(x.kind, TraceKind::StopRequested)
                });
                if requested {
                    continue;
                }
                // The scope it ends is its own, and the trace says so
                // differently at each level. The root emits `Settling`. An
                // inner scope emits none, and its release cannot open until
                // INV-5 lets it, so the observable consequence there is T5:
                // nothing of that scope starts afterwards.
                let scope = scope_of(path);
                if scope.segments().is_empty() {
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
                    let prefix = format!("{scope}/");
                    for (j, x) in evs.iter().enumerate().skip(i + 1) {
                        let Some(q) = &x.node else { continue };
                        let text = q.to_string();
                        if is_start(&x.kind) && text.starts_with(&prefix) && scope_of(q) == scope {
                            out.push(violation(
                                "TERMINAL",
                                format!(
                                    "{q} started at #{j}, after terminal service {path} ended its scope"
                                ),
                            ));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // T5-INNER.
    for n in view.nodes.iter().filter(|n| n.kind == Kind::Component) {
        let Some(end) = evs.iter().position(|e| {
            e.node.as_ref() == Some(&n.path)
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
        for (i, e) in evs.iter().enumerate().skip(end + 1) {
            let Some(p) = &e.node else { continue };
            if is_start(&e.kind) && p.to_string().starts_with(&format!("{}/", n.path)) {
                out.push(violation(
                    "T5-INNER",
                    format!(
                        "{p} started at #{i}, after component {}'s attempt ended at #{end}",
                        n.path
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
