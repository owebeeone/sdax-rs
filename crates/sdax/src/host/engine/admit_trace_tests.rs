//! Compare the single-scope path with the existing global admission sweep.
use super::*;
use crate::host::engine::Event;
use crate::{Plan, Policy, Retry, Shutdown};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

fn fixture(broken: bool) -> Plan {
    let mut p = Plan::builder("Admission");
    let pool = p.pool("Workers", 1);
    let base = p
        .resource("Base")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(1u8)) })
        .release(|_, _| async { Ok(()) });
    for name in ["First", "Second"] {
        p.step(name)
            .needs(base)
            .exclusive(base)
            .limit(pool)
            .run(|_, _| async { Ok(()) });
    }
    p.step("PoolOnly").limit(pool).run(|_, ()| async { Ok(()) });
    p.resource("Retry")
        .needs(base)
        .idempotent()
        .retry(Retry::attempts(2))
        .acquire(|cx, _| async move { Ok(cx.hold_value(2u8)) })
        .release(|_, _| async { Ok(()) });
    if broken {
        let fail = p.step("Broken").needs(base).run(|_, _| async { Ok(()) });
        p.step("Skipped").needs(fail).run(|_, _| async { Ok(()) });
    }
    p.build(
        Policy::Isolate,
        Shutdown::within(Duration::from_secs(1)),
        crate::Mode::Finite,
    )
    .unwrap()
}

fn effects_trace(plan: &Plan, use_global_sweep: bool) -> Vec<String> {
    let mut topology = plan.layout.as_ref().unwrap().as_ref().clone();
    if use_global_sweep {
        // An inert Planned scope has no nodes, events, grants or parent. It
        // forces the multi-scope admission path while leaving the fixture's
        // graph and event order unchanged, without a production test switch.
        let mut inert = topology.scopes[0].clone();
        inert.name = "Inert reference scope".into();
        inert.nodes.clear();
        inert.pools.clear();
        inert.export = None;
        topology.scopes.push(inert);
    }
    let mut machine = Machine::from_table(Arc::new(topology));
    let mut pending = VecDeque::new();
    let mut timers = Vec::new();
    let mut effects = machine.begin();
    let mut trace = Vec::new();
    for _ in 0..1000 {
        for effect in effects {
            trace.push(format!("{effect:?}"));
            match effect {
                Effect::Spawn { node, attempt } => {
                    pending.push_back(Event::Started(node));
                    if machine.kind_of(node).unwrap().can_hold() {
                        pending.push_back(Event::Held(node));
                    }
                    let name = machine.path_of(node).unwrap();
                    let fails = (*name == *"Retry" && attempt == 1) || *name == *"Broken";
                    pending.push_back(if fails {
                        Event::NodeErr(node, crate::FaultKind::Error("fixture failure".into()))
                    } else {
                        Event::NodeOk(node)
                    });
                }
                Effect::Release(node) => pending.push_back(Event::NodeOk(node)),
                Effect::Timer { id, at } => timers.push((id, at)),
                Effect::CancelTimer(id) => timers.retain(|(timer, _)| *timer != id),
                Effect::Reject(rejected) => panic!("unexpected rejection: {rejected:?}"),
                Effect::End(_) => return trace,
                _ => {}
            }
        }
        let event = pending.pop_front().unwrap_or_else(|| {
            let earliest = timers
                .iter()
                .enumerate()
                .min_by_key(|(_, (_, at))| *at)
                .map(|(i, _)| i)
                .expect("fixture cannot stall");
            let (id, at) = timers.remove(earliest);
            machine.advance(at);
            Event::Timer(id)
        });
        effects = machine.step(event);
    }
    panic!("fixture exceeded bounded event count")
}

#[test]
fn single_scope_admission_preserves_global_effect_order_with_grants_and_retry() {
    for broken in [false, true] {
        let plan = fixture(broken);
        let reference = effects_trace(&plan, true);
        let actual = effects_trace(&plan, false);
        assert_eq!(actual, reference);
        assert!(actual.iter().any(|e| e.contains("attempt: 2")));
        assert!(actual.iter().any(|e| e.contains("ReleaseOk")));
        if broken {
            assert!(actual.iter().any(|e| e.contains("Skipped")));
        }
    }
}
