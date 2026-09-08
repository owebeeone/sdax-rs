use sdax::*;
use sdax_tokio::{PlanStart, TokioRuntime};
use std::sync::Arc;
use std::time::Duration;

fn build<O, I>(p: PlanBuilder<O, I>) -> Plan<O, I> {
    p.build(
        Policy::FailFast,
        Shutdown::within(Duration::from_secs(5)),
        Mode::Finite,
    )
    .unwrap()
}

#[test]
fn typed_repeated_mounts_and_concurrent_runs_are_independent() {
    let mut c = Plan::with_input::<u32>("child");
    let input = c.input();
    let output = c
        .step("double")
        .needs(input)
        .run(|_, n: Arc<u32>| async move { Ok(*n * 2) });
    let child = build(c.export(output));
    let mut p = Plan::with_input::<u32>("parent");
    let input = p.input();
    let other = p
        .step("other")
        .needs(input)
        .run(|_, n: Arc<u32>| async move { Ok(*n + 10) });
    let left = p.component("left", &child, input);
    let right = p.component("right", &child, other);
    let answer = p
        .step("answer")
        .needs((left, right))
        .run(|_, x: (Arc<u32>, Arc<u32>)| async move { Ok((*x.0, *x.1)) });
    let parent = build(p.export(answer));
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    let host = Arc::new(TokioRuntime::current_thread_no_background_drain(
        rt.handle().clone(),
    ));
    rt.block_on(async {
        let a = parent.start(host.clone(), 3);
        let b = parent.start(host.clone(), 7);
        assert_eq!(a.await.into_result().unwrap().as_deref(), Some(&(6, 26)));
        assert_eq!(b.await.into_result().unwrap().as_deref(), Some(&(14, 34)));
    });
}

#[test]
fn same_formal_port_definition_binds_in_different_parents() {
    let mut c = Plan::with_input::<u32>("child");
    let port = c.port::<u32>("rate");
    let input = c.input();
    let output = c
        .step("multiply")
        .needs((input, port))
        .run(|_, x: (Arc<u32>, Arc<u32>)| async move { Ok(*x.0 * *x.1) });
    let child = build(c.export(output));
    let parent = |name, rate| {
        let mut p = Plan::with_input::<u32>(name);
        let input = p.input();
        let rate = p.step("rate").run(move |_, ()| async move { Ok(rate) });
        let bound = child.bind(port, rate).unwrap();
        let output = p.component("child", &bound, input);
        build(p.export(output))
    };
    let a = parent("a", 2u32);
    let b = parent("b", 5u32);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    let host = Arc::new(TokioRuntime::current_thread_no_background_drain(
        rt.handle().clone(),
    ));
    rt.block_on(async {
        assert_eq!(a.start(host.clone(), 3).await.output.as_deref(), Some(&6));
        assert_eq!(b.start(host.clone(), 3).await.output.as_deref(), Some(&15));
    });
}

#[test]
fn unit_input_is_a_regular_explicit_mount_binding() {
    let mut c = Plan::builder("child");
    let input = c.input();
    c.step("read")
        .needs(input)
        .run(|_, _: Arc<()>| async { Ok(()) });
    let child = build(c);
    let mut p = Plan::with_input::<()>("parent");
    p.component("child", &child, p.input());
    let parent = build(p);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    let host = Arc::new(TokioRuntime::current_thread_no_background_drain(
        rt.handle().clone(),
    ));
    let report = rt.block_on(async { parent.start(host, ()).await });
    assert_eq!(report.outcome, Outcome::Ok);
}

#[test]
fn nested_repeated_mounts_resolve_captured_template_handles_locally() {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut leaf = Plan::with_input::<u32>("leaf");
    let input = leaf.input();
    let sink = seen.clone();
    leaf.step("record").needs(input).run(move |_, n: Arc<u32>| {
        let sink = sink.clone();
        async move {
            sink.lock().unwrap().push(*n);
            Ok(())
        }
    });
    let leaf = leaf
        .build(
            Policy::Isolate,
            Shutdown::within(Duration::from_secs(1)),
            Mode::Finite,
        )
        .unwrap();
    let mut child = Plan::with_input::<u32>("child");
    let input = child.input();
    let template = child.template("leaf", &leaf);
    child
        .service("spawn")
        .needs(input)
        .spawns(&template)
        .stop_within(Duration::from_secs(1))
        .initialize(move |cx, n: Arc<u32>| async move {
            let instance = cx.spawn(&template, *n)?;
            instance.ready().await?;
            Ok(())
        })
        .serve(|cx, _| async move {
            cx.stop().await;
            Ok(())
        });
    let child = child
        .build(
            Policy::Isolate,
            Shutdown::within(Duration::from_secs(2)),
            Mode::Resident,
        )
        .unwrap();
    let mut middle = Plan::with_input::<u32>("middle");
    middle.component("child", &child, middle.input());
    let middle = middle
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(3)),
            Mode::Resident,
        )
        .unwrap();
    let mut parent = Plan::builder("parent");
    let a = parent.step("a").run(|_, ()| async { Ok(13u32) });
    let b = parent.step("b").run(|_, ()| async { Ok(29u32) });
    parent.component("left", &middle, a);
    parent.component("right", &middle, b);
    let plan = parent
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(5)),
            Mode::Resident,
        )
        .unwrap();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .unwrap();
    let host = Arc::new(TokioRuntime::current_thread_no_background_drain(
        rt.handle().clone(),
    ));
    rt.block_on(async {
        let mut run = plan.start(host, ());
        run.ready().await.unwrap();
        run.shutdown();
        assert_eq!(run.await.outcome, Outcome::Ok);
    });
    let mut seen = seen.lock().unwrap().clone();
    seen.sort();
    assert_eq!(seen, [13, 29]);
}

#[test]
fn missing_formal_bindings_and_foreign_input_fail_parent_build() {
    let mut child = Plan::builder("child");
    let port = child.port::<u32>("rate");
    child
        .step("use")
        .needs(port)
        .run(|_, _: Arc<u32>| async { Ok(()) });
    let child = build(child);
    let mut parent = Plan::builder("parent");
    parent.component("child", &child, ());
    let invalid = parent
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(5)),
            Mode::Finite,
        )
        .unwrap_err();
    assert!(invalid
        .checks
        .iter()
        .any(|f| f.rule == Rule::ImportScope && f.nodes.iter().any(|n| n.contains("rate"))));
    let mut child = Plan::with_input::<u32>("typed");
    child.step("step").run(|_, ()| async { Ok(()) });
    let child = build(child);
    let foreign = Plan::with_input::<u32>("foreign").input();
    let mut parent = Plan::builder("parent");
    parent.component("child", &child, foreign);
    assert!(parent
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(5)),
            Mode::Finite
        )
        .is_err());
}

#[test]
fn resource_input_and_formal_import_remain_alive_through_each_mount_cleanup() {
    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut child = Plan::with_input::<u32>("child");
    let input = child.input();
    let extra = child.port::<u32>("extra");
    let pool = child.pool("one", 1);
    let record = events.clone();
    let held = child
        .resource("held")
        .needs((input, extra))
        .limit(pool)
        .acquire(|cx, (a, b): (Arc<u32>, Arc<u32>)| async move { Ok(cx.hold_value((*a, *b))) })
        .release(move |_, value: Arc<(u32, u32)>| {
            let record = record.clone();
            async move {
                record.lock().unwrap().push((value.0, "child"));
                Ok(())
            }
        });
    let out = child
        .step("out")
        .needs(held)
        .run(|_, value: Arc<(u32, u32)>| async move { Ok(value.0 + value.1) });
    let child = build(child.export(out));
    let mut parent = Plan::with_input::<u32>("parent");
    let input = parent.input();
    let record = events.clone();
    let resource = parent
        .resource("parent")
        .needs(input)
        .acquire(|cx, n: Arc<u32>| async move { Ok(cx.hold_value(*n)) })
        .release(move |_, n: Arc<u32>| {
            let record = record.clone();
            async move {
                record.lock().unwrap().push((*n, "parent"));
                Ok(())
            }
        });
    let bound = child.bind(extra, resource).unwrap();
    let a = parent.component("a", &bound, resource);
    let b = parent.component("b", &bound, resource);
    let total = parent
        .step("total")
        .needs((a, b))
        .run(|_, x: (Arc<u32>, Arc<u32>)| async move { Ok(*x.0 + *x.1) });
    let plan = build(parent.export(total));
    let view = plan.inspect();
    assert_eq!(
        view.node("a/held").unwrap().needs,
        vec![NodePath::root("parent"), NodePath::root("parent")]
    );
    assert!(view.release_order().before("a/held", "parent"));

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .unwrap();
    let host = Arc::new(TokioRuntime::current_thread_no_background_drain(
        rt.handle().clone(),
    ));
    rt.block_on(async {
        let a = plan.start(host.clone(), 3);
        let b = plan.start(host.clone(), 7);
        assert_eq!(a.await.output.as_deref(), Some(&12));
        assert_eq!(b.await.output.as_deref(), Some(&28));
    });
    let events = events.lock().unwrap();
    for id in [3, 7] {
        let order: Vec<_> = events
            .iter()
            .filter(|(n, _)| *n == id)
            .map(|(_, e)| *e)
            .collect();
        assert_eq!(order, ["child", "child", "parent"]);
    }
}

#[test]
fn sibling_mount_pools_admit_independently() {
    use sdax::host::engine::{Effect, Machine};
    let mut child = Plan::builder("child");
    let pool = child.pool("one", 1);
    child.step("work").limit(pool).run(|_, ()| async { Ok(()) });
    let child = build(child);
    let mut parent = Plan::builder("parent");
    parent.component("left", &child, ());
    parent.component("right", &child, ());
    let parent = build(parent);
    let mut machine = Machine::new(&parent).unwrap();
    let starts: Vec<_> = machine
        .begin()
        .into_iter()
        .filter_map(|effect| {
            if let Effect::Spawn { node, .. } = effect {
                Some(machine.path_of(node).unwrap().to_string())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(starts, ["left/work", "right/work"]);
}
