use sdax::prelude::*;
use sdax_tokio::{PlanStart, TokioRuntime};
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct Destination(&'static str);

#[test]
fn unknown_operation_recovers_by_its_declared_identity() {
    let recovered = Arc::new(Mutex::new(Vec::new()));
    let record = recovered.clone();

    let mut p = Plan::with_input::<u64>("thumbnail publication");
    let operation: Key<u64> = p.input();
    let destination = p
        .step("destination")
        .run(|_cx, ()| async move { Ok(Destination("preview")) });
    p.effect("publish thumbnail")
        .on_ambiguous(Ambiguity::Recover)
        .identified_by(operation)
        .needs(destination)
        .idempotent()
        .within(Duration::from_millis(5))
        .perform(
            |cx, (destination, operation): (Arc<Destination>, Arc<u64>)| async move {
                cx.hold(|| async move {
                    let _request = (destination.0, *operation);
                    std::future::pending::<Result<u64, Error>>().await
                })
                .await
            },
        )
        .recover_unknown(move |_cx, operation: Arc<u64>| {
            let record = record.clone();
            async move {
                record.lock().expect("record").push(*operation);
                Ok(Recovery::Resolved)
            }
        })
        .persistent();
    let plan: Plan<(), u64> = p
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
    let runtime = Arc::new(TokioRuntime::current_thread_no_background_drain(
        tokio_rt.handle().clone(),
    ));
    let report = tokio_rt.block_on(plan.start(runtime, 73));

    assert_eq!(*recovered.lock().expect("record"), [73]);
    assert!(report.ambiguous.is_empty());
    assert!(report.cleanup_failures.is_empty());
}
