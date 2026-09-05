//! The trace-level invariant checker: the second opinion.
//!
//! Recomputes every checkable clause of INV-1…20 and `orphans: none` from
//! the plan view and the trace alone. It never reads the machine: a checker
//! that trusted the thing it checks would prove nothing.
//!
//! [`check_trace_prefix`] holds at every prefix of a run and is what the
//! scripted driver runs after every step; [`check_trace`] adds what only
//! holds at `End` and cross-checks the report.

use super::{violation, Violation};
use sdax::{Kind, NodePath, NodeView, Phase, PlanView, Report, Trace, TraceEvent, TraceKind};

pub(super) struct Ctx<'a> {
    pub trace: &'a Trace,
    pub view: &'a PlanView,
}

pub(super) fn attempt(e: &TraceEvent) -> u32 {
    e.order.as_ref().map(|o| o.attempt).unwrap_or(0)
}

fn is_body_end(k: &TraceKind) -> bool {
    matches!(
        k,
        TraceKind::Ready
            | TraceKind::Fail(Phase::Prepare | Phase::Run, _)
            | TraceKind::Interrupted { .. }
            | TraceKind::Ambiguous
            | TraceKind::Abandoned
    )
}

fn is_cleanup_start(k: &TraceKind) -> bool {
    matches!(
        k,
        TraceKind::ReleaseStart | TraceKind::CompensateStart | TraceKind::StopRequested
    )
}

fn is_cleanup_end(k: &TraceKind) -> bool {
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

    /// `(index, event)` for one node, in trace order.
    pub fn of(&self, path: &NodePath) -> Vec<(usize, &'a TraceEvent)> {
        self.trace
            .events
            .iter()
            .enumerate()
            .filter(|(_, e)| e.node.as_ref() == Some(path))
            .collect()
    }

    pub fn persistent(n: &NodeView) -> bool {
        n.attr("release") == Some("persistent")
    }

    pub fn owes_when_held(n: &NodeView) -> bool {
        matches!(n.kind, Kind::Resource | Kind::Effect) && !Self::persistent(n)
    }

    /// Whether `path` is finished with everything before index `r`: its last
    /// attempt ended and, if it held or served or ran an inner scope, that
    /// obligation ended too (the INV-5 clause "M's cleanup has ended").
    pub fn cleanup_ended_by(&self, path: &NodePath, r: usize) -> bool {
        let Some(n) = self.node(path) else {
            return true;
        };
        let evs: Vec<&TraceEvent> = self
            .of(path)
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
            Kind::Component => after.iter().any(|e| {
                matches!(
                    e.kind,
                    TraceKind::ReleaseOk | TraceKind::Stopped | TraceKind::Abandoned
                )
            }),
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
        let k = attempt(e);
        let mine = c.of(path);
        let before: Vec<&TraceEvent> = mine
            .iter()
            .filter(|(j, _)| *j < i)
            .map(|(_, e)| *e)
            .collect();
        match &e.kind {
            TraceKind::Start(_) => {
                // INV-1: every need is Ready now, and nothing else ordered it.
                for need in &n.needs {
                    let last = c.of(need).into_iter().rfind(|(j, _)| *j < i);
                    let ready_now = match last {
                        Some((_, le)) => match le.kind {
                            TraceKind::Ready => true,
                            TraceKind::Stopped => !c
                                .of(need)
                                .iter()
                                .any(|(j, x)| *j < i && matches!(x.kind, TraceKind::StopRequested)),
                            _ => false,
                        },
                        None => false,
                    };
                    if !ready_now {
                        out.push(violation(
                            "INV-1",
                            format!("{path} started at #{i} while its need {need} is not Ready"),
                        ));
                    }
                }
                if let Some(s) = settling {
                    if i > s {
                        out.push(violation(
                            "T5",
                            format!("{path} started at #{i} after the run settled"),
                        ));
                    }
                }
                if before
                    .iter()
                    .any(|x| matches!(x.kind, TraceKind::Skipped { .. }))
                {
                    out.push(violation(
                        "T4",
                        format!("{path} started after being skipped"),
                    ));
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
                        format!("{path} attempt {k} started where attempt {expected} was expected"),
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
                            format!("{path} attempt {k} started while attempt {pk} was in flight"),
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
                            format!(
                                "{path} attempt {k} started before attempt {pk}'s release ended"
                            ),
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
                            format!("{path} is Ready at #{i} without a live attempt {k}"),
                        ));
                    }
                }
            }
            TraceKind::ReleaseStart | TraceKind::CompensateStart => {
                // INV-4 and INV-18: a release body runs only for a held value.
                if Ctx::persistent(n) {
                    out.push(violation(
                        "INV-18",
                        format!("persistent effect {path} ran a compensation"),
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
                            format!("{path} attempt {k} ran a release body without a Held"),
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
                            format!("{path}'s cleanup was interrupted"),
                        ));
                    }
                }
            }
            TraceKind::Ambiguous => {
                // INV-11.
                if n.kind != Kind::Effect {
                    out.push(violation(
                        "INV-11",
                        format!("{path} is not an effect but is Ambiguous"),
                    ));
                }
                if before
                    .iter()
                    .any(|x| attempt(x) == k && matches!(x.kind, TraceKind::Held))
                {
                    out.push(violation(
                        "INV-11",
                        format!("{path} held and is still Ambiguous"),
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
                format!("{path} was retried after an ambiguous attempt"),
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
                format!("{path} was compensated after an ambiguous attempt"),
            ));
        }
        // INV-5: a release of N never before a dependent's cleanup ended.
        if is_cleanup_start(&e.kind) {
            for m in view.nodes.iter().filter(|m| m.needs.contains(path)) {
                if !c.cleanup_ended_by(&m.path, i) {
                    out.push(violation(
                        "INV-5",
                        format!(
                            "{path}'s cleanup started at #{i} while dependent {} had not finished",
                            m.path
                        ),
                    ));
                }
            }
        }
    }
    out
}

/// The declaration path of a node as the report orders it: the position of
/// each segment among its siblings in the view, which is declaration order.
fn expected_steps(view: &PlanView, path: &NodePath) -> Vec<u32> {
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
pub fn check_trace<Out>(trace: &Trace, view: &PlanView, report: &Report<Out>) -> Vec<Violation> {
    let mut out = check_trace_prefix(trace, view);
    let c = Ctx { trace, view };
    let evs = &trace.events;
    let ends: Vec<usize> = evs
        .iter()
        .enumerate()
        .filter(|(_, e)| matches!(e.kind, TraceKind::End(_)))
        .map(|(i, _)| i)
        .collect();
    match ends.as_slice() {
        [last] if *last + 1 == evs.len() => {}
        [] => out.push(violation("T8", "the trace has no End".into())),
        _ => out.push(violation("T8", "End is not the single last event".into())),
    }
    if let Some(TraceKind::End(o)) = evs.last().map(|e| &e.kind) {
        if *o != report.outcome {
            out.push(violation(
                "INV-9",
                format!("the trace ends {o} but the report says {}", report.outcome),
            ));
        }
    }
    // INV-8: bounded shutdown.
    if let (Some(budget), Some(end)) = (view.shutdown.budget(), evs.last()) {
        if let Some(s) = evs.iter().find(|e| matches!(e.kind, TraceKind::Settling)) {
            let elapsed = end.at.checked_duration_since(s.at).unwrap_or_default();
            if elapsed > budget {
                out.push(violation(
                    "INV-8",
                    format!("{elapsed:?} elapsed from Settling to End, over the budget {budget:?}"),
                ));
            }
        }
    }
    let incomplete = |p: &NodePath| report.incomplete.iter().any(|r| r.node == *p);
    let ambiguous = |p: &NodePath| report.ambiguous.iter().any(|r| r.node == *p);
    for n in &view.nodes {
        let mine = c.of(&n.path);
        let attempts: Vec<u32> = {
            let mut v: Vec<u32> = mine.iter().map(|(_, e)| attempt(e)).collect();
            v.sort();
            v.dedup();
            v
        };
        for k in attempts {
            let evk: Vec<&TraceEvent> = mine
                .iter()
                .filter(|(_, e)| attempt(e) == k)
                .map(|(_, e)| *e)
                .collect();
            let started = evk.iter().any(|e| matches!(e.kind, TraceKind::Start(_)));
            let ended = evk.iter().any(|e| is_body_end(&e.kind));
            // INV-15: every spawned body was joined or recorded abandoned.
            if started && !ended {
                out.push(violation(
                    "INV-15",
                    format!("{} attempt {k} started and never ended (orphan)", n.path),
                ));
            }
            let held = evk.iter().any(|e| matches!(e.kind, TraceKind::Held));
            let starts = evk.iter().filter(|e| is_cleanup_start(&e.kind)).count();
            let cleanup_ended = evk.iter().any(|e| is_cleanup_end(&e.kind));
            // INV-3: held ⇒ exactly one release attempt, unless abandoned first.
            if held && Ctx::owes_when_held(n) {
                match starts {
                    1 => {
                        if !cleanup_ended {
                            out.push(violation(
                                "INV-15",
                                format!("{} attempt {k}: its release never ended", n.path),
                            ));
                        }
                    }
                    0 => {
                        if !incomplete(&n.path) {
                            out.push(violation(
                                "INV-3",
                                format!(
                                    "{} attempt {k} held and was never released nor abandoned",
                                    n.path
                                ),
                            ));
                        }
                    }
                    _ => out.push(violation(
                        "INV-3",
                        format!("{} attempt {k} was released {starts} times", n.path),
                    )),
                }
            }
            if starts > 0 && !cleanup_ended {
                out.push(violation(
                    "INV-15",
                    format!("{} attempt {k}: a cleanup started and never ended", n.path),
                ));
            }
            // A ready service's or component's obligation is discharged.
            let ready_pos = evk.iter().position(|e| matches!(e.kind, TraceKind::Ready));
            if let Some(p) = ready_pos {
                let discharged = match n.kind {
                    Kind::Service => evk[p..].iter().any(|e| {
                        matches!(
                            e.kind,
                            TraceKind::Stopped
                                | TraceKind::Fail(Phase::Stop | Phase::Serve, _)
                                | TraceKind::Abandoned
                        )
                    }),
                    Kind::Component => evk[p..].iter().any(|e| {
                        matches!(
                            e.kind,
                            TraceKind::ReleaseOk | TraceKind::Stopped | TraceKind::Abandoned
                        )
                    }),
                    _ => true,
                };
                if !discharged {
                    out.push(violation(
                        "INV-3",
                        format!(
                            "{} attempt {k} was ready and its obligation never ended",
                            n.path
                        ),
                    ));
                }
            }
            for e in &evk {
                match &e.kind {
                    TraceKind::Fail(
                        phase @ (Phase::Prepare | Phase::Run | Phase::Serve),
                        label,
                    ) => {
                        let absorbed = mine
                            .iter()
                            .any(|(_, x)| attempt(x) > k && matches!(x.kind, TraceKind::Ready));
                        // The engine cancelled this attempt, so whatever the
                        // body returned is not a fault (INV-10) — a panic is
                        // observed in the trace and nowhere else. For an
                        // effect that had not held, `Ambiguous` *is* that
                        // terminal observation (INV-11), not `Interrupted`.
                        let interrupted = evk.iter().any(|x| {
                            matches!(x.kind, TraceKind::Interrupted { .. } | TraceKind::Ambiguous)
                        });
                        let recorded = report.faults.iter().any(|f| {
                            f.node == n.path
                                && f.order.attempt == k
                                && f.phase == *phase
                                && f.kind.label() == *label
                        });
                        if !absorbed && !interrupted && !recorded {
                            out.push(violation("INV-9", format!("{} attempt {k} failed ({phase}, {label:?}) and the report does not say so", n.path)));
                        }
                    }
                    TraceKind::ReleaseFail(label)
                    | TraceKind::CompensateFail(label)
                    | TraceKind::Fail(Phase::Stop, label) => {
                        let recorded = report
                            .cleanup_failures
                            .iter()
                            .any(|f| f.node == n.path && f.kind.label() == *label);
                        if !recorded {
                            out.push(violation(
                                "INV-9",
                                format!(
                                    "{}'s cleanup failed and the report does not say so",
                                    n.path
                                ),
                            ));
                        }
                    }
                    TraceKind::Abandoned => {
                        if !incomplete(&n.path) {
                            out.push(violation(
                                "INV-9",
                                format!("{} was abandoned and is not in incomplete", n.path),
                            ));
                        }
                    }
                    TraceKind::Ambiguous => {
                        if !ambiguous(&n.path) {
                            out.push(violation(
                                "INV-9",
                                format!("{} is ambiguous and is not in the report", n.path),
                            ));
                        }
                    }
                    TraceKind::Interrupted { .. }
                        if report
                            .faults
                            .iter()
                            .any(|f| f.node == n.path && f.order.attempt == k) =>
                    {
                        // INV-10.
                        out.push(violation(
                            "INV-10",
                            format!(
                                "{} attempt {k} was interrupted and is reported as a fault",
                                n.path
                            ),
                        ));
                    }
                    _ => {}
                }
            }
        }
        if Ctx::persistent(n) && report.cleanup_failures.iter().any(|f| f.node == n.path) {
            out.push(violation(
                "INV-18",
                format!("persistent effect {} has a cleanup failure", n.path),
            ));
        }
    }
    // The report names nothing the trace did not observe, in F4 order.
    let orders: Vec<(&str, Vec<(&NodePath, &sdax::RecordOrder)>)> = vec![
        (
            "faults",
            report.faults.iter().map(|f| (&f.node, &f.order)).collect(),
        ),
        (
            "cleanup_failures",
            report
                .cleanup_failures
                .iter()
                .map(|f| (&f.node, &f.order))
                .collect(),
        ),
        (
            "incomplete",
            report
                .incomplete
                .iter()
                .map(|r| (&r.node, &r.order))
                .collect(),
        ),
        (
            "ambiguous",
            report
                .ambiguous
                .iter()
                .map(|r| (&r.node, &r.order))
                .collect(),
        ),
    ];
    for (name, list) in orders {
        for w in list.windows(2) {
            if w[1].1 < w[0].1 {
                out.push(violation(
                    "INV-20",
                    format!("{name} is not in record order at {}", w[1].0),
                ));
            }
        }
        for (path, order) in list {
            if c.node(path).is_none() {
                out.push(violation(
                    "INV-9",
                    format!("{name} names {path}, which the plan does not declare"),
                ));
                continue;
            }
            let steps: Vec<u32> = order.steps.iter().map(|(i, _)| *i).collect();
            if steps != expected_steps(view, path) {
                out.push(violation(
                    "INV-20",
                    format!(
                        "{name}: {path} has record steps {steps:?}, declaration says {:?}",
                        expected_steps(view, path)
                    ),
                ));
            }
        }
    }
    for f in &report.faults {
        let seen = c.of(&f.node).iter().any(|(_, e)| {
            attempt(e) == f.order.attempt
                && matches!(&e.kind, TraceKind::Fail(p, l) if *p == f.phase && *l == f.kind.label())
        });
        if !seen {
            out.push(violation(
                "INV-9",
                format!(
                    "the report lists a fault the trace never observed: {} attempt {} ({})",
                    f.node, f.order.attempt, f.phase
                ),
            ));
        }
    }
    for r in &report.incomplete {
        if !c
            .of(&r.node)
            .iter()
            .any(|(_, e)| matches!(e.kind, TraceKind::Abandoned))
        {
            out.push(violation(
                "INV-9",
                format!("incomplete lists {} but nothing was abandoned", r.node),
            ));
        }
    }
    for r in &report.ambiguous {
        if !c
            .of(&r.node)
            .iter()
            .any(|(_, e)| matches!(e.kind, TraceKind::Ambiguous))
        {
            out.push(violation(
                "INV-9",
                format!("ambiguous lists {} but the trace never said so", r.node),
            ));
        }
    }
    let clean = report.faults.is_empty()
        && report.cleanup_failures.is_empty()
        && report.incomplete.is_empty()
        && report.ambiguous.is_empty()
        && report.outcome == sdax::Outcome::Ok;
    if clean != report.is_clean() {
        out.push(violation(
            "INV-9",
            "is_clean disagrees with the lists".into(),
        ));
    }
    out
}
