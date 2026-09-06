//! What only holds at `End`, and the report against the trace: INV-3, INV-8,
//! INV-9, INV-10, INV-15, INV-18 and INV-20.
//!
//! Every per-node clause runs **per copy** ([`Occ`]), so an instance's
//! orphan, unreleased hold or unreported fault is caught exactly as the static
//! graph's is. The report's own lists are matched by path *and* by the
//! instance the record order names, which is what makes "a fault the trace
//! never observed" a real check when two instances share a path.

use super::occ::{instance, label, occ_of, occurrences, Occ};
use super::trace::{attempt, expected_steps, is_body_end, is_cleanup_end, is_cleanup_start, Ctx};
use super::{violation, Violation};
use sdax::host::InstanceId;
use sdax::{Kind, NodePath, Phase, PlanView, RecordOrder, Report, Trace, TraceEvent, TraceKind};
use std::time::Duration;

/// The instance a record belongs to, from its order.
fn record_instance(o: &RecordOrder) -> Option<InstanceId> {
    o.steps.iter().rev().find_map(|(_, i)| *i)
}

/// Whether a report record is about this copy of a node.
fn about(node: &NodePath, occ: &Occ, rec_node: &NodePath, rec: &RecordOrder) -> bool {
    rec_node == node && record_instance(rec) == instance(occ)
}

/// The report against the trace, per copy: INV-3, INV-8, INV-9, INV-10,
/// INV-15, INV-18 and INV-20, with no allowance for an inexact clock.
pub fn check_report<Out>(trace: &Trace, view: &PlanView, report: &Report<Out>) -> Vec<Violation> {
    check_report_with_slack(trace, view, report, Duration::ZERO)
}

/// [`check_report`], forgiving `slack` of engine time on INV-8's bound.
///
/// INV-8 is the one rule whose subject is a *duration*, so it is the one rule
/// a clock that is not exact can break on its own: a real timer fires at or
/// after its deadline, never before, and a compressed clock multiplies that
/// overshoot by the compression factor. `slack` is the substrate's measured
/// slop expressed in engine time, so the assertion still says "the engine did
/// not wait longer than the budget" and no longer says "the scheduler is
/// punctual". Pass [`Duration::ZERO`] on an exact clock — that is what
/// [`check_report`] does, and it is what `start_paused` deserves.
pub fn check_report_with_slack<Out>(
    trace: &Trace,
    view: &PlanView,
    report: &Report<Out>,
    slack: Duration,
) -> Vec<Violation> {
    let mut out = Vec::new();
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
            if elapsed > budget + slack {
                let over = elapsed - budget;
                out.push(violation(
                    "INV-8",
                    format!(
                        "{elapsed:?} elapsed from Settling to End, {over:?} over the budget \
                         {budget:?} and over the clock slack {slack:?}"
                    ),
                ));
            }
        }
    }
    for (path, occ) in occurrences(trace, view) {
        let Some(n) = c.node(&path) else { continue };
        let me = label(&path, &occ);
        let incomplete = |p: &NodePath| {
            report
                .incomplete
                .iter()
                .any(|r| about(p, &occ, &r.node, &r.order))
        };
        let ambiguous = |p: &NodePath| {
            report
                .ambiguous
                .iter()
                .any(|r| about(p, &occ, &r.node, &r.order))
        };
        let mine = c.of(&path, &occ);
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
                    format!("{me} attempt {k} started and never ended (orphan)"),
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
                                format!("{me} attempt {k}: its release never ended"),
                            ));
                        }
                    }
                    0 => {
                        if !incomplete(&path) {
                            out.push(violation(
                                "INV-3",
                                format!(
                                    "{me} attempt {k} held and was never released nor abandoned"
                                ),
                            ));
                        }
                    }
                    _ => out.push(violation(
                        "INV-3",
                        format!("{me} attempt {k} was released {starts} times"),
                    )),
                }
            }
            if starts > 0 && !cleanup_ended {
                out.push(violation(
                    "INV-15",
                    format!("{me} attempt {k}: a cleanup started and never ended"),
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
                    Kind::Component => evk[p..]
                        .iter()
                        .any(|e| matches!(e.kind, TraceKind::ReleaseOk | TraceKind::Abandoned)),
                    _ => true,
                };
                if !discharged {
                    out.push(violation(
                        "INV-3",
                        format!("{me} attempt {k} was ready and its obligation never ended"),
                    ));
                }
            }
            for e in &evk {
                match &e.kind {
                    TraceKind::Fail(phase @ (Phase::Prepare | Phase::Run | Phase::Serve), l) => {
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
                            about(&path, &occ, &f.node, &f.order)
                                && f.order.attempt == k
                                && f.phase == *phase
                                && f.kind.label() == *l
                        });
                        if !absorbed && !interrupted && !recorded {
                            out.push(violation("INV-9", format!("{me} attempt {k} failed ({phase}, {l:?}) and the report does not say so")));
                        }
                    }
                    TraceKind::ReleaseFail(l)
                    | TraceKind::CompensateFail(l)
                    | TraceKind::Fail(Phase::Stop, l) => {
                        let recorded = report
                            .cleanup_failures
                            .iter()
                            .any(|f| about(&path, &occ, &f.node, &f.order) && f.kind.label() == *l);
                        if !recorded {
                            out.push(violation(
                                "INV-9",
                                format!("{me}'s cleanup failed and the report does not say so"),
                            ));
                        }
                    }
                    TraceKind::Abandoned => {
                        if !incomplete(&path) {
                            out.push(violation(
                                "INV-9",
                                format!("{me} was abandoned and is not in incomplete"),
                            ));
                        }
                    }
                    TraceKind::Ambiguous => {
                        // Absorbed by a later attempt that reached `Ready`,
                        // exactly as a fault is: `Ambiguity::Retry` says the
                        // effect may simply be done again, and doing it again
                        // successfully is the answer to the ambiguity. The
                        // trace still carries it.
                        let absorbed = mine
                            .iter()
                            .any(|(_, x)| attempt(x) > k && matches!(x.kind, TraceKind::Ready));
                        if !absorbed && !ambiguous(&path) {
                            out.push(violation(
                                "INV-9",
                                format!("{me} is ambiguous and is not in the report"),
                            ));
                        }
                    }
                    TraceKind::Interrupted { .. }
                        if report.faults.iter().any(|f| {
                            about(&path, &occ, &f.node, &f.order) && f.order.attempt == k
                        }) =>
                    {
                        // INV-10.
                        out.push(violation(
                            "INV-10",
                            format!("{me} attempt {k} was interrupted and is reported as a fault"),
                        ));
                    }
                    _ => {}
                }
            }
        }
        if Ctx::persistent(n)
            && report
                .cleanup_failures
                .iter()
                .any(|f| about(&path, &occ, &f.node, &f.order))
        {
            out.push(violation(
                "INV-18",
                format!("persistent effect {me} has a cleanup failure"),
            ));
        }
    }
    out.extend(check_lists(trace, view, report));
    out
}

/// The report names nothing the trace did not observe, in F4 order.
fn check_lists<Out>(trace: &Trace, view: &PlanView, report: &Report<Out>) -> Vec<Violation> {
    let mut out = Vec::new();
    let c = Ctx { trace, view };
    let orders: Vec<(&str, Vec<(&NodePath, &RecordOrder)>)> = vec![
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
    let seen_in = |node: &NodePath, order: &RecordOrder, want: fn(&TraceEvent) -> bool| {
        trace.events.iter().any(|e| {
            e.node.as_ref() == Some(node)
                && record_instance(order) == instance(&occ_of(e))
                && want(e)
        });
    };
    let _ = seen_in;
    for f in &report.faults {
        let seen = trace.events.iter().any(|e| {
            e.node.as_ref() == Some(&f.node)
                && record_instance(&f.order) == instance(&occ_of(e))
                && attempt(e) == f.order.attempt
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
        let seen = trace.events.iter().any(|e| {
            e.node.as_ref() == Some(&r.node)
                && record_instance(&r.order) == instance(&occ_of(e))
                && matches!(e.kind, TraceKind::Abandoned)
        });
        if !seen {
            out.push(violation(
                "INV-9",
                format!("incomplete lists {} but nothing was abandoned", r.node),
            ));
        }
    }
    for r in &report.ambiguous {
        let seen = trace.events.iter().any(|e| {
            e.node.as_ref() == Some(&r.node)
                && record_instance(&r.order) == instance(&occ_of(e))
                && matches!(e.kind, TraceKind::Ambiguous)
        });
        if !seen {
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
