use sdax::prelude::*;
use sdax_tokio::{PlanStart, TokioRuntime};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug)]
struct Disconnected;

impl std::fmt::Display for Disconnected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("disconnected")
    }
}

impl std::error::Error for Disconnected {}

#[test]
fn a_resident_service_recovers_then_stops() {
    let initializations = Arc::new(AtomicUsize::new(0));
    let live_episode = Arc::new(AtomicUsize::new(0));
    let mut p = Plan::builder("Listen");
    p.service("Accept")
        .idempotent()
        .restart(Restart::on_error(Backoff::fixed(Duration::from_secs(1))).max(1))
        .stop_within(Duration::from_secs(1))
        .initialize({
            let initializations = initializations.clone();
            move |_cx, ()| {
                initializations.fetch_add(1, Ordering::SeqCst);
                async { Ok(()) }
            }
        })
        .serve({
            let live_episode = live_episode.clone();
            move |cx, _handle: Arc<()>| {
                live_episode.store(cx.episode() as usize, Ordering::SeqCst);
                async move {
                    if cx.episode() == 1 {
                        Err(Box::new(Disconnected) as Error)
                    } else {
                        cx.stop().await;
                        Ok(())
                    }
                }
            }
        });
    let plan = p
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(10)),
            Mode::Resident,
        )
        .expect("valid");

    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .expect("runtime");
    let rt = Arc::new(TokioRuntime::current_thread_no_background_drain(
        tokio_rt.handle().clone(),
    ));
    let report = tokio_rt.block_on(async {
        let mut running = plan.start(rt.clone(), ());
        running.ready().await.expect("initial readiness");
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert_eq!(initializations.load(Ordering::SeqCst), 1);
        assert_eq!(live_episode.load(Ordering::SeqCst), 2);
        running.shutdown();
        running.await
    });
    assert_eq!(report.outcome, Outcome::Ok);
    assert!(report.is_clean());
}
