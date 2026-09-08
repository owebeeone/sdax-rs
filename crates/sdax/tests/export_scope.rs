use sdax::prelude::*;

#[test]
fn exporting_a_foreign_key_is_rejected_before_execution() {
    let mut first = Plan::builder("First");
    let foreign = first.step("Original").run(|_, ()| async { Ok(111u64) });
    let mut second = Plan::builder("Second");
    second.step("Different").run(|_, ()| async { Ok(222u64) });
    let invalid = second
        .export(foreign)
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(1)),
            Mode::Finite,
        )
        .expect_err("equal declaration indices do not make foreign keys interchangeable");
    let finding = invalid
        .checks
        .iter()
        .find(|f| f.rule.id() == "V-FOREIGN-KEY")
        .expect("structured export scope finding");
    assert_eq!(finding.keys, [foreign.raw()]);
    assert!(finding.detail.contains("export"));
}
