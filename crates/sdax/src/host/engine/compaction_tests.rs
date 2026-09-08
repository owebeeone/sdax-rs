use super::*;
use crate::{Mode, Plan, Policy, Shutdown};
use std::time::Duration;

fn machine() -> (Machine, RawKey, RawKey, RawKey) {
    let mut p = Plan::builder("Churn");
    let endpoint = p
        .resource("Endpoint")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(())) })
        .release(|_, _| async { Ok(()) });
    let mut child = Plan::builder("Child");
    let imported = child.import(endpoint);
    child
        .resource("Socket")
        .needs(imported)
        .acquire(|cx, _| async move { Ok(cx.hold_value(())) })
        .release(|_, _| async { Ok(()) });
    let child = child
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(2)),
            Mode::Resident,
        )
        .unwrap();
    let template = p.template("Child", &child);
    p.service("Acceptor")
        .needs(endpoint)
        .spawns(&template)
        .initialize(|_, _| async { Ok(()) })
        .serve(|_, _| async { Ok(()) });
    let p = p
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(10)),
            Mode::Resident,
        )
        .unwrap();
    let mut m = Machine::new(&p).unwrap();
    let endpoint = m.key_of("Endpoint").unwrap();
    let spawner = m.key_of("Acceptor").unwrap();
    let template = m.key_of("Child").unwrap();
    m.begin();
    checked(&mut m, Event::Held(endpoint));
    checked(&mut m, Event::NodeOk(endpoint));
    checked(&mut m, Event::NodeOk(spawner));
    (m, endpoint, spawner, template)
}

fn checked(m: &mut Machine, ev: Event) -> Vec<Effect> {
    let fx = m.step(ev);
    assert!(!fx.iter().any(|e| matches!(e, Effect::Reject(_))), "{fx:?}");
    fx
}

fn spawn(m: &mut Machine, spawner: RawKey, template: RawKey, id: u64) -> RawKey {
    checked(
        m,
        Event::InstanceSpawned {
            spawner,
            template,
            id: InstanceId(id),
        },
    );
    let key = m.instance_nodes(InstanceId(id))[0].0;
    checked(m, Event::Held(key));
    checked(m, Event::NodeOk(key));
    key
}

fn stop(m: &mut Machine, id: u64, key: RawKey) {
    checked(m, Event::StopInstance(InstanceId(id)));
    checked(m, Event::NodeOk(key));
}

#[test]
fn compact_churn_preserves_history_and_reclaims_execution_rows() {
    let (mut m, endpoint, spawner, template) = machine();
    let base = m.t.nodes.len();
    let mut keys = Vec::new();
    for id in 1..=100 {
        let key = spawn(&mut m, spawner, template, id);
        stop(&mut m, id, key);
        let nodes = m.nodes();
        let instance = m.instance_nodes(InstanceId(id));
        let state = m.state(key);
        let deadline = m.deadline_for(key);
        m.compact_ended_instances();
        assert_eq!(m.nodes(), nodes);
        assert_eq!(m.instance_nodes(InstanceId(id)), instance);
        assert_eq!(m.state(key), state);
        assert_eq!(m.deadline_for(key), deadline);
        assert_eq!(m.path_of(key), Some(&instance[0].2));
        assert_eq!(m.origin(key), Some((instance[0].1, Some(InstanceId(id)))));
        assert_eq!(m.kind_of(key), Some(instance[0].3));
        assert_eq!(m.t.nodes.len(), base);
        assert_eq!(m.slots.len(), base);
        assert_eq!(m.locks.len(), base);
        assert_eq!(m.scopes.len(), 1);
        assert_eq!(m.timers.capacity(), 0, "retired timer storage is reclaimed");
        assert!(m.instances.is_empty());
        keys.push(key);
    }
    assert_eq!(m.instances().len(), 100);
    assert_eq!(m.active_instance_states().count(), 0);
    for key in keys {
        assert_eq!(m.state(key), Some(NodeState::Released));
    }
    checked(&mut m, Event::ShutdownRequested);
    checked(
        &mut m,
        Event::ServeEnded {
            node: spawner,
            fault: None,
        },
    );
    checked(&mut m, Event::NodeOk(endpoint));
    assert!(m.take_report().unwrap().is_clean());
}

#[test]
fn compact_does_not_alias_sibling_and_keeps_identity_tombstones() {
    let (mut m, endpoint, spawner, template) = machine();
    let first = spawn(&mut m, spawner, template, 9);
    let sibling = spawn(&mut m, spawner, template, 3);
    stop(&mut m, 9, first);
    m.compact_ended_instances();
    assert_eq!(m.state(sibling), Some(NodeState::Ready));
    assert_eq!(
        m.instances(),
        vec![
            (InstanceId(9), RunState::Ended),
            (InstanceId(3), RunState::Steady)
        ]
    );
    checked(&mut m, Event::StopInstance(InstanceId(9)));
    assert!(matches!(
        m.step(Event::InstanceSpawned {
            spawner,
            template,
            id: InstanceId(9)
        })
        .as_slice(),
        [Effect::Reject(_)]
    ));
    assert!(matches!(
        m.step(Event::NodeOk(first)).as_slice(),
        [Effect::Reject(_)]
    ));
    assert_eq!(m.state(sibling), Some(NodeState::Ready));
    // Parent release must still wait for the sibling after its indices move.
    let fx = checked(&mut m, Event::ShutdownRequested);
    assert!(!fx
        .iter()
        .any(|e| matches!(e, Effect::Release(k) if *k == endpoint)));
    checked(
        &mut m,
        Event::ServeEnded {
            node: spawner,
            fault: None,
        },
    );
    checked(&mut m, Event::NodeOk(sibling));
    m.compact_ended_instances();
    checked(&mut m, Event::NodeOk(endpoint));
    assert!(m.take_report().unwrap().is_clean());
}

// A deterministic host that settles successful bodies and consumes the entire
// batch, including publication acknowledgements, before compaction.
fn complete(m: &mut Machine, initial: Vec<Effect>) {
    let mut queue: std::collections::VecDeque<_> = initial.into();
    while let Some(effect) = queue.pop_front() {
        let event = match effect {
            Effect::Spawn { node, .. } | Effect::SpawnBlocking { node, .. } => {
                if m.kind_of(node).unwrap().can_hold() {
                    queue.extend(checked(m, Event::Held(node)));
                }
                Some(Event::NodeOk(node))
            }
            Effect::PublishReady { node } => Some(Event::ReadyPublished {
                node,
                result: Ok(()),
            }),
            Effect::Release(node) | Effect::Compensate(node) | Effect::Recover(node) => {
                Some(Event::NodeOk(node))
            }
            Effect::StopService(node) | Effect::Signal(node) => {
                Some(Event::ServeEnded { node, fault: None })
            }
            Effect::Abort(node) => Some(Event::NodeCancelled { node, held: false }),
            Effect::Reject(r) => panic!("unexpected rejection: {r:?}"),
            _ => None,
        };
        if let Some(event) = event {
            queue.extend(checked(m, event));
        }
    }
}

#[test]
fn compact_nested_instances_keep_parent_ownership_and_remapped_template_imports() {
    let mut outer = Plan::builder("Outer");
    let value = outer
        .resource("Value")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(())) })
        .release(|_, _| async { Ok(()) });
    let mut inner = Plan::builder("Inner");
    let imported = inner.import(value);
    inner
        .resource("Leaf")
        .needs(imported)
        .acquire(|cx, _| async move { Ok(cx.hold_value(())) })
        .release(|_, _| async { Ok(()) });
    let inner = inner
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(1)),
            Mode::Resident,
        )
        .unwrap();
    let nested = outer.template("Inner", &inner);
    outer
        .service("NestedSpawner")
        .needs(value)
        .spawns(&nested)
        .initialize(|_, _| async { Ok(()) })
        .serve(|_, _| async { Ok(()) });
    let outer = outer
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(2)),
            Mode::Resident,
        )
        .unwrap();
    let mut root = Plan::builder("Root");
    let template = root.template("Outer", &outer);
    root.service("RootSpawner")
        .spawns(&template)
        .initialize(|_, ()| async { Ok(()) })
        .serve(|_, _| async { Ok(()) });
    let p = root
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(5)),
            Mode::Resident,
        )
        .unwrap();
    let mut m = Machine::new(&p).unwrap();
    let base = m.execution_node_count();
    let root_spawner = m.key_of("RootSpawner").unwrap();
    let root_template = m.key_of("Outer").unwrap();
    let fx = m.begin();
    complete(&mut m, fx);
    for id in [10, 20] {
        let fx = checked(
            &mut m,
            Event::InstanceSpawned {
                spawner: root_spawner,
                template: root_template,
                id: InstanceId(id),
            },
        );
        complete(&mut m, fx);
    }
    let sibling_nodes = m.instance_nodes(InstanceId(20));
    let spawner = sibling_nodes
        .iter()
        .find(|n| n.3 == crate::Kind::Service)
        .unwrap()
        .0;
    let template = sibling_nodes
        .iter()
        .find(|n| n.3 == crate::Kind::Template)
        .unwrap()
        .1;
    let fx = checked(&mut m, Event::StopInstance(InstanceId(10)));
    complete(&mut m, fx);
    m.compact_ended_instances();
    assert_eq!(m.instance_nodes(InstanceId(20)), sibling_nodes);
    for id in [30, 40] {
        let fx = checked(
            &mut m,
            Event::InstanceSpawned {
                spawner,
                template,
                id: InstanceId(id),
            },
        );
        complete(&mut m, fx);
        assert_eq!(m.instance_parent(InstanceId(id)), Some(InstanceId(20)));
        let fx = checked(&mut m, Event::StopInstance(InstanceId(id)));
        complete(&mut m, fx);
        m.compact_ended_instances();
        assert_eq!(m.instance_parent(InstanceId(id)), Some(InstanceId(20)));
        assert_eq!(m.instance_nodes(InstanceId(20)), sibling_nodes);
    }
    // A still-live nested child is stopped by its parent, then both are archived.
    let fx = checked(
        &mut m,
        Event::InstanceSpawned {
            spawner,
            template,
            id: InstanceId(50),
        },
    );
    complete(&mut m, fx);
    let fx = checked(&mut m, Event::StopInstance(InstanceId(20)));
    complete(&mut m, fx);
    m.compact_ended_instances();
    assert_eq!(m.execution_node_count(), base);
    assert_eq!(m.instance_parent(InstanceId(50)), Some(InstanceId(20)));
    let fx = checked(&mut m, Event::ShutdownRequested);
    complete(&mut m, fx);
    assert!(m.take_report().unwrap().is_clean());
}

#[test]
fn compact_preserves_cleanup_failure_records_and_sibling_timers() {
    let (mut m, endpoint, spawner, template) = machine();
    let first = spawn(&mut m, spawner, template, 1);
    checked(
        &mut m,
        Event::InstanceSpawned {
            spawner,
            template,
            id: InstanceId(2),
        },
    );
    let sibling = m.instance_nodes(InstanceId(2))[0].0;
    // An active node timer and active scope timer both carry dense references.
    let index = m.t.index_of(sibling).unwrap();
    let scope = m.t.nodes[index].scope;
    let timer = m.timer(super::state::Purpose::Within(index), Time::from_nanos(10));
    m.slots[index].timer = Some(timer);
    let budget = m.timer(super::state::Purpose::Budget(scope), Time::from_nanos(100));
    m.scopes[scope].budget_timer = Some(budget);
    m.fx.clear(); // This host has consumed the two timer registrations.
    checked(&mut m, Event::StopInstance(InstanceId(1)));
    checked(
        &mut m,
        Event::NodeErr(
            first,
            crate::FaultKind::Error(Box::new(std::io::Error::other("release failed"))),
        ),
    );
    let records = format!("{:?}", m.cleanup_failures);
    m.compact_ended_instances();
    assert_eq!(format!("{:?}", m.cleanup_failures), records);
    assert_eq!(m.state(first), Some(NodeState::ReleaseFailed));
    let fx = checked(&mut m, Event::Timer(timer));
    assert!(fx
        .iter()
        .any(|fx| matches!(fx, Effect::Abort(k) if *k == sibling)));
    checked(
        &mut m,
        Event::NodeCancelled {
            node: sibling,
            held: false,
        },
    );
    m.compact_ended_instances();
    // A forgotten timer remains harmless after its scope has been removed.
    checked(&mut m, Event::Timer(budget));
    let fx = checked(&mut m, Event::ShutdownRequested);
    complete(&mut m, fx);
    let report = m.take_report().unwrap();
    assert_eq!(report.cleanup_failures.len(), 1);
    assert!(m.state(endpoint).is_some());
}

#[test]
fn compact_batches_a_small_retired_tail_then_reclaims_it_as_churn_accumulates() {
    let (mut m, _, spawner, template) = machine();
    let base = m.execution_node_count();
    let keys: Vec<_> = (1..=4)
        .map(|id| spawn(&mut m, spawner, template, id))
        .collect();
    stop(&mut m, 1, keys[0]);
    m.compact_ended_instances();
    assert_eq!(
        m.execution_node_count(),
        base + 4,
        "one completion must not remap the whole live graph"
    );
    stop(&mut m, 2, keys[1]);
    m.compact_ended_instances();
    assert_eq!(
        m.execution_node_count(),
        base + 2,
        "reclaim once retired work matches live work"
    );
    for id in 3..=4 {
        stop(&mut m, id, keys[id as usize - 1]);
        m.compact_ended_instances();
    }
    assert_eq!(m.execution_node_count(), base);
    assert_eq!(m.instances().len(), 4);
}

#[test]
fn compact_archives_skipped_cause_before_moving_its_path() {
    let mut child = Plan::builder("Child");
    let r = child
        .resource("Cause")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(())) })
        .release(|_, _| async { Ok(()) });
    child.step("Skipped").needs(r).run(|_, _| async { Ok(()) });
    let child = child
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(1)),
            Mode::Resident,
        )
        .unwrap();
    let mut root = Plan::builder("Root");
    let t = root.template("Child", &child);
    root.service("Spawner")
        .spawns(&t)
        .initialize(|_, ()| async { Ok(()) })
        .serve(|_, _| async { Ok(()) });
    let p = root
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(2)),
            Mode::Resident,
        )
        .unwrap();
    let mut m = Machine::new(&p).unwrap();
    let spawner = m.key_of("Spawner").unwrap();
    let template = m.key_of("Child").unwrap();
    let fx = m.begin();
    complete(&mut m, fx);
    checked(
        &mut m,
        Event::InstanceSpawned {
            spawner,
            template,
            id: InstanceId(1),
        },
    );
    let nodes = m.instance_nodes(InstanceId(1));
    checked(
        &mut m,
        Event::NodeErr(
            nodes[0].0,
            crate::FaultKind::Error(Box::new(std::io::Error::other("startup failed"))),
        ),
    );
    let expected = Some(NodeState::Skipped {
        because: Some(nodes[0].2.clone()),
    });
    assert_eq!(m.state(nodes[1].0), expected);
    m.compact_ended_instances();
    assert_eq!(m.state(nodes[1].0), expected);
    assert_eq!(m.path_of(nodes[0].0), Some(&nodes[0].2));
    let fx = checked(&mut m, Event::ShutdownRequested);
    complete(&mut m, fx);
    let report = m.take_report().unwrap();
    assert_eq!(report.faults.len(), 1);
    assert_eq!(report.faults[0].node, nodes[0].2);
}
