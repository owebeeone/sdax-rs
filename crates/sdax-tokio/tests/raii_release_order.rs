//! Actual destructor ordering, including engine-managed aliases.
use sdax::*;
use sdax_tokio::{PlanStart, TokioRuntime};
use std::sync::{Arc, Mutex};
use std::time::Duration;

type Events = Arc<Mutex<Vec<&'static str>>>;

struct DropWitness {
    events: Events,
    panic: bool,
}

impl Drop for DropWitness {
    fn drop(&mut self) {
        self.events.lock().unwrap().push("child dropped");
        assert!(!self.panic, "destructor failure");
    }
}

fn run<O: Send + Sync + 'static>(plan: &Plan<O>) -> Report<O> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .unwrap();
    let host = Arc::new(TokioRuntime::current_thread_no_background_drain(
        runtime.handle().clone(),
    ));
    let report = runtime.block_on(plan.start(host.clone(), ()));
    assert_eq!(host.tracked(), 0);
    report
}

#[test]
fn raii_dependent_is_actually_dropped_before_parent_release() {
    let events: Events = Arc::new(Mutex::new(Vec::new()));
    let mut p = Plan::builder("RAII order");
    let events_parent = events.clone();
    let parent = p
        .resource("Parent")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(1_u64)) })
        .release(move |_, _| {
            let events = events_parent.clone();
            async move {
                events.lock().unwrap().push("parent released");
                Ok(())
            }
        });
    let events_child = events.clone();
    p.resource("Child")
        .needs(parent)
        .acquire(move |cx, _| {
            let events = events_child.clone();
            async move {
                Ok(cx.hold_value(DropWitness {
                    events,
                    panic: false,
                }))
            }
        })
        .release(release::by_drop());
    let p = p
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(1)),
            Mode::Finite,
        )
        .unwrap();
    let report = run(&p);
    assert!(report.is_clean(), "{report}");
    assert_eq!(
        *events.lock().unwrap(),
        ["child dropped", "parent released"]
    );
}

fn parent_resource(p: &mut PlanBuilder, events: &Events) -> Key<u64> {
    let events = events.clone();
    p.resource("Parent")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(1_u64)) })
        .release(move |_, _| {
            let events = events.clone();
            async move {
                events.lock().unwrap().push("parent released");
                Ok(())
            }
        })
}

#[test]
fn repeated_static_input_and_formal_import_aliases_do_not_delay_drop() {
    let events: Events = Arc::new(Mutex::new(Vec::new()));
    let mut borrowed = Plan::with_input::<DropWitness>("borrowed");
    let input = borrowed.input();
    let port = borrowed.port::<DropWitness>("formal");
    borrowed.step("read").needs((input, port)).run(
        |_, values: (Arc<DropWitness>, Arc<DropWitness>)| async move {
            assert!(Arc::ptr_eq(&values.0, &values.1));
            values.0.events.lock().unwrap().push("alias read");
            Ok(())
        },
    );
    let borrowed = borrowed
        .build(Policy::FailFast, Shutdown::unbounded(), Mode::Finite)
        .unwrap();
    let mut p = Plan::builder("static aliases");
    let parent = parent_resource(&mut p, &events);
    let acquired = events.clone();
    let child = p
        .resource("Child")
        .needs(parent)
        .acquire(move |cx, _| {
            let events = acquired.clone();
            async move {
                Ok(cx.hold_value(DropWitness {
                    events,
                    panic: false,
                }))
            }
        })
        .release(release::by_drop());
    let bound = borrowed.bind(port, child).unwrap();
    p.component("left", &bound, child);
    p.component("right", &bound, child);
    let plan = p
        .build(Policy::FailFast, Shutdown::unbounded(), Mode::Finite)
        .unwrap();
    assert!(run(&plan).is_clean());
    assert_eq!(
        *events.lock().unwrap(),
        [
            "alias read",
            "alias read",
            "child dropped",
            "parent released"
        ]
    );
}

#[test]
fn nested_resident_component_exports_are_disposed_before_parent_cleanup() {
    let events: Events = Arc::new(Mutex::new(Vec::new()));
    let mut p = Plan::builder("export aliases");
    let parent = parent_resource(&mut p, &events);
    let mut leaf = Plan::with_input::<u64>("leaf");
    let input = leaf.input();
    let acquired = events.clone();
    let child = leaf
        .resource("Child")
        .needs(input)
        .acquire(move |cx, _| {
            let events = acquired.clone();
            async move {
                Ok(cx.hold_value(DropWitness {
                    events,
                    panic: false,
                }))
            }
        })
        .release(release::by_drop());
    let leaf = leaf
        .export(child)
        .build(Policy::FailFast, Shutdown::unbounded(), Mode::Resident)
        .unwrap();
    let mut middle = Plan::with_input::<u64>("middle");
    let child = middle.component("leaf", &leaf, middle.input());
    let middle = middle
        .export(child)
        .build(Policy::FailFast, Shutdown::unbounded(), Mode::Resident)
        .unwrap();
    let exported = p.component("middle", &middle, parent);
    p.step("read")
        .needs(exported)
        .run(|_, value: Arc<DropWitness>| async move {
            value.events.lock().unwrap().push("alias read");
            Ok(())
        });
    let plan = p
        .build(Policy::FailFast, Shutdown::unbounded(), Mode::Finite)
        .unwrap();
    assert!(run(&plan).is_clean());
    assert_eq!(
        *events.lock().unwrap(),
        ["alias read", "child dropped", "parent released"]
    );
}

#[test]
fn destructor_panic_is_a_cleanup_failure_and_upstream_cleanup_continues() {
    let events: Events = Arc::new(Mutex::new(Vec::new()));
    let mut p = Plan::builder("drop panic");
    let parent = parent_resource(&mut p, &events);
    let acquired = events.clone();
    p.resource("Child")
        .needs(parent)
        .acquire(move |cx, _| {
            let events = acquired.clone();
            async move {
                Ok(cx.hold_value(DropWitness {
                    events,
                    panic: true,
                }))
            }
        })
        .release(release::by_drop());
    let plan = p
        .build(Policy::FailFast, Shutdown::unbounded(), Mode::Finite)
        .unwrap();
    let report = run(&plan);
    assert_eq!(report.cleanup_failures.len(), 1, "{report}");
    assert_eq!(report.cleanup_failures[0].phase, Phase::ReleaseBody);
    assert_eq!(report.cleanup_failures[0].node, NodePath::root("Child"));
    assert!(matches!(
        report.cleanup_failures[0].kind,
        FaultKind::Panic(_)
    ));
    assert_eq!(
        *events.lock().unwrap(),
        ["child dropped", "parent released"]
    );
}

#[test]
fn explicit_caller_clone_can_outlive_declared_raii_cleanup() {
    let events: Events = Arc::new(Mutex::new(Vec::new()));
    let retained = Arc::new(Mutex::new(None));
    let mut p = Plan::builder("caller clone");
    let parent = parent_resource(&mut p, &events);
    let acquired = events.clone();
    let child = p
        .resource("Child")
        .needs(parent)
        .acquire(move |cx, _| {
            let events = acquired.clone();
            async move {
                Ok(cx.hold_value(DropWitness {
                    events,
                    panic: false,
                }))
            }
        })
        .release(release::by_drop());
    let keep = retained.clone();
    p.step("keep")
        .needs(child)
        .run(move |_, value: Arc<DropWitness>| {
            let keep = keep.clone();
            async move {
                *keep.lock().unwrap() = Some(value);
                Ok(())
            }
        });
    let plan = p
        .build(Policy::FailFast, Shutdown::unbounded(), Mode::Finite)
        .unwrap();
    assert!(run(&plan).is_clean());
    assert_eq!(*events.lock().unwrap(), ["parent released"]);
    let caller_owned = retained.lock().unwrap().take();
    drop(caller_owned);
    assert_eq!(
        *events.lock().unwrap(),
        ["parent released", "child dropped"]
    );
}

#[test]
fn dynamic_instances_dispose_only_their_own_nested_aliases() {
    type TaggedEvents = Arc<Mutex<Vec<(u64, &'static str)>>>;
    struct TaggedDrop(u64, TaggedEvents);
    impl Drop for TaggedDrop {
        fn drop(&mut self) {
            self.1.lock().unwrap().push((self.0, "dropped"));
        }
    }
    let events: TaggedEvents = Arc::new(Mutex::new(Vec::new()));
    let mut reader = Plan::with_input::<TaggedDrop>("reader");
    let input = reader.input();
    reader
        .step("read")
        .needs(input)
        .run(|_, value: Arc<TaggedDrop>| async move {
            value.1.lock().unwrap().push((value.0, "read"));
            Ok(())
        });
    let reader = reader
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(1)),
            Mode::Finite,
        )
        .unwrap();
    let mut instance = Plan::with_input::<u64>("instance");
    let input = instance.input();
    let released = events.clone();
    let parent = instance
        .resource("Parent")
        .needs(input)
        .acquire(|cx, value: Arc<u64>| async move { Ok(cx.hold_value(*value)) })
        .release(move |_, value| {
            let events = released.clone();
            async move {
                events.lock().unwrap().push((*value, "released"));
                Ok(())
            }
        });
    let acquired = events.clone();
    let child = instance
        .resource("Child")
        .needs(parent)
        .acquire(move |cx, value: Arc<u64>| {
            let events = acquired.clone();
            async move { Ok(cx.hold_value(TaggedDrop(*value, events))) }
        })
        .release(release::by_drop());
    instance.component("left", &reader, child);
    instance.component("right", &reader, child);
    let instance = instance
        .build(
            Policy::Isolate,
            Shutdown::within(Duration::from_secs(1)),
            Mode::Resident,
        )
        .unwrap();
    let mut root = Plan::with_input::<u64>("dynamic aliases");
    let run_input = root.input();
    let template = root.template("instances", &instance);
    root.service("owner")
        .needs(run_input)
        .spawns(&template)
        .stop_within(Duration::from_secs(1))
        .initialize(move |cx, run: Arc<u64>| async move {
            let first = cx.spawn(&template, *run * 10 + 7)?;
            let second = cx.spawn(&template, *run * 10 + 9)?;
            first.ready().await?;
            second.ready().await?;
            Ok(())
        })
        .serve(|cx, _| async move {
            cx.stop().await;
            Ok(())
        });
    let plan = root
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(2)),
            Mode::Resident,
        )
        .unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .unwrap();
    let host = Arc::new(TokioRuntime::current_thread_no_background_drain(
        runtime.handle().clone(),
    ));
    let reports = runtime.block_on(async {
        let mut first = plan.start(host.clone(), 1);
        let mut second = plan.start(host.clone(), 2);
        first.ready().await.unwrap();
        second.ready().await.unwrap();
        first.shutdown();
        let first = first.await;
        assert!(!events
            .lock()
            .unwrap()
            .iter()
            .any(|(id, phase)| *id >= 20 && *phase == "dropped"));
        second.shutdown();
        [first, second.await]
    });
    for report in reports {
        assert!(report.is_clean(), "{report}");
    }
    assert_eq!(host.tracked(), 0);
    let events = events.lock().unwrap();
    for id in [17, 19, 27, 29] {
        let own: Vec<_> = events
            .iter()
            .filter(|(who, _)| *who == id)
            .map(|(_, what)| *what)
            .collect();
        assert_eq!(own, ["read", "read", "dropped", "released"]);
    }
}
