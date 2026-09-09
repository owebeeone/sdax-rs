use sdax::prelude::*;

#[test]
fn replacing_dependencies_retains_identity_and_drops_old_dependencies() {
    let mut p = Plan::with_input::<u64>("identity");
    let input = p.input();
    let id = p
        .step("identity")
        .needs(input)
        .run(|_, id| async move { Ok(*id) });
    let old = p.step("old").run(|_, ()| async { Ok(1u32) });
    p.effect("operation")
        .on_ambiguous(Ambiguity::Report)
        .needs(old)
        .identified_by(id)
        .needs(())
        .perform(|cx, ((), id)| async move { cx.hold(|| async move { Ok::<_, Error>(*id) }).await })
        .recover_unknown(|_, _| async { Ok(Recovery::StillUnknown) })
        .persistent();
    let p = p
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(1)),
            Mode::Finite,
        )
        .unwrap();
    let view = p.inspect();
    let operation = view
        .nodes
        .iter()
        .find(|n| n.path.to_string().ends_with("operation"))
        .unwrap();
    assert_eq!(operation.needs.len(), 1);
    assert!(!operation.needs[0].to_string().ends_with("old"));
}

#[test]
fn forwarding_does_not_bypass_retry_safety_or_completion_validation() {
    let mut p = Plan::with_input::<u64>("unsafe");
    let input = p.input();
    let id = p
        .step("identity")
        .needs(input)
        .run(|_, id| async move { Ok(*id) });
    p.effect("operation")
        .on_ambiguous(Ambiguity::Report)
        .identified_by(id)
        .retry(Retry::attempts(2))
        .perform(|cx, ((), id)| async move { cx.hold(|| async move { Ok::<_, Error>(*id) }).await })
        .recover_unknown(|_, _| async { Ok(Recovery::StillUnknown) })
        .persistent();
    assert!(p
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(1)),
            Mode::Finite
        )
        .is_err());
    let mut p = Plan::with_input::<u64>("unfinished");
    let input = p.input();
    let id = p
        .step("identity")
        .needs(input)
        .run(|_, id| async move { Ok(*id) });
    drop(
        p.effect("operation")
            .on_ambiguous(Ambiguity::Report)
            .identified_by(id)
            .needs(()),
    );
    assert!(p
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(1)),
            Mode::Finite
        )
        .is_err());
}
