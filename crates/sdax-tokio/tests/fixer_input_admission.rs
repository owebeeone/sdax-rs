//! Every component input needs a supplier; declaring unit is not supplying it.
use sdax::host::engine::{EngineError, Machine};
use sdax::host::sim::ScriptError;
use sdax::*;
use sdax_tokio::{PlanStart, TokioRuntime};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn budget() -> Shutdown {
    Shutdown::within(Duration::from_secs(5))
}

fn invalid_plan(
    template_constructor: bool,
    used: bool,
    placement: u8,
    calls: Arc<AtomicUsize>,
) -> (Plan<()>, String) {
    let mut bad = if template_constructor {
        Plan::template::<()>("bad")
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
            root.component("Bad", &bad);
            "Bad/input"
        }
        1 => {
            let mut middle = Plan::builder("middle");
            middle.component("Bad", &bad);
            let middle = middle
                .build(Policy::FailFast, budget(), Mode::Finite)
                .unwrap();
            root.component("Middle", &middle);
            "Middle/Bad/input"
        }
        _ => {
            let mut template = Plan::template::<u32>("template");
            let input = template.input();
            template
                .step("OwnInput")
                .needs(input)
                .run(|_, _: Arc<u32>| async { Ok(()) });
            template.component("Bad", &bad);
            let template = template
                .build(Policy::FailFast, budget(), Mode::Finite)
                .unwrap();
            if placement == 2 {
                root.template("Factory", &template);
                "Factory/Bad/input"
            } else {
                let mut outer = Plan::template::<u16>("outer");
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
fn invalid_component_inputs_refuse_before_root_effects_in_every_entry_point() {
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
                let (plan, path) = invalid_plan(constructor, used, placement, calls.clone());
                let expected = EngineError::TemplateAsScope(
                    path.split('/')
                        .fold(NodePath::default(), |prefix, segment| prefix.child(segment)),
                );
                assert_eq!(Machine::new(&plan).unwrap_err(), expected);
                assert_eq!(Machine::with_input(&plan).unwrap_err(), expected);
                assert!(matches!(plan.try_start(rt.clone(), ()), Err(e) if e == expected));
                for simulation in [
                    plan.simulate(&Script::new()),
                    plan.simulate_with_input((), &Script::new()),
                ] {
                    assert!(matches!(simulation, Err(ScriptError::Engine(e)) if e == expected));
                }
                let running = plan.start(rt.clone(), ());
                let report = executor.block_on(running);
                assert_eq!(report.outcome, Outcome::Failed);
                assert_eq!(report.faults.len(), 1);
                assert!(
                    matches!(&report.faults[0].kind, FaultKind::Error(e) if e.downcast_ref::<EngineError>() == Some(&expected))
                );
                assert_eq!(calls.load(Ordering::SeqCst), 0);
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
    root.component("Child", &child);
    let plan = root
        .build(Policy::FailFast, budget(), Mode::Finite)
        .unwrap();
    assert!(Machine::with_input(&plan).is_ok());
    assert!(matches!(
        Machine::new(&plan),
        Err(EngineError::TemplateAsScope(_))
    ));
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
    let mut template = Plan::template::<()>("unit");
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
        .start(move |cx, ()| async move {
            let child = cx.spawn(&unit, ())?;
            child.ready().await?;
            Ok(Serving::new((), async { Ok(()) }))
        });
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
    let child = root.component("Child", &child);
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
fn supplied_root_input_does_not_supply_a_components_own_unit_input() {
    let mut bad = Plan::with_input::<()>("bad");
    bad.step("Unused").run(|_, ()| async { Ok(()) });
    let bad = bad.build(Policy::FailFast, budget(), Mode::Finite).unwrap();
    let mut root = Plan::with_input::<u32>("root");
    root.component("Bad", &bad);
    let root = root
        .build(Policy::FailFast, budget(), Mode::Finite)
        .unwrap();
    assert_eq!(
        Machine::with_input(&root).unwrap_err(),
        EngineError::TemplateAsScope(NodePath::root("Bad").child("input"))
    );
}
