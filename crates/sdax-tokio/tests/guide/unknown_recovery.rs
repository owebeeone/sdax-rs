use sdax::prelude::*;
use sdax_tokio::{PlanStart, TokioRuntime};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[test]
fn recovery_uses_the_recorded_identity_after_an_unknown_outcome() {
    let recovered = Arc::new(Mutex::new(Vec::new()));
    let seen = recovered.clone();
    let mut p = Plan::with_input::<u64>("reserve");
    let operation = p.input();
    p.effect("remote reservation")
        .idempotent()
        .within(Duration::from_millis(5))
        .on_ambiguous(Ambiguity::Recover)
        .identified_by(operation)
        .perform(|cx, ((), operation)| async move {
            cx.hold(|| async move {
                let _ = operation;
                std::future::pending::<Result<u64, Error>>().await
            })
            .await
        })
        .recover_unknown(move |_cx, operation| {
            let seen = seen.clone();
            async move {
                seen.lock().expect("record").push(*operation);
                Ok(Recovery::Resolved)
            }
        })
        .persistent();
    let plan = p
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_millis(20)),
            Mode::Finite,
        )
        .expect("valid plan");

    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .expect("runtime");
    let rt = Arc::new(TokioRuntime::current_thread_no_background_drain(
        tokio_rt.handle().clone(),
    ));
    let report = tokio_rt.block_on(plan.start(rt, 42));

    assert_eq!(*recovered.lock().expect("record"), [42]);
    assert!(
        report.ambiguous.is_empty(),
        "recovery resolved the uncertainty"
    );
    assert!(
        report.cleanup_failures.is_empty(),
        "recovery completed cleanly"
    );
}
