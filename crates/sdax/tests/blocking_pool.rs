use sdax::prelude::*;

#[test]
fn a_blocking_pool_cannot_be_overridden_by_a_general_limit() {
    let mut p = Plan::builder("Pools");
    let broad = p.pool("Broad", 2);
    let serial = p.pool("Serial", 1);
    p.blocking_step("Work")
        .limit(broad)
        .on(serial)
        .run(|_, ()| Ok(()));
    let invalid = p
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(1)),
            Mode::Finite,
        )
        .expect_err("two declared pools must not silently choose one");
    let finding = invalid
        .checks
        .iter()
        .find(|f| f.rule.id() == "V-BLOCKING-LIMIT")
        .expect("structured conflicting pool finding");
    assert_eq!(finding.nodes, ["Work"]);
    assert!(finding.fix.contains("limit"));
}
