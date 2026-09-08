//! Explicit unit bindings work at every static and dynamic boundary.
use sdax::host::engine::Machine;
use sdax::*;
use sdax_tokio::{PlanStart, TokioRuntime};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn budget() -> Shutdown {
    Shutdown::within(Duration::from_secs(5))
}

fn bound_plan(
    template_constructor: bool,
    used: bool,
    placement: u8,
    calls: Arc<AtomicUsize>,
) -> (Plan<()>, String) {
    let mut bad = if template_constructor {
        Plan::builder("bad")
    } else {
        Plan::with_input::<()>("bad")
    };
    if used {
        let input = bad.input();
        bad.step("Read")
            .needs(input)
            .run(|_, _: Arc<()>| async { Ok(()) });
    } else {
        bad.step("Unused").run(|_, ()| async { Ok(()) });
    }
    let bad = bad.build(Policy::FailFast, budget(), Mode::Finite).unwrap();
    let mut root = Plan::builder("root");
    root.resource("Probe")
        .acquire(move |cx, ()| {
            calls.fetch_add(1, Ordering::SeqCst);
            async move { Ok(cx.hold_value(())) }
        })
        .release(|_, _| async { Ok(()) });
    let path = match placement {
        0 => {
            root.component("Bad", &bad, ());
            "Bad/input"
        }
        1 => {
            let mut middle = Plan::builder("middle");
            middle.component("Bad", &bad, ());
            let middle = middle
                .build(Policy::FailFast, budget(), Mode::Finite)
                .unwrap();
            root.component("Middle", &middle, ());
            "Middle/Bad/input"
        }
        _ => {
            let mut template = Plan::with_input::<u32>("template");
            let input = template.input();
            template
                .step("OwnInput")
                .needs(input)
                .run(|_, _: Arc<u32>| async { Ok(()) });
            template.component("Bad", &bad, ());
            let template = template
                .build(Policy::FailFast, budget(), Mode::Finite)
                .unwrap();
            if placement == 2 {
                root.template("Factory", &template);
                "Factory/Bad/input"
            } else {
                let mut outer = Plan::with_input::<u16>("outer");
                outer.template("Inner", &template);
                let outer = outer
                    .build(Policy::FailFast, budget(), Mode::Resident)
                    .unwrap();
                root.template("Factory", &outer);
                "Factory/Inner/Bad/input"
            }
        }
    };
    let mode = if placement >= 2 {
        Mode::Resident
    } else {
        Mode::Finite
    };
    (
        root.build(Policy::FailFast, budget(), mode).unwrap(),
        path.into(),
    )
}

#[test]
fn explicitly_bound_unit_components_work_at_every_boundary() {
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .unwrap();
    let rt = Arc::new(TokioRuntime::current_thread_no_background_drain(
        executor.handle().clone(),
    ));
    for constructor in [false, true] {
        for used in [false, true] {
            for placement in 0..4 {
                let calls = Arc::new(AtomicUsize::new(0));
                let (plan, _) = bound_plan(constructor, used, placement, calls.clone());
                assert!(Machine::new(&plan).is_ok());
                assert!(Machine::with_input(&plan).is_ok());
                let report = executor.block_on(async {
                    let mut running = plan.try_start(rt.clone(), ()).unwrap();
                    if placement >= 2 {
                        running.ready().await.unwrap();
                        running.shutdown();
                    }
                    running.await
                });
                assert_eq!(report.outcome, Outcome::Ok);
                assert_eq!(calls.load(Ordering::SeqCst), 1);
                assert_eq!(rt.tracked(), 0);
            }
        }
    }
}

#[test]
fn supplied_unit_root_and_ordinary_component_remain_valid() {
    let mut child = Plan::builder("child");
    child.step("Work").run(|_, ()| async { Ok(()) });
    let child = child
        .build(Policy::FailFast, budget(), Mode::Finite)
        .unwrap();
    let mut root = Plan::with_input::<()>("root");
    let input = root.input();
    root.step("Input")
        .needs(input)
        .run(|_, _: Arc<()>| async { Ok(()) });
    root.component("Child", &child, ());
    let plan = root
        .build(Policy::FailFast, budget(), Mode::Finite)
        .unwrap();
    assert!(Machine::with_input(&plan).is_ok());
    assert!(Machine::new(&plan).is_ok());
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .unwrap();
    let rt = Arc::new(TokioRuntime::current_thread_no_background_drain(
        executor.handle().clone(),
    ));
    assert_eq!(executor.block_on(plan.start(rt, ())).outcome, Outcome::Ok);
}

#[test]
fn supplied_unit_template_input_remains_valid_at_spawn() {
    let read = Arc::new(AtomicUsize::new(0));
    let observed = read.clone();
    let mut template = Plan::with_input::<()>("unit");
    let input = template.input();
    template
        .step("Read")
        .needs(input)
        .run(move |_, _: Arc<()>| {
            observed.fetch_add(1, Ordering::SeqCst);
            async { Ok(()) }
        });
    let template = template
        .build(Policy::FailFast, budget(), Mode::Finite)
        .unwrap();
    let mut root = Plan::builder("root");
    let unit = root.template("Unit", &template);
    root.service("Owner")
        .spawns(&unit)
        .stop_within(Duration::from_secs(1))
        .initialize(move |cx, ()| async move {
            let child = cx.spawn(&unit, ())?;
            child.ready().await?;
            Ok(())
        })
        .serve(|_cx, _handle| async { Ok(()) });
    let plan = root
        .build(Policy::FailFast, budget(), Mode::Resident)
        .unwrap();
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .unwrap();
    let rt = Arc::new(TokioRuntime::current_thread_no_background_drain(
        executor.handle().clone(),
    ));
    let report = executor.block_on(async {
        let mut running = plan.start(rt, ());
        running.ready().await.unwrap();
        assert_eq!(read.load(Ordering::SeqCst), 1);
        running.shutdown();
        running.await
    });
    assert_eq!(report.outcome, Outcome::Ok);
}

#[test]
fn inspection_and_independent_checker_agree_with_actual_imported_values() {
    let mut root = Plan::with_input::<u32>("root");
    let input = root.input();
    let resource = root
        .resource("Token")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(7u32)) })
        .release(|_, _| async { Ok(()) });
    let mut child = Plan::builder("child");
    let imported = child.import(input);
    let token = child.import(resource);
    let value = child
        .step("Read")
        .needs((imported, token))
        .run(|_, (a, b): (Arc<u32>, Arc<u32>)| async move { Ok(*a + *b) });
    let child = child
        .export(value)
        .build(Policy::FailFast, budget(), Mode::Finite)
        .unwrap();
    let child = root.component("Child", &child, ());
    let plan = root
        .export(child)
        .build(Policy::FailFast, budget(), Mode::Finite)
        .unwrap();
    let view = plan.inspect();
    assert!(sdax_testkit::invariants::check_plan(&view).is_empty());
    let driven = sdax_testkit::ScriptedDriver::run_with_input(&plan, 42, &Script::new()).unwrap();
    driven.check();
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .unwrap();
    let rt = Arc::new(TokioRuntime::current_thread_no_background_drain(
        executor.handle().clone(),
    ));
    let report = executor.block_on(plan.start(rt, 42));
    assert_eq!(report.outcome, Outcome::Ok);
    assert_eq!(report.output.as_deref(), Some(&49));
    assert_eq!(
        view.node("Child/Read").unwrap().needs,
        vec![NodePath::root("Token")]
    );
    assert!(view.release_order().before("Child/Read", "Token"));
}

#[test]
fn explicit_unit_binding_is_independent_of_the_root_input_type() {
    let mut bad = Plan::with_input::<()>("bad");
    bad.step("Unused").run(|_, ()| async { Ok(()) });
    let bad = bad.build(Policy::FailFast, budget(), Mode::Finite).unwrap();
    let mut root = Plan::with_input::<u32>("root");
    root.component("Bad", &bad, ());
    let root = root
        .build(Policy::FailFast, budget(), Mode::Finite)
        .unwrap();
    assert!(Machine::with_input(&root).is_ok());
}
