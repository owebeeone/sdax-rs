use sdax::*;
use sdax_tokio::{PlanStart, TokioRuntime};
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn run(plan: Plan) -> Report<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .unwrap();
    let adapter = Arc::new(TokioRuntime::current_thread_no_background_drain(
        rt.handle().clone(),
    ));
    let report = rt.block_on(plan.start(adapter.clone(), ()));
    assert_eq!(
        adapter.tracked(),
        0,
        "factory panics must not escape the driver"
    );
    report
}

#[test]
fn panic_in_acquisition_factory_after_hold_keeps_obligation() {
    for effect in [false, true] {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let released = seen.clone();
        let mut p = Plan::builder("FactoryPanic");
        let prepare = |cx: Cx<Acquire>, ()| -> std::future::Ready<Result<Held<u8>, Error>> {
            let _held = cx.hold_value(7u8);
            panic!("prepare factory panic after registration")
        };
        let cleanup = move |_, receipt: Arc<u8>| {
            let released = released.clone();
            async move {
                released.lock().unwrap().push(*receipt);
                Ok(())
            }
        };
        if effect {
            p.effect("N")
                .on_ambiguous(Ambiguity::Report)
                .perform(prepare)
                .compensate(cleanup);
        } else {
            p.resource("N").acquire(prepare).release(cleanup);
        }
        let report = run(p
            .build(
                Policy::FailFast,
                Shutdown::within(Duration::from_secs(1)),
                Mode::Finite,
            )
            .unwrap());
        assert_eq!(*seen.lock().unwrap(), [7]);
        assert_eq!(report.faults[0].kind.label(), FaultLabel::Panic);
        assert!(report.cleanup_failures.is_empty());
    }
}

#[test]
fn cleanup_factory_panic_is_reported_and_upstream_cleanup_runs() {
    for effect in [false, true] {
        let released = Arc::new(Mutex::new(false));
        let flag = released.clone();
        let mut p = Plan::builder("CleanupFactoryPanic");
        let upstream = p
            .resource("Upstream")
            .acquire(|cx, ()| async { Ok(cx.hold_value(1u8)) })
            .release(move |_, _| {
                let flag = flag.clone();
                async move {
                    *flag.lock().unwrap() = true;
                    Ok(())
                }
            });
        let prepare = |cx: Cx<Acquire>, _: Arc<u8>| async { Ok(cx.hold_value(2u8)) };
        let cleanup = |_, _: Arc<u8>| -> std::future::Ready<Result<(), Error>> {
            panic!("cleanup factory panic")
        };
        if effect {
            p.effect("N")
                .needs(upstream)
                .on_ambiguous(Ambiguity::Report)
                .perform(prepare)
                .compensate(cleanup);
        } else {
            p.resource("N")
                .needs(upstream)
                .acquire(prepare)
                .release(cleanup);
        }
        let report = run(p
            .build(
                Policy::FailFast,
                Shutdown::within(Duration::from_secs(1)),
                Mode::Finite,
            )
            .unwrap());
        assert!(*released.lock().unwrap());
        assert_eq!(report.cleanup_failures[0].kind.label(), FaultLabel::Panic);
    }
}
