use super::*;
use crate::running::Control;
use crate::scope::RunScope;
use crate::TokioRuntime;
use sdax::host::engine::{Event, Machine};
use sdax::host::{CxInner, InstanceId, RawKey, Time};
use sdax::{Mode, Plan, Policy, Shutdown, TraceKind};
use std::sync::Arc;
use std::time::Duration;

fn plan() -> Plan {
    let mut p = Plan::builder("Retention");
    p.step("S").run(|_cx, ()| async move { Ok(()) });
    p.build(
        Policy::FailFast,
        Shutdown::within(Duration::from_secs(1)),
        Mode::Finite,
    )
    .expect("valid plan")
}

fn driver() -> (Driver<TokioRuntime>, RawKey, tokio::runtime::Runtime) {
    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("runtime");
    let rt = Arc::new(TokioRuntime::current_thread_no_background_drain(
        tokio_rt.handle().clone(),
    ));
    let p = plan();
    let machine = Machine::new(&p).expect("machine");
    let key = machine.key_of("S").expect("step key");
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let (done, _done_rx) = tokio::sync::oneshot::channel();
    let d = Driver::new(
        rt,
        machine,
        sdax::host::bodies_of::<(), ()>(&p),
        tx.clone(),
        rx,
        Arc::new(Control::detached(tx.clone())),
        None,
        done,
        RunScope::new(tx),
    );
    (d, key, tokio_rt)
}

#[test]
fn ended_instance_context_is_dropped_and_late_body_is_ignored() {
    let (mut d, key, _rt) = driver();
    let id = InstanceId(7);
    let cx = CxInner::new(key, Arc::new(TestClock));
    let weak = Arc::downgrade(&cx);
    d.origins.insert(key, (key, Some(id)));
    d.live.insert(
        key,
        Live {
            epoch: 11,
            cx,
            handle: None,
        },
    );
    d.perform(Effect::Emit(Box::new(sdax::TraceEvent::node(
        Time::ZERO,
        sdax::NodePath::root("S"),
        Default::default(),
        TraceKind::InstanceEnded(id, sdax::Outcome::Ok),
    ))));
    assert!(weak.upgrade().is_none(), "ended context must be released");
    d.handle(Msg::Body {
        node: key,
        epoch: 11,
        ev: Event::NodeOk(key),
    });
    assert!(d
        .trace
        .events
        .iter()
        .all(|e| { !matches!(e.kind, TraceKind::Rejected(_)) }));
}

#[test]
fn context_count_is_cumulative_and_siblings_are_isolated() {
    let (mut d, key, tokio_rt) = driver();
    let sibling = RawKey {
        plan: key.plan,
        idx: key.idx + 1,
    };
    let parent = InstanceId(20);
    let child = InstanceId(21);
    let parent_key = RawKey {
        plan: key.plan + 1,
        idx: 0,
    };
    d.origins.insert(parent_key, (key, Some(parent)));
    d.live.insert(
        parent_key,
        Live {
            epoch: 1,
            cx: CxInner::new(parent_key, Arc::new(TestClock)),
            handle: None,
        },
    );
    d.origins.insert(sibling, (sibling, Some(child)));
    let child_cx = CxInner::new(sibling, Arc::new(TestClock));
    let child_weak = Arc::downgrade(&child_cx);
    d.live.insert(
        sibling,
        Live {
            epoch: 2,
            cx: child_cx,
            handle: None,
        },
    );
    d.spawn_body(key, 1);
    assert_eq!(d.contexts, 1);
    d.spawn_body(key, 2);
    assert_eq!(d.contexts, 1, "a retry reuses the run key");
    d.retire_instance(parent);
    assert_eq!(d.contexts, 1, "retirement preserves the cumulative count");
    assert!(!d.live.contains_key(&parent_key));
    assert!(d.live.contains_key(&sibling));
    assert!(child_weak.upgrade().is_some(), "sibling remains live");
    drop(tokio_rt);
}

struct TestClock;
impl sdax::host::Clock for TestClock {
    fn now(&self) -> Time {
        Time::ZERO
    }

    fn sleep(&self, _d: Duration) -> sdax::host::BoxFuture<'static, ()> {
        Box::pin(async {})
    }
}

#[test]
fn driver_compacts_only_after_consuming_instance_end_effects() {
    let mut child = Plan::builder("Child");
    child
        .resource("R")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(())) })
        .release(|_, _| async { Ok(()) });
    let child = child
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(1)),
            Mode::Resident,
        )
        .unwrap();
    let mut p = Plan::builder("Root");
    let template = p.template("Child", &child);
    p.service("Spawner")
        .spawns(&template)
        .initialize(|_, ()| async { Ok(()) })
        .serve(|_, _| async { Ok(()) });
    let p = p
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(2)),
            Mode::Resident,
        )
        .unwrap();
    let mut m = Machine::new(&p).unwrap();
    let base = m.execution_node_count();
    let spawner = m.key_of("Spawner").unwrap();
    let template = m.key_of("Child").unwrap();
    m.begin();
    m.step(Event::NodeOk(spawner));
    let id = InstanceId(42);
    m.step(Event::InstanceSpawned {
        spawner,
        template,
        id,
    });
    let key = m.instance_nodes(id)[0].0;
    m.step(Event::Held(key));
    m.step(Event::NodeOk(key));
    m.step(Event::StopInstance(id));
    let ended_effects = m.step(Event::NodeOk(key));
    assert!(ended_effects.iter().any(
        |fx| matches!(fx, Effect::Emit(ev) if matches!(ev.kind, TraceKind::InstanceEnded(..)))
    ));
    assert_eq!(
        m.execution_node_count(),
        base + 1,
        "returned effects still have their topology"
    );
    let (mut d, _, _runtime) = driver();
    d.machine = m;
    d.src = sdax::host::bodies_of::<(), ()>(&p);
    d.origins.insert(key, (key, Some(id)));
    let cx = CxInner::new(key, Arc::new(TestClock));
    let weak = Arc::downgrade(&cx);
    d.live.insert(
        key,
        Live {
            epoch: 1,
            cx,
            handle: None,
        },
    );
    d.after("instance ended".into(), ended_effects);
    assert!(weak.upgrade().is_none());
    assert_eq!(d.machine.execution_node_count(), base);
    assert!(
        d.machine.origin(key).is_some(),
        "historical identity remains"
    );
}
