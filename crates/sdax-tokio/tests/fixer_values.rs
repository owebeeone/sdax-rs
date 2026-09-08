//! Real-value regressions for structural readiness.
use sdax::*;
use sdax_tokio::{PlanStart, TokioRuntime};
use std::sync::Arc;
use std::time::Duration;

fn shutdown() -> Shutdown {
    Shutdown::within(Duration::from_secs(10))
}
fn run<O: Send + Sync + 'static, I: Send + Sync + 'static>(
    plan: &Plan<O, I>,
    input: I,
) -> Report<O> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .unwrap();
    let adapter = Arc::new(TokioRuntime::current_thread_no_background_drain(
        rt.handle().clone(),
    ));
    rt.block_on(plan.start(adapter, input))
}
fn ninety_nine() -> Plan<u32> {
    let mut p = Plan::builder("child");
    let value = p.step("value").run(|_, ()| async { Ok(99u32) });
    p.export(value)
        .build(Policy::FailFast, shutdown(), Mode::Finite)
        .unwrap()
}
#[test]
fn join_and_chained_join_supply_actual_unit() {
    let mut p = Plan::builder("joins");
    let first = p.join("first", ());
    let second = p.join("second", first);
    let value = p
        .step("consumer")
        .needs((first, second))
        .run(|_, _: (Arc<()>, Arc<()>)| async { Ok(7u32) });
    let report = run(
        &p.export(value)
            .build(Policy::FailFast, shutdown(), Mode::Finite)
            .unwrap(),
        (),
    );
    assert_eq!(report.outcome, Outcome::Ok);
    assert_eq!(report.output.as_deref(), Some(&7));
}
#[test]
fn component_exported_by_root_is_actual_99() {
    let mut p = Plan::builder("root");
    let component = p.component("child", &ninety_nine(), ());
    let report = run(
        &p.export(component)
            .build(Policy::FailFast, shutdown(), Mode::Finite)
            .unwrap(),
        (),
    );
    assert_eq!(report.outcome, Outcome::Ok);
    assert_eq!(report.output.as_deref(), Some(&99));
}
#[test]
fn parent_consumes_actual_component_value() {
    let mut p = Plan::builder("root");
    let component = p.component("child", &ninety_nine(), ());
    let value = p
        .step("consumer")
        .needs(component)
        .run(|_, v: Arc<u32>| async move { Ok(*v + 1) });
    let report = run(
        &p.export(value)
            .build(Policy::FailFast, shutdown(), Mode::Finite)
            .unwrap(),
        (),
    );
    assert_eq!(report.outcome, Outcome::Ok);
    assert_eq!(report.output.as_deref(), Some(&100));
}
fn input_component_plan() -> Plan<u32, u32> {
    let mut root = Plan::with_input::<u32>("root");
    let input = root.input();
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
    let component = root.component("child", &child, ());
    root.export(component)
        .build(Policy::FailFast, shutdown(), Mode::Finite)
        .unwrap()
}
#[test]
fn imported_root_input_crosses_component_boundary() {
    let report = run(&input_component_plan(), 42);
    assert_eq!(report.outcome, Outcome::Ok);
    assert_eq!(report.output.as_deref(), Some(&42));
}
#[test]
fn no_export_and_explicit_unit_export_both_supply_unit() {
    for explicit in [false, true] {
        let mut child = Plan::builder("child");
        let unit = child.step("unit").run(|_, ()| async { Ok(()) });
        let child = if explicit {
            child
                .export(unit)
                .build(Policy::FailFast, shutdown(), Mode::Finite)
        } else {
            child.build(Policy::FailFast, shutdown(), Mode::Finite)
        }
        .unwrap();
        let mut root = Plan::builder("root");
        let component = root.component("child", &child, ());
        let value = root
            .step("consumer")
            .needs(component)
            .run(|_, _: Arc<()>| async { Ok(7u32) });
        let report = run(
            &root
                .export(value)
                .build(Policy::FailFast, shutdown(), Mode::Finite)
                .unwrap(),
            (),
        );
        assert_eq!(report.outcome, Outcome::Ok, "explicit={explicit}");
        assert_eq!(report.output.as_deref(), Some(&7));
    }
}
#[test]
fn nested_components_publish_through_every_boundary() {
    let mut middle = Plan::builder("middle");
    let inner = middle.component("inner", &ninety_nine(), ());
    let middle = middle
        .export(inner)
        .build(Policy::FailFast, shutdown(), Mode::Finite)
        .unwrap();
    let mut root = Plan::builder("root");
    let outer = root.component("outer", &middle, ());
    let report = run(
        &root
            .export(outer)
            .build(Policy::FailFast, shutdown(), Mode::Finite)
            .unwrap(),
        (),
    );
    assert_eq!(report.outcome, Outcome::Ok);
    assert_eq!(report.output.as_deref(), Some(&99));
}
#[test]
fn concurrent_runs_do_not_share_component_slots() {
    let plan = input_component_plan();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .unwrap();
    let adapter = Arc::new(TokioRuntime::current_thread_no_background_drain(
        rt.handle().clone(),
    ));
    let (a, b) = rt.block_on(async {
        let a = plan.start(adapter.clone(), 11);
        let b = plan.start(adapter, 22);
        (a.await, b.await)
    });
    assert_eq!(a.output.as_deref(), Some(&11));
    assert_eq!(b.output.as_deref(), Some(&22));
    assert!(!Arc::ptr_eq(
        a.output.as_ref().unwrap(),
        b.output.as_ref().unwrap()
    ));
}
