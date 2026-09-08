use sdax::prelude::*;
use sdax_tokio::{PlanStart, TokioRuntime};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[test]
fn step_factory_panic_is_reported_and_upstream_resource_released() {
    for try_step in [false, true] {
        let released = Arc::new(AtomicBool::new(false));
        let flag = released.clone();
        let mut p = Plan::builder("FactoryPanic");
        let lease = p
            .resource("Lease")
            .acquire(|cx, ()| async move { Ok(cx.hold_value(())) })
            .release(move |_, _| {
                flag.store(true, Ordering::SeqCst);
                async { Ok(()) }
            });
        fn factory(_: Cx<Run>, _: Arc<()>) -> std::future::Ready<Result<(), Error>> {
            panic!("factory panicked before returning its future");
        }
        if try_step {
            let result = p.try_step("Work").needs(lease).run(factory);
            p.step("Consume").needs(result).run(|_, _| async { Ok(()) });
        } else {
            p.step("Work").needs(lease).run(factory);
        }
        let plan = p
            .build(
                Policy::FailFast,
                Shutdown::within(Duration::from_secs(2)),
                Mode::Finite,
            )
            .unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .start_paused(true)
            .build()
            .unwrap();
        let host = Arc::new(TokioRuntime::current_thread_no_background_drain(
            runtime.handle().clone(),
        ));
        runtime.block_on(async {
            let report = tokio::time::timeout(Duration::from_secs(3), plan.start(host.clone(), ()))
                .await
                .expect("run settles");
            assert_eq!(report.outcome, Outcome::Failed);
            assert_eq!(report.panics().len(), 1);
            assert!(
                released.load(Ordering::SeqCst),
                "factory panic must not skip cleanup"
            );
            host.shutdown(Duration::from_secs(1)).await.unwrap();
        });
    }
}
