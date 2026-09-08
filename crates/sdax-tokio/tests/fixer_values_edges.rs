//! Structural-value isolation, failure, and readiness controls.
use sdax::*;
use sdax_tokio::{PlanStart, RunOptions, TokioRuntime};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn shutdown() -> Shutdown {
    Shutdown::within(Duration::from_secs(10))
}
fn runtime() -> (tokio::runtime::Runtime, Arc<TokioRuntime>) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .unwrap();
    let adapter = Arc::new(TokioRuntime::current_thread_no_background_drain(
        rt.handle().clone(),
    ));
    (rt, adapter)
}
#[test]
fn failed_and_skipped_exports_never_construct_parent_consumer() {
    for skipped in [false, true] {
        let constructed = Arc::new(AtomicU32::new(0));
        let mut child = Plan::builder("child");
        let failure = child
            .step("failure")
            .run(|_, ()| async { Err::<u32, _>("export failure".into()) });
        let export = if skipped {
            child
                .step("skipped")
                .needs(failure)
                .run(|_, _: Arc<u32>| async { Ok(99u32) })
        } else {
            failure
        };
        let child = child
            .export(export)
            .build(Policy::Isolate, shutdown(), Mode::Finite)
            .unwrap();
        let mut root = Plan::builder("root");
        let component = root.component("child", &child, ());
        let counter = constructed.clone();
        root.step("consumer")
            .needs(component)
            .run(move |_, _: Arc<u32>| {
                counter.fetch_add(1, Ordering::SeqCst);
                async { Ok(()) }
            });
        let plan = root
            .build(Policy::Isolate, shutdown(), Mode::Finite)
            .unwrap();
        let (rt, adapter) = runtime();
        let report = rt.block_on(plan.start(adapter, ()));
        assert_eq!(report.outcome, Outcome::Failed);
        assert_eq!(constructed.load(Ordering::SeqCst), 0, "skipped={skipped}");
        assert!(!report.faults.is_empty());
    }
}
#[test]
fn isolated_unrelated_child_failure_retains_fault_and_delivers_export() {
    let mut child = Plan::builder("child");
    child
        .step("failure")
        .run(|_, ()| async { Err::<(), _>("unrelated".into()) });
    let value = child.step("value").run(|_, ()| async { Ok(99u32) });
    let child = child
        .export(value)
        .build(Policy::Isolate, shutdown(), Mode::Finite)
        .unwrap();
    let mut root = Plan::builder("root");
    let component = root.component("child", &child, ());
    let value = root
        .step("consumer")
        .needs(component)
        .run(|_, v: Arc<u32>| async move { Ok(*v + 1) });
    let plan = root
        .export(value)
        .build(Policy::Isolate, shutdown(), Mode::Finite)
        .unwrap();
    let (rt, adapter) = runtime();
    let report = rt.block_on(plan.start(adapter, ()));
    assert_eq!(report.output.as_deref(), Some(&100));
    assert_eq!(report.faults.len(), 1);
    assert_eq!(report.faults[0].node.leaf(), "failure");
}
#[test]
fn concurrent_template_instances_publish_component_values_before_child_ready() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut template = Plan::with_input::<u32>("template");
    let input = template.input();
    let mut child = Plan::builder("child");
    let imported = child.import(input);
    let value = child
        .step("value")
        .needs(imported)
        .run(|_, v: Arc<u32>| async move { Ok(*v) });
    let child = child
        .export(value)
        .build(Policy::FailFast, shutdown(), Mode::Finite)
        .unwrap();
    let component = template.component("child", &child, ());
    let values = seen.clone();
    template
        .step("consume")
        .needs(component)
        .run(move |_, value: Arc<u32>| {
            values.lock().unwrap().push(*value);
            async { Ok(()) }
        });
    let template = template
        .build(Policy::FailFast, shutdown(), Mode::Finite)
        .unwrap();
    let mut root = Plan::builder("root");
    let registered = root.template("template", &template);
    let at_ready = seen.clone();
    root.service("owner")
        .spawns(&registered)
        .stop_within(Duration::from_secs(1))
        .initialize(move |cx, ()| {
            let at_ready = at_ready.clone();
            async move {
                let first = cx.spawn(&registered, 11u32)?;
                let second = cx.spawn(&registered, 22u32)?;
                first.ready().await?;
                second.ready().await?;
                let mut observed = at_ready.lock().unwrap().clone();
                observed.sort_unstable();
                assert_eq!(observed, vec![11, 22]);
                Ok(())
            }
        })
        .serve(|_cx, _handle| async { Ok(()) });
    let plan = root
        .build(Policy::FailFast, shutdown(), Mode::Resident)
        .unwrap();
    let (rt, adapter) = runtime();
    let report = rt.block_on(async {
        let mut running = plan.start(adapter, ());
        running.ready().await.unwrap();
        running.shutdown();
        running.await
    });
    assert_eq!(report.outcome, Outcome::Ok);
    let mut observed = seen.lock().unwrap().clone();
    observed.sort_unstable();
    assert_eq!(observed, vec![11, 22]);
}
#[test]
fn resident_ready_without_observation_on_multithread_has_real_value() {
    let mut child = Plan::builder("child");
    let output = child.step("value").run(|_, ()| async { Ok(99u32) });
    let child = child
        .export(output)
        .build(Policy::FailFast, shutdown(), Mode::Finite)
        .unwrap();
    let mut root = Plan::builder("root");
    let component = root.component("child", &child, ());
    let seen = Arc::new(AtomicU32::new(0));
    let value = seen.clone();
    root.service("consumer")
        .needs(component)
        .stop_within(Duration::from_secs(1))
        .initialize(move |_, input: Arc<u32>| {
            value.store(*input, Ordering::SeqCst);
            async { Ok(()) }
        })
        .serve(|_cx, _handle| async { Ok(()) });
    let plan = root
        .export(component)
        .build(Policy::FailFast, shutdown(), Mode::Resident)
        .unwrap();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_time()
        .build()
        .unwrap();
    let adapter = Arc::new(TokioRuntime::new(rt.handle().clone()));
    let report = rt.block_on(async {
        let mut running = plan.start_with(adapter, (), RunOptions::new());
        running.ready().await.unwrap();
        assert_eq!(seen.load(Ordering::SeqCst), 99);
        running.shutdown();
        running.await
    });
    assert_eq!(report.outcome, Outcome::Ok);
    assert_eq!(report.output.as_deref(), Some(&99));
}

#[test]
fn nested_template_component_reads_own_input_and_ancestor_import() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut outer = Plan::with_input::<u32>("outer");
    let outer_input = outer.input();
    let mut inner = Plan::with_input::<u32>("inner");
    let inner_input = inner.input();
    let ancestor = inner.import(outer_input);
    let mut component = Plan::builder("component");
    let own = component.import(inner_input);
    let parent = component.import(ancestor);
    let sum = component
        .step("sum")
        .needs((own, parent))
        .run(|_, d: (Arc<u32>, Arc<u32>)| async move { Ok((*d.0, *d.1)) });
    let component = component
        .export(sum)
        .build(Policy::FailFast, shutdown(), Mode::Finite)
        .unwrap();
    let component = inner.component("component", &component, ());
    let values = seen.clone();
    inner
        .step("consume")
        .needs(component)
        .run(move |_, pair: Arc<(u32, u32)>| {
            values.lock().unwrap().push(*pair);
            async { Ok(()) }
        });
    let inner = inner
        .build(Policy::FailFast, shutdown(), Mode::Finite)
        .unwrap();
    let nested = outer.template("inner", &inner);
    outer
        .service("nested_owner")
        .needs(outer_input)
        .spawns(&nested)
        .stop_within(Duration::from_secs(1))
        .initialize(move |cx, input: Arc<u32>| async move {
            cx.spawn(&nested, *input + 100)?.ready().await?;
            Ok(())
        })
        .serve(|_cx, _handle| async { Ok(()) });
    let outer = outer
        .build(Policy::FailFast, shutdown(), Mode::Resident)
        .unwrap();
    let mut root = Plan::builder("root");
    let instances = root.template("outer", &outer);
    root.service("owner")
        .spawns(&instances)
        .stop_within(Duration::from_secs(1))
        .initialize(move |cx, ()| async move {
            let a = cx.spawn(&instances, 11u32)?;
            let b = cx.spawn(&instances, 22u32)?;
            a.ready().await?;
            b.ready().await?;
            Ok(())
        })
        .serve(|_cx, _handle| async { Ok(()) });
    let plan = root
        .build(Policy::FailFast, shutdown(), Mode::Resident)
        .unwrap();
    let (rt, adapter) = runtime();
    let report = rt.block_on(async {
        let mut running = plan.start(adapter, ());
        running.ready().await.unwrap();
        let mut observed = seen.lock().unwrap().clone();
        observed.sort_unstable();
        assert_eq!(observed, vec![(111, 11), (122, 22)]);
        running.shutdown();
        running.await
    });
    assert_eq!(report.outcome, Outcome::Ok);
}
