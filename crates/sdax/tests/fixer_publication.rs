use sdax::host::engine::{Effect, Event, Machine, NodeState};
use sdax::prelude::*;

fn plan() -> Plan<()> {
    let mut b = Plan::builder("publication");
    let source = b.step("Source").run(|_, ()| async { Ok(()) });
    let first = b.join("First", source);
    b.join("Second", source);
    b.step("Consumer").needs(first).run(|_, _| async { Ok(()) });
    b.build(Policy::FailFast, Shutdown::unbounded(), Mode::Resident)
        .unwrap()
}

#[test]
fn structural_ready_waits_for_publication_and_rejects_bad_acknowledgements() {
    let p = plan();
    let mut m = Machine::new(&p).unwrap();
    m.begin();
    let source = m.key_of("Source").unwrap();
    let first = m.key_of("First").unwrap();
    let consumer = m.key_of("Consumer").unwrap();
    let unsolicited = m.step(Event::ReadyPublished {
        node: first,
        result: Ok(()),
    });
    assert!(matches!(unsolicited.as_slice(), [Effect::Reject(_)]));
    let unknown = m.step(Event::ReadyPublished {
        node: sdax::host::RawKey {
            plan: u64::MAX,
            idx: 0,
        },
        result: Ok(()),
    });
    assert!(matches!(unknown.as_slice(), [Effect::Reject(_)]));
    let fx = m.step(Event::NodeOk(source));
    assert!(fx
        .iter()
        .any(|f| matches!(f, Effect::PublishReady { node } if *node == first)));
    assert!(!fx
        .iter()
        .any(|f| matches!(f, Effect::Spawn { node, .. } if *node == consumer)));
    assert_eq!(m.state(first), Some(NodeState::Publishing));
    let rejected = m.step(Event::ReadyPublished {
        node: source,
        result: Ok(()),
    });
    assert!(matches!(rejected.as_slice(), [Effect::Reject(_)]));
    assert_eq!(m.state(first), Some(NodeState::Publishing));
    let fx = m.step(Event::ReadyPublished {
        node: first,
        result: Ok(()),
    });
    assert!(fx
        .iter()
        .any(|f| matches!(f, Effect::Spawn { node, .. } if *node == consumer)));
    assert_eq!(m.state(first), Some(NodeState::Ready));
    let rejected = m.step(Event::ReadyPublished {
        node: first,
        result: Ok(()),
    });
    assert!(matches!(rejected.as_slice(), [Effect::Reject(_)]));
}

#[test]
fn cancellation_waits_for_every_publication_without_resurrection_or_abort() {
    let p = plan();
    let mut m = Machine::new(&p).unwrap();
    m.begin();
    let source = m.key_of("Source").unwrap();
    let first = m.key_of("First").unwrap();
    let second = m.key_of("Second").unwrap();
    m.step(Event::NodeOk(source));
    let fx = m.step(Event::CancelRequested);
    assert!(!fx
        .iter()
        .any(|f| matches!(f, Effect::Abort(_) | Effect::Signal(_) | Effect::End(_))));
    assert!(matches!(m.state(first), Some(NodeState::Skipped { .. })));
    let fx = m.step(Event::ReadyPublished {
        node: first,
        result: Ok(()),
    });
    assert!(!fx.iter().any(|f| matches!(f, Effect::End(_))));
    assert!(!m.ended());
    let fx = m.step(Event::ReadyPublished {
        node: second,
        result: Ok(()),
    });
    assert!(fx
        .iter()
        .any(|f| matches!(f, Effect::End(Outcome::Cancelled))));
    assert!(matches!(m.state(first), Some(NodeState::Skipped { .. })));
}

#[test]
fn publication_failure_does_not_end_before_another_owed_acknowledgement() {
    let p = plan();
    let mut m = Machine::new(&p).unwrap();
    m.begin();
    let source = m.key_of("Source").unwrap();
    let first = m.key_of("First").unwrap();
    let second = m.key_of("Second").unwrap();
    m.step(Event::NodeOk(source));
    let fx = m.step(Event::ReadyPublished {
        node: first,
        result: Err("missing structural value".into()),
    });
    assert!(!fx
        .iter()
        .any(|f| matches!(f, Effect::End(_) | Effect::Spawn { .. } | Effect::Abort(_))));
    assert_eq!(m.state(first), Some(NodeState::Failed { held: false }));
    assert!(matches!(m.state(second), Some(NodeState::Skipped { .. })));
    m.step(Event::ReadyPublished {
        node: second,
        result: Ok(()),
    });
    assert!(m.ended());
    let report = m.take_report().unwrap();
    assert_eq!(report.outcome, Outcome::Failed);
    assert_eq!(report.faults.len(), 1);
}

fn component_plan(nested_join: bool) -> Plan<()> {
    let budget = Shutdown::within(std::time::Duration::ZERO);
    let mut child = Plan::builder("inner");
    let leaf = child.step("Leaf").run(|_, ()| async { Ok(()) });
    if nested_join {
        child.join("Barrier", leaf);
    }
    let child = child.build(Policy::FailFast, budget, Mode::Finite).unwrap();
    let mut root = Plan::builder("root");
    let child = root.component("Child", &child, ());
    root.step("Consumer")
        .needs(child)
        .run(|_, _| async { Ok(()) });
    root.build(Policy::FailFast, budget, Mode::Resident)
        .unwrap()
}

#[test]
fn component_or_descendant_publication_survives_cancel_and_budget_until_ack() {
    for nested_join in [false, true] {
        let p = component_plan(nested_join);
        let mut m = Machine::new(&p).unwrap();
        m.begin();
        let leaf = m.key_of("Child/Leaf").unwrap();
        let child = m.key_of("Child").unwrap();
        let pending = if nested_join {
            m.key_of("Child/Barrier").unwrap()
        } else {
            child
        };
        let fx = m.step(Event::NodeOk(leaf));
        assert!(fx
            .iter()
            .any(|f| matches!(f, Effect::PublishReady { node } if *node == pending)));
        let fx = m.step(Event::CancelRequested);
        assert!(!fx
            .iter()
            .any(|f| matches!(f, Effect::End(_) | Effect::Abort(_) | Effect::Release(_))));
        assert_eq!(m.state(child), Some(NodeState::Interrupted { held: true }));
        let timers: Vec<_> = fx
            .iter()
            .filter_map(|f| match f {
                Effect::Timer { id, .. } => Some(*id),
                _ => None,
            })
            .collect();
        assert!(!timers.is_empty());
        for timer in timers {
            let fx = m.step(Event::Timer(timer));
            assert!(!fx
                .iter()
                .any(|f| matches!(f, Effect::End(_) | Effect::Abort(_))));
            assert_eq!(m.state(child), Some(NodeState::Interrupted { held: true }));
        }
        assert!(!m.ended());
        let fx = m.step(Event::ReadyPublished {
            node: pending,
            result: Ok(()),
        });
        assert!(m.ended());
        assert!(fx
            .iter()
            .any(|f| matches!(f, Effect::End(Outcome::Cancelled))));
        assert!(!fx
            .iter()
            .any(|f| matches!(f, Effect::Emit(e) if matches!(e.kind, sdax::TraceKind::Ready))));
    }
}

#[test]
fn component_publication_error_is_a_prepare_fault_and_cleans_up_inner_scope() {
    let p = component_plan(false);
    let mut m = Machine::new(&p).unwrap();
    m.begin();
    let leaf = m.key_of("Child/Leaf").unwrap();
    let child = m.key_of("Child").unwrap();
    m.step(Event::NodeOk(leaf));
    let fx = m.step(Event::ReadyPublished {
        node: child,
        result: Err("export slot missing".into()),
    });
    assert!(!fx
        .iter()
        .any(|f| matches!(f, Effect::Spawn { .. } | Effect::Abort(_))));
    assert!(m.ended());
    let report = m.take_report().unwrap();
    assert_eq!(report.outcome, Outcome::Failed);
    assert_eq!(report.faults.len(), 1);
    assert_eq!(report.faults[0].phase, sdax::Phase::Prepare);
    assert_eq!(report.faults[0].node.to_string(), "Child");
}

#[test]
fn stopped_instance_waits_for_component_descendant_publication_only_in_that_instance() {
    use sdax::host::engine::RunState;
    use sdax::host::InstanceId;
    let mut child = Plan::builder("child");
    let leaf = child.step("Leaf").run(|_, ()| async { Ok(()) });
    child.join("Barrier", leaf);
    let child = child
        .build(Policy::FailFast, Shutdown::unbounded(), Mode::Finite)
        .unwrap();
    let mut template = Plan::with_input::<u8>("worker");
    template.component("Child", &child, ());
    let template = template
        .build(Policy::FailFast, Shutdown::unbounded(), Mode::Finite)
        .unwrap();
    let mut root = Plan::builder("root");
    let template = root.template("Workers", &template);
    root.service("Owner")
        .stop_within(std::time::Duration::from_secs(1))
        .spawns(&template)
        .initialize(|_, ()| async { Ok(()) })
        .serve(|_, _handle| async { Ok(()) });
    let p = root
        .build(Policy::FailFast, Shutdown::unbounded(), Mode::Resident)
        .unwrap();
    let mut m = Machine::new(&p).unwrap();
    m.begin();
    let owner = m.key_of("Owner").unwrap();
    let template = m.key_of("Workers").unwrap();
    for id in [InstanceId(11), InstanceId(22)] {
        let fx = m.step(Event::InstanceSpawned {
            spawner: owner,
            template,
            id,
        });
        assert!(!fx.iter().any(|f| matches!(f, Effect::Reject(_))));
        let nodes = m.instance_nodes(id);
        assert_eq!(nodes.len(), 3, "must include component descendants");
        for (run, decl, _, _) in nodes {
            assert_eq!(m.origin(run), Some((decl, Some(id))));
        }
    }
    let nodes = m.instance_nodes(InstanceId(11));
    let leaf = nodes
        .iter()
        .find(|(_, _, path, _)| path.leaf() == "Leaf")
        .unwrap()
        .0;
    let barrier = nodes
        .iter()
        .find(|(_, _, path, _)| path.leaf() == "Barrier")
        .unwrap()
        .0;
    m.step(Event::NodeOk(leaf));
    assert_eq!(m.state(barrier), Some(NodeState::Publishing));
    let fx = m.step(Event::StopInstance(InstanceId(11)));
    assert!(!fx.iter().any(
        |f| matches!(f, Effect::Emit(e) if matches!(e.kind, sdax::TraceKind::InstanceEnded(..)))
    ));
    assert!(m
        .instances()
        .iter()
        .all(|(_, state)| *state != RunState::Ended));
    m.step(Event::ReadyPublished {
        node: barrier,
        result: Ok(()),
    });
    assert!(m.instances().contains(&(InstanceId(11), RunState::Ended)));
    assert!(m
        .instances()
        .contains(&(InstanceId(22), RunState::Admitting)));
}
