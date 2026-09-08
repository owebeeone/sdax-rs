use sdax::prelude::*;
use std::sync::Arc;

#[test]
fn a_finite_plan_cannot_export_a_released_resource_directly() {
    let mut p = Plan::builder("Lease");
    let lease = p
        .resource("Connection")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(7u32)) })
        .release(|_, _| async { Ok(()) });
    let built = p.export(lease).build(
        Policy::FailFast,
        Shutdown::within(Duration::from_secs(1)),
        Mode::Finite,
    );
    let invalid = match built {
        Err(e) => e,
        Ok(_) => panic!("a released resource is not a completed result"),
    };
    assert!(invalid
        .checks
        .iter()
        .any(|c| c.rule.id() == "V-LIVE-EXPORT"));
}

#[test]
fn a_finite_plan_can_export_data_computed_from_a_resource() {
    let mut p = Plan::builder("Result");
    let lease = p
        .resource("Connection")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(7u32)) })
        .release(|_, _| async { Ok(()) });
    let result = p
        .step("Result")
        .needs(lease)
        .run(|_, lease: Arc<u32>| async move { Ok(*lease + 1) });
    assert!(p
        .export(result)
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(1)),
            Mode::Finite
        )
        .is_ok());
}

#[test]
fn a_finite_parent_cannot_forward_a_resident_components_live_output() {
    let mut child = Plan::builder("Child");
    let lease = child
        .resource("Lease")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(7u32)) })
        .release(|_, _| async { Ok(()) });
    let child = child
        .export(lease)
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(1)),
            Mode::Resident,
        )
        .unwrap();
    let mut parent = Plan::builder("Parent");
    let mounted = parent.component("Mounted", &child, ());
    let invalid = parent
        .export(mounted)
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(2)),
            Mode::Finite,
        )
        .expect_err("forwarding a live handle must retain its lifetime");
    assert!(invalid
        .checks
        .iter()
        .any(|c| c.rule.id() == "V-LIVE-EXPORT"));
}

#[test]
fn a_finite_parent_cannot_forward_a_resource_through_a_child_input() {
    let mut child = Plan::with_input::<u32>("Forwarder");
    let input = child.input();
    child
        .step("Observe")
        .needs(input)
        .run(|_, _: Arc<u32>| async { Ok(()) });
    let child = child
        .export(input)
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(1)),
            Mode::Finite,
        )
        .unwrap();
    let mut parent = Plan::builder("Parent");
    let lease = parent
        .resource("Lease")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(7u32)) })
        .release(|_, _| async { Ok(()) });
    let mounted = parent.component("Forwarder", &child, lease);
    let invalid = parent
        .export(mounted)
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(2)),
            Mode::Finite,
        )
        .expect_err("input binding cannot hide a live resource export");
    assert!(invalid
        .checks
        .iter()
        .any(|c| c.rule.id() == "V-LIVE-EXPORT"));
}
