//! The trace-level invariant checker: the second opinion.
//!
//! Recomputes every checkable clause of INV-1…20 and `orphans: none` from
//! the plan view and the trace alone. It never reads the machine: a checker
//! that trusted the thing it checks would prove nothing.
//!
//! [`check_trace_prefix`] holds at every prefix of a run and is what the
//! scripted driver runs after every step; [`check_trace`] adds what only
//! holds at `End` and cross-checks the report.
//!
//! Every rule groups events by **[`Occ`]** — the copy of a declaration, path
//! plus instance chain — and never by path alone, so a template's instances
//! are as thoroughly checked as the static graph and never merged into it.

use super::occ::{label, occ_of, occ_of_target, occurrences, static_occ, Occ};
use super::{violation, Violation};
use sdax::{Kind, NodePath, NodeView, Phase, PlanView, Trace, TraceEvent, TraceKind};

pub(super) struct Ctx<'a> {
    pub trace: &'a Trace,
    pub view: &'a PlanView,
}

pub(super) fn attempt(e: &TraceEvent) -> u32 {
    e.order.as_ref().map(|o| o.attempt).unwrap_or(0)
}

pub(super) fn is_body_end(k: &TraceKind) -> bool {
    matches!(
        k,
        TraceKind::Ready
            | TraceKind::Fail(Phase::Prepare | Phase::Run, _)
            | TraceKind::Interrupted { .. }
            | TraceKind::Ambiguous
            | TraceKind::Abandoned
    )
}

pub(super) fn is_cleanup_start(k: &TraceKind) -> bool {
    matches!(
        k,
        TraceKind::ReleaseStart | TraceKind::CompensateStart | TraceKind::StopRequested
    )
}

pub(super) fn is_cleanup_end(k: &TraceKind) -> bool {
    matches!(
        k,
        TraceKind::ReleaseOk
            | TraceKind::ReleaseFail(_)
            | TraceKind::CompensateOk
            | TraceKind::CompensateFail(_)
            | TraceKind::Stopped
            | TraceKind::Fail(Phase::Stop, _)
            | TraceKind::Abandoned
    )
}

impl<'a> Ctx<'a> {
    pub fn node(&self, path: &NodePath) -> Option<&'a NodeView> {
        self.view.nodes.iter().find(|n| n.path == *path)
    }

    /// `(index, event)` for one **copy** of a node, in trace order.
    pub fn of(&self, path: &NodePath, occ: &Occ) -> Vec<(usize, &'a TraceEvent)> {
        self.trace
            .events
            .iter()
            .enumerate()
            .filter(|(_, e)| e.node.as_ref() == Some(path) && occ_of(e) == *occ)
            .collect()
    }

    /// Every copy of `path` the trace observed, plus its static one.
    pub fn copies(&self, path: &NodePath) -> Vec<Occ> {
        let mut out = vec![static_occ(path)];
        for e in &self.trace.events {
            if e.node.as_ref() != Some(path) {
                continue;
            }
            let occ = occ_of(e);
            if !out.contains(&occ) {
                out.push(occ);
            }
        }
        out
    }

    pub fn persistent(n: &NodeView) -> bool {
        n.attr("release") == Some("persistent")
    }

    pub fn owes_when_held(n: &NodeView) -> bool {
        matches!(n.kind, Kind::Resource | Kind::Effect) && !Self::persistent(n)
    }

    /// Whether this copy of `path` is finished with everything before index
    /// `r`: its last attempt ended and, if it held or served or ran an inner
    /// scope, that obligation ended too (the INV-5 clause "M's cleanup has
    /// ended").
    pub fn cleanup_ended_by(&self, path: &NodePath, occ: &Occ, r: usize) -> bool {
        let Some(n) = self.node(path) else {
            return true;
        };
        let evs: Vec<&TraceEvent> = self
            .of(path, occ)
            .into_iter()
            .filter(|(i, _)| *i < r)
            .map(|(_, e)| e)
            .collect();
        let Some(last_start) = evs
            .iter()
            .rposition(|e| matches!(e.kind, TraceKind::Start(_)))
        else {
            return true;
        };
        let after: Vec<&TraceEvent> = evs[last_start..].to_vec();
        let k = attempt(after[0]);
        if !after
            .iter()
            .any(|e| is_body_end(&e.kind) && attempt(e) == k)
        {
            return false;
        }
        if after.iter().any(|e| matches!(e.kind, TraceKind::Abandoned)) {
            return true;
        }
        match n.kind {
            Kind::Resource | Kind::Effect => {
                let held = after.iter().any(|e| matches!(e.kind, TraceKind::Held));
                // A *persistent* effect has no compensation to run, so
                // `Ambiguous` is where it ends even under
                // `on_ambiguous(Compensate)`: the declaration says there is
                // nothing to undo, and the view says so too.
                let ambiguous_compensated =
                    after.iter().any(|e| matches!(e.kind, TraceKind::Ambiguous))
                        && n.attr("ambiguous") == Some("compensate")
                        && !Self::persistent(n);
                if (held && Self::owes_when_held(n)) || ambiguous_compensated {
                    after.iter().any(|e| is_cleanup_end(&e.kind))
                } else {
                    true
                }
            }
            Kind::Service => {
                let ready = after
                    .iter()
                    .position(|e| matches!(e.kind, TraceKind::Ready));
                match ready {
                    Some(p) => after[p..].iter().any(|e| {
                        matches!(
                            e.kind,
                            TraceKind::Stopped
                                | TraceKind::Fail(Phase::Stop | Phase::Serve, _)
                                | TraceKind::Abandoned
                        )
                    }),
                    None => true,
                }
            }
            // A component's inner graph *is* its release: `ReleaseOk`, or an
            // abandonment. `Stopped` was accepted here for a shape the machine
            // never emits — every route into an inner cleanup goes through
            // `open_component`, which sets `Releasing` first.
            Kind::Component => after
                .iter()
                .any(|e| matches!(e.kind, TraceKind::ReleaseOk | TraceKind::Abandoned)),
            _ => true,
        }
    }
}

/// The invariants that hold at every prefix of a run.
pub fn check_trace_prefix(trace: &Trace, view: &PlanView) -> Vec<Violation> {
    let c = Ctx { trace, view };
    let mut out = Vec::new();
    let evs = &trace.events;

    // Time never runs backwards along the trace, and End is last.
    for w in evs.windows(2) {
        if w[1].at < w[0].at {
            out.push(violation(
                "TIME",
                format!("{:?} is timestamped before {:?}", w[1], w[0]),
            ));
        }
    }
    if let Some(p) = evs.iter().position(|e| matches!(e.kind, TraceKind::End(_))) {
        if p + 1 != evs.len() {
            out.push(violation("T8", "an event follows End".into()));
        }
    }
    let settling = evs
        .iter()
        .position(|e| matches!(e.kind, TraceKind::Settling));

    for (i, e) in evs.iter().enumerate() {
        let Some(path) = &e.node else { continue };
        let Some(n) = c.node(path) else {
            out.push(violation(
                "VIEW",
                format!("trace names {path}, which the plan does not"),
            ));
            continue;
        };
        let occ = occ_of(e);
        let me = label(path, &occ);
        let k = attempt(e);
        let mine = c.of(path, &occ);
        let before: Vec<&TraceEvent> = mine
            .iter()
            .filter(|(j, _)| *j < i)
            .map(|(_, e)| *e)
            .collect();
        match &e.kind {
            TraceKind::Start(_) => {
                // INV-1: every need is Ready now, and nothing else ordered it.
                for need in &n.needs {
                    let nocc = occ_of_target(path, &occ, need);
                    // A need on a **template** is the per-instance input: it is
                    // available from the spawn of *this* instance and from no
                    // earlier moment (INV-16, INV-17).
                    if c.node(need).map(|m| m.kind) == Some(Kind::Template) {
                        let id = occ.get(need.segments().len() - 1).copied().flatten();
                        let spawned = c.of(need, &nocc).into_iter().any(|(j, x)| {
                            j < i
                                && matches!(x.kind, TraceKind::InstanceSpawned(s) if Some(s) == id)
                        });
                        if !spawned {
                            out.push(violation(
                                "INV-16",
                                format!("{me} started at #{i} before its instance was spawned"),
                            ));
                        }
                        continue;
                    }
                    let last = c.of(need, &nocc).into_iter().rfind(|(j, _)| *j < i);
                    let ready_now = match last {
                        Some((_, le)) => match le.kind {
                            TraceKind::Ready => true,
                            TraceKind::Stopped => !c
                                .of(need, &nocc)
                                .iter()
                                .any(|(j, x)| *j < i && matches!(x.kind, TraceKind::StopRequested)),
                            _ => false,
                        },
                        None => false,
                    };
                    if !ready_now {
                        out.push(violation(
                            "INV-1",
                            format!("{me} started at #{i} while its need {need} is not Ready"),
                        ));
                    }
                }
                if let Some(s) = settling {
                    if i > s {
                        out.push(violation(
                            "T5",
                            format!("{me} started at #{i} after the run settled"),
                        ));
                    }
                }
                if before
                    .iter()
                    .any(|x| matches!(x.kind, TraceKind::Skipped { .. }))
                {
                    out.push(violation("T4", format!("{me} started after being skipped")));
                }
                // INV-12: attempts are 1, 2, 3… and never overlap; a held
                // attempt is released before the next starts.
                let prev: Vec<&TraceEvent> = before
                    .iter()
                    .copied()
                    .filter(|x| matches!(x.kind, TraceKind::Start(_)))
                    .collect();
                let expected = prev.len() as u32 + 1;
                if k != expected {
                    out.push(violation(
                        "INV-12",
                        format!("{me} attempt {k} started where attempt {expected} was expected"),
                    ));
                }
                if let Some(last) = prev.last() {
                    let pk = attempt(last);
                    let ended = before
                        .iter()
                        .any(|x| attempt(x) == pk && is_body_end(&x.kind));
                    if !ended {
                        out.push(violation(
                            "INV-12",
                            format!("{me} attempt {k} started while attempt {pk} was in flight"),
                        ));
                    }
                    let held = before
                        .iter()
                        .any(|x| attempt(x) == pk && matches!(x.kind, TraceKind::Held));
                    let released = before
                        .iter()
                        .any(|x| attempt(x) == pk && is_cleanup_end(&x.kind));
                    if held && Ctx::owes_when_held(n) && !released {
                        out.push(violation(
                            "INV-12",
                            format!("{me} attempt {k} started before attempt {pk}'s release ended"),
                        ));
                    }
                }
            }
            TraceKind::Ready => {
                // INV-2: readiness is a return of this attempt's body.
                if n.kind != Kind::Join {
                    let started = before
                        .iter()
                        .any(|x| attempt(x) == k && matches!(x.kind, TraceKind::Start(_)));
                    let ended_already = before.iter().any(|x| {
                        attempt(x) == k
                            && is_body_end(&x.kind)
                            && !matches!(x.kind, TraceKind::Ready)
                    });
                    if !started || ended_already {
                        out.push(violation(
                            "INV-2",
                            format!("{me} is Ready at #{i} without a live attempt {k}"),
                        ));
                    }
                }
            }
            TraceKind::ReleaseStart | TraceKind::CompensateStart => {
                // INV-4 and INV-18: a release body runs only for a held value.
                if Ctx::persistent(n) {
                    out.push(violation(
                        "INV-18",
                        format!("persistent effect {me} ran a compensation"),
                    ));
                }
                if matches!(n.kind, Kind::Resource | Kind::Effect) {
                    let held = before
                        .iter()
                        .any(|x| attempt(x) == k && matches!(x.kind, TraceKind::Held));
                    let ambiguous_ok = n.attr("ambiguous") == Some("compensate")
                        && before
                            .iter()
                            .any(|x| attempt(x) == k && matches!(x.kind, TraceKind::Ambiguous));
                    if !held && !ambiguous_ok {
                        out.push(violation(
                            "INV-4",
                            format!("{me} attempt {k} ran a release body without a Held"),
                        ));
                    }
                }
            }
            TraceKind::Interrupted { .. } => {
                // INV-7: a cleanup body is never interrupted.
                let open = before
                    .iter()
                    .rev()
                    .find(|x| is_cleanup_start(&x.kind) || is_cleanup_end(&x.kind));
                if let Some(x) = open {
                    if is_cleanup_start(&x.kind) {
                        out.push(violation(
                            "INV-7",
                            format!("{me}'s cleanup was interrupted"),
                        ));
                    }
                }
            }
            TraceKind::Ambiguous => {
                // INV-11.
                if n.kind != Kind::Effect {
                    out.push(violation(
                        "INV-11",
                        format!("{me} is not an effect but is Ambiguous"),
                    ));
                }
                if before
                    .iter()
                    .any(|x| attempt(x) == k && matches!(x.kind, TraceKind::Held))
                {
                    out.push(violation(
                        "INV-11",
                        format!("{me} held and is still Ambiguous"),
                    ));
                }
            }
            _ => {}
        }
        // INV-11, the other direction: an ambiguous effect is neither retried
        // nor compensated unless declared so.
        if matches!(e.kind, TraceKind::Start(_))
            && n.attr("ambiguous") != Some("retry")
            && before
                .iter()
                .any(|x| matches!(x.kind, TraceKind::Ambiguous))
        {
            out.push(violation(
                "INV-11",
                format!("{me} was retried after an ambiguous attempt"),
            ));
        }
        if matches!(e.kind, TraceKind::CompensateStart)
            && n.attr("ambiguous") != Some("compensate")
            && before
                .iter()
                .any(|x| attempt(x) == k && matches!(x.kind, TraceKind::Ambiguous))
        {
            out.push(violation(
                "INV-11",
                format!("{me} was compensated after an ambiguous attempt"),
            ));
        }
        // INV-5 and INV-16: a release of N never before a dependent's cleanup
        // ended — and every *copy* of that dependent counts, so a live
        // instance holds the key it imports shut.
        if is_cleanup_start(&e.kind) {
            for m in view.nodes.iter().filter(|m| m.needs.contains(path)) {
                // A **template** node has no lifetime of its own: what has to
                // outlive `path` is its live *instances*, and each of their
                // nodes is a dependent here in its own right. Counting the
                // template node too made a retried effect's between-attempts
                // release (INV-12, and no dependent can ever have started from
                // an attempt that failed) look like an INV-5 breach. The
                // instance-level clause is `INSTANCE-RELEASE` in
                // `containment.rs`, which asks the question of the release
                // that discharges the obligation rather than of every attempt.
                if m.kind == Kind::Template {
                    continue;
                }
                for mocc in c.copies(&m.path) {
                    if occ_of_target(&m.path, &mocc, path) != occ {
                        continue;
                    }
                    if !c.cleanup_ended_by(&m.path, &mocc, i) {
                        let rule = if mocc.iter().any(|x| x.is_some()) {
                            "INV-16"
                        } else {
                            "INV-5"
                        };
                        out.push(violation(
                            rule,
                            format!(
                                "{me}'s cleanup started at #{i} while dependent {} had not finished",
                                label(&m.path, &mocc)
                            ),
                        ));
                    }
                }
            }
        }
    }
    out.extend(super::check_containment(trace, view));
    out
}

/// The declaration path of a node as the report orders it: the position of
/// each segment among its siblings in the view, which is declaration order.
pub(super) fn expected_steps(view: &PlanView, path: &NodePath) -> Vec<u32> {
    let segs = path.segments();
    let mut out = Vec::new();
    for depth in 1..=segs.len() {
        let prefix = &segs[..depth - 1];
        let pos = view
            .nodes
            .iter()
            .filter(|n| n.path.segments().len() == depth && n.path.segments().starts_with(prefix))
            .position(|n| n.path.segments() == &segs[..depth]);
        out.push(pos.map(|p| p as u32).unwrap_or(u32::MAX));
    }
    out
}

/// Everything [`check_trace_prefix`] checks, plus what only holds at `End`,
/// plus the report against the trace (INV-3, INV-8, INV-9, INV-10, INV-15,
/// INV-20).
pub fn check_trace<Out>(
    trace: &Trace,
    view: &PlanView,
    report: &sdax::Report<Out>,
) -> Vec<Violation> {
    check_trace_with_slack(trace, view, report, std::time::Duration::ZERO)
}

/// [`check_trace`], forgiving `slack` of engine time on INV-8's bound alone.
///
/// Every other rule is about order and presence and is exact on any clock;
/// INV-8 is about a duration, so it is the only one an inexact clock can break
/// by itself. See [`check_report_with_slack`](super::check_report_with_slack).
pub fn check_trace_with_slack<Out>(
    trace: &Trace,
    view: &PlanView,
    report: &sdax::Report<Out>,
    slack: std::time::Duration,
) -> Vec<Violation> {
    let mut out = check_trace_prefix(trace, view);
    // Global over the whole trace, so once at the end rather than at every
    // prefix: the prefix pass is already quadratic.
    out.extend(super::check_arbitration(trace, view));
    out.extend(super::check_scopes(trace, view));
    out.extend(super::check_instance_releases(trace, view));
    out.extend(super::check_report_with_slack(trace, view, report, slack));
    let _ = occurrences(trace, view);
    out
}
