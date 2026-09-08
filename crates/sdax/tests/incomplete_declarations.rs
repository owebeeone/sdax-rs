use sdax::prelude::*;
use std::time::Duration;

#[test]
fn abandoned_resource_and_effect_are_build_findings() {
    let mut p = Plan::builder("Incomplete");
    let _ = p
        .resource("Lease")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(1u8)) });
    let unfinished = p
        .effect("Reservation")
        .on_ambiguous(Ambiguity::Report)
        .perform(|cx, ()| async move { Ok(cx.hold_value(2u8)) });
    std::mem::forget(unfinished);
    p.step("Unrelated").run(|_, ()| async { Ok(()) });
    let invalid = match p.build(
        Policy::FailFast,
        Shutdown::within(Duration::from_secs(1)),
        Mode::Finite,
    ) {
        Err(e) => e,
        Ok(_) => panic!("unfinished declarations must not disappear"),
    };
    let incomplete: Vec<_> = invalid
        .checks
        .iter()
        .filter(|f| f.rule.id() == "V-INCOMPLETE-DECLARATION")
        .collect();
    assert_eq!(incomplete.len(), 2);
    assert_eq!(incomplete[0].nodes, ["Lease"]);
    assert!(incomplete[0].fix.contains("release"));
    assert_eq!(incomplete[1].nodes, ["Reservation"]);
    assert!(incomplete[1].fix.contains("compensate"));
}

#[test]
fn complete_chains_keep_contiguous_keys() {
    let mut p = Plan::builder("Complete");
    let lease = p
        .resource("Lease")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(1u8)) })
        .release(|_, _| async { Ok(()) });
    p.step("Use").needs(lease).run(|_, _| async { Ok(()) });
    assert!(p
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(1)),
            Mode::Finite
        )
        .is_ok());
}

#[test]
fn abandoned_initial_chain_is_also_reported() {
    let mut p = Plan::builder("Dropped");
    let _ = p.resource("NeverAcquired");
    p.step("Kept").run(|_, ()| async { Ok(()) });
    assert!(p
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(1)),
            Mode::Finite
        )
        .is_err());
}

#[test]
fn abandoned_service_initializer_requires_the_serving_terminal() {
    let mut p = Plan::builder("ServiceDeclaration");
    let initialized = p.service("Worker").initialize(|_, ()| async { Ok(()) });
    std::mem::forget(initialized);
    p.step("Other").run(|_, ()| async { Ok(()) });
    let invalid = p
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(1)),
            Mode::Resident,
        )
        .expect_err("the service cannot silently vanish");
    let finding = invalid
        .checks
        .iter()
        .find(|f| f.rule.id() == "V-INCOMPLETE-DECLARATION")
        .unwrap();
    assert_eq!(finding.nodes, ["Worker"]);
    assert!(finding.fix.contains("serve"), "{}", finding.fix);
}
