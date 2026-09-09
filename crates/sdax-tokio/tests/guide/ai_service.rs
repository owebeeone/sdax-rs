use sdax::prelude::*;
use sdax_tokio::{PlanStart, TokioRuntime};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct WatchHandle(u64);

#[test]
fn resident_service_initializes_restarts_and_stops() {
    let initializations = Arc::new(AtomicUsize::new(0));
    let episodes = Arc::new(AtomicUsize::new(0));
    let handles = Arc::new(Mutex::new(Vec::new()));

    let mut p = Plan::builder("index monitor");
    p.service("watch index")
        .idempotent()
        .restart(Restart::on_error(Backoff::fixed(Duration::from_secs(1))).max(1))
        .stop_within(Duration::from_secs(1))
        .initialize({
            let initializations = initializations.clone();
            move |_cx, ()| {
                initializations.fetch_add(1, Ordering::SeqCst);
                async { Ok(WatchHandle(91)) }
            }
        })
        .serve({
            let episodes = episodes.clone();
            let handles = handles.clone();
            move |cx, handle: Arc<WatchHandle>| {
                episodes.store(cx.episode() as usize, Ordering::SeqCst);
                handles
                    .lock()
                    .expect("handles")
                    .push((Arc::as_ptr(&handle) as usize, handle.0));
                async move {
                    if cx.episode() == 1 {
                        Err::<(), Error>("index changed".into())
                    } else {
                        cx.stop().await;
                        Ok(())
                    }
                }
            }
        });
    let plan: Plan = p
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(5)),
            Mode::Resident,
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
    let report = tokio_rt.block_on(async {
        let mut running = plan.start(runtime, ());
        running.ready().await.expect("ready");
        tokio::time::sleep(Duration::from_secs(2)).await;
        running.shutdown();
        running.await
    });

    assert_eq!(initializations.load(Ordering::SeqCst), 1);
    assert_eq!(episodes.load(Ordering::SeqCst), 2);
    let handles = handles.lock().expect("handles");
    assert_eq!(handles.len(), 2);
    assert_eq!(handles[0], handles[1]);
    assert!(report.is_clean());
}
