use sdax::prelude::*;
use sdax_tokio::{PlanStart, TokioRuntime};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

struct Endpoint;
struct Sock;

#[test]
fn a_service_spawns_a_child_that_releases_before_the_import() {
    let parent_released = Arc::new(AtomicBool::new(false));
    let child_released = Arc::new(AtomicBool::new(false));
    let child_flag = child_released.clone();
    let parent_flag = parent_released.clone();
    let saw_child_first = Arc::new(AtomicBool::new(false));
    let order = saw_child_first.clone();

    let mut p = Plan::builder("Mesh");
    let endpoint = p
        .resource("Endpoint")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Endpoint)) })
        .release(move |_cx, _e: Arc<Endpoint>| {
            order.store(child_flag.load(Ordering::SeqCst), Ordering::SeqCst);
            parent_flag.store(true, Ordering::SeqCst);
            async move { Ok(()) }
        });

    let mut t = Plan::with_input::<u8>("Link");
    let imported = t.import(endpoint);
    let cr = child_released.clone();
    t.resource("Sock")
        .needs(imported)
        .acquire(|cx, _e: Arc<Endpoint>| async move { Ok(cx.hold_value(Sock)) })
        .release(move |_cx, _s: Arc<Sock>| {
            cr.store(true, Ordering::SeqCst);
            async move { Ok(()) }
        });
    let child = t
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(2)),
            Mode::Finite,
        )
        .expect("valid template");

    let link = p.template("Link", &child);
    p.service("Accept")
        .needs(endpoint)
        .spawns(&link)
        .stop_within(Duration::from_secs(1))
        .initialize(move |cx, _e: Arc<Endpoint>| async move {
            let ch = cx.spawn(&link, 1u8)?;
            ch.ready().await?;
            Ok(())
        })
        .serve(|_cx, _handle| async { Ok(()) });
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
        running.ready().await.expect("steady");
        running.shutdown();
        running.await
    });
    assert_eq!(report.outcome, Outcome::Ok);
    assert!(report.is_clean());
    assert!(child_released.load(Ordering::SeqCst));
    assert!(parent_released.load(Ordering::SeqCst));
    assert!(
        saw_child_first.load(Ordering::SeqCst),
        "imported key released only after the instance ended"
    );
}
