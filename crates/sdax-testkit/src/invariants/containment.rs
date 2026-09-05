//! **INV-16, INV-17 and the instance lifecycle** — the rules that exist only
//! because a template has instances.
//!
//! INV-16 has three clauses. The first, "an instance's nodes can need only
//! their own, their input and imported parent keys", and the second, "a parent
//! node can never name an instance node", are properties of the *declaration*:
//! [`check_containment`] recomputes both from the view, so a plan that could
//! express the violation is refused here whether or not a run reached it. The
//! third, "every live instance ends before any key it imports is released", is
//! a property of the run, and `trace.rs` enforces it as INV-5 over every copy.
//!
//! Then the lifecycle itself: an instance is spawned once, ends once, and
//! nothing of it is observed before the spawn or after the end (`T5-INSTANCE`,
//! the instance form of the rule `arbitration.rs` applies to components).

use super::occ::{inside_instance, label, occ_of, static_occ};
use super::{violation, Violation};
use sdax::host::InstanceId;
use sdax::{Kind, NodePath, PlanView, Trace, TraceKind};

/// Whether `inner` lies strictly under `outer`.
fn under(inner: &NodePath, outer: &NodePath) -> bool {
    inner.segments().len() > outer.segments().len()
        && inner.segments().starts_with(outer.segments())
}

/// `INV-16` (declaration) and `T5-INSTANCE`, `INSTANCE` (trace).
pub fn check_containment(trace: &Trace, view: &PlanView) -> Vec<Violation> {
    let mut out = Vec::new();
    let templates: Vec<&NodePath> = view
        .nodes
        .iter()
        .filter(|n| n.kind == Kind::Template)
        .map(|n| &n.path)
        .collect();

    // INV-16, clause 2: nothing outside a template's subtree may name a node
    // inside it. A parent that could depend on an instance node could not say
    // *which* instance, and its release would have no gate.
    //
    // Clause 1 — "an instance's node may name only its own, its input and
    // imported parent keys" — needs no separate rule here: a need of a node
    // inside `t` is under `t` (its own scope), is `t` itself (the input), or
    // is outside `t` (an ancestor key, which `V-IMPORT-SCOPE` already
    // restricts at `build`), and a need that reached into *another* template
    // is this same clause applied to that one.
    for t in &templates {
        for m in &view.nodes {
            if under(&m.path, t) {
                continue;
            }
            for need in &m.needs {
                if under(need, t) {
                    out.push(violation(
                        "INV-16",
                        format!("{} names {need}, which is inside the template {t}", m.path),
                    ));
                }
            }
        }
    }

    // The lifecycle: one spawn and at most one end per instance, and nothing
    // of an instance observed outside that window.
    let mut spawned: Vec<(NodePath, InstanceId, usize)> = Vec::new();
    let mut ended: Vec<(NodePath, InstanceId, usize)> = Vec::new();
    for (i, e) in trace.events.iter().enumerate() {
        let Some(path) = &e.node else { continue };
        match e.kind {
            TraceKind::InstanceSpawned(id) => {
                if view.node(&path.to_string()).map(|n| n.kind) != Some(Kind::Template) {
                    out.push(violation(
                        "INSTANCE",
                        format!("{path} spawned an instance but is not a template"),
                    ));
                }
                if spawned.iter().any(|(_, x, _)| *x == id) {
                    out.push(violation(
                        "INSTANCE",
                        format!("instance {id} was spawned twice"),
                    ));
                }
                spawned.push((path.clone(), id, i));
            }
            TraceKind::InstanceEnded(id, _) => {
                if !spawned.iter().any(|(p, x, _)| *x == id && p == path) {
                    out.push(violation(
                        "INSTANCE",
                        format!("instance {id} of {path} ended without ever being spawned"),
                    ));
                }
                if ended.iter().any(|(_, x, _)| *x == id) {
                    out.push(violation("INSTANCE", format!("instance {id} ended twice")));
                }
                ended.push((path.clone(), id, i));
            }
            _ => {}
        }
    }

    for (root, id, at) in &spawned {
        // T5-INSTANCE: nothing of an instance is observed before its spawn,
        // and nothing **starts** inside it after it has ended.
        let end = ended.iter().find(|(_, x, _)| x == id).map(|(_, _, j)| *j);
        for (i, e) in trace.events.iter().enumerate() {
            let Some(path) = &e.node else { continue };
            let occ = occ_of(e);
            if !inside_instance(path, &occ, root, *id) {
                continue;
            }
            if i < *at {
                out.push(violation(
                    "T5-INSTANCE",
                    format!(
                        "{} is observed at #{i}, before instance {id} was spawned at #{at}",
                        label(path, &occ)
                    ),
                ));
            }
            if let Some(j) = end {
                if i > j && matches!(e.kind, TraceKind::Start(_)) {
                    out.push(violation(
                        "T5-INSTANCE",
                        format!(
                            "{} started at #{i}, after instance {id} ended at #{j}",
                            label(path, &occ)
                        ),
                    ));
                }
            }
        }
        // A template's own obligation covers its instances: if the trace ends,
        // every instance it spawned must have ended too (INV-16's live-set
        // clause, and T8).
        let run_ended = trace
            .events
            .iter()
            .any(|e| matches!(e.kind, TraceKind::End(_)));
        if run_ended && end.is_none() {
            out.push(violation(
                "INV-16",
                format!("instance {id} of {root} was still live at End"),
            ));
        }
    }

    // A template that spawned an instance carries the obligation of stopping
    // it, so its own record must close: `Stopped`, or an abandonment.
    for t in &templates {
        let occ = static_occ(t);
        let evs: Vec<&TraceKind> = trace
            .events
            .iter()
            .filter(|e| e.node.as_ref() == Some(*t) && occ_of(e) == occ)
            .map(|e| &e.kind)
            .collect();
        let spawned_any = evs
            .iter()
            .any(|k| matches!(k, TraceKind::InstanceSpawned(_)));
        let closed = evs
            .iter()
            .any(|k| matches!(k, TraceKind::Stopped | TraceKind::Abandoned));
        let run_ended = trace
            .events
            .iter()
            .any(|e| matches!(e.kind, TraceKind::End(_)));
        if run_ended && spawned_any && !closed {
            out.push(violation(
                "INV-16",
                format!("template {t} spawned instances and its obligation never ended"),
            ));
        }
    }
    out
}

/// `INSTANCE-RELEASE` — INV-16's run-time clause: **every live instance ends
/// before any key it imports is released**.
///
/// Asked of the release that *discharges* a key's obligation: its last
/// cleanup start **after the key became `Ready`**. A retried node's
/// between-attempts release (INV-12) is not that: the attempt it undoes never
/// reached `Ready`, so no instance could have consumed its value and there is
/// nothing for it to be ordered against. Whole-trace only, so "last" is last.
///
/// The complement — an instance's node that really *started* on an imported
/// key — is INV-5 in `trace.rs`, per copy. This clause is what catches the
/// instance whose nodes were all skipped and which is nonetheless still live.
pub fn check_instance_releases(trace: &Trace, view: &PlanView) -> Vec<Violation> {
    let mut out = Vec::new();
    let last_cleanup = |k: &NodePath| -> Option<usize> {
        let mine = |e: &sdax::TraceEvent| {
            e.node.as_ref() == Some(k) && occ_of(e).iter().all(|i| i.is_none())
        };
        let ready = trace
            .events
            .iter()
            .position(|e| mine(e) && matches!(e.kind, TraceKind::Ready))?;
        trace
            .events
            .iter()
            .enumerate()
            .filter(|(i, e)| {
                *i > ready
                    && mine(e)
                    && matches!(
                        e.kind,
                        TraceKind::ReleaseStart
                            | TraceKind::CompensateStart
                            | TraceKind::StopRequested
                    )
            })
            .map(|(i, _)| i)
            .next_back()
    };
    for t in view.nodes.iter().filter(|n| n.kind == Kind::Template) {
        let mut live: Vec<(InstanceId, usize)> = Vec::new();
        for (i, e) in trace.events.iter().enumerate() {
            if e.node.as_ref() != Some(&t.path) {
                continue;
            }
            match e.kind {
                TraceKind::InstanceSpawned(id) => live.push((id, i)),
                TraceKind::InstanceEnded(id, _) => live.retain(|(x, _)| *x != id),
                _ => {}
            }
        }
        // Whatever is still in `live` never ended at all, which the
        // "still live at End" clause above already reports.
        let ends: Vec<(InstanceId, usize)> = trace
            .events
            .iter()
            .enumerate()
            .filter_map(|(i, e)| match e.kind {
                TraceKind::InstanceEnded(id, _) if e.node.as_ref() == Some(&t.path) => {
                    Some((id, i))
                }
                _ => None,
            })
            .collect();
        // T7b: once the shutdown budget is spent the remaining gated releases
        // are "started in order to be abandoned", so the ordering INV-16 asks
        // for is what INV-8 overrode. `cleanup_ended_by` waives INV-5 on an
        // abandonment for the same reason.
        let abandoned = trace.events.iter().position(|e| {
            e.node.as_ref() == Some(&t.path) && matches!(e.kind, TraceKind::Abandoned)
        });
        for key in &t.needs {
            let Some(r) = last_cleanup(key) else { continue };
            if abandoned.is_some_and(|a| a < r) {
                continue;
            }
            for (id, at) in &ends {
                if *at > r {
                    out.push(violation(
                        "INSTANCE-RELEASE",
                        format!(
                            "{key}'s release started at #{r}, before instance {id} of {} ended at #{at}",
                            t.path
                        ),
                    ));
                }
            }
        }
    }
    out
}
