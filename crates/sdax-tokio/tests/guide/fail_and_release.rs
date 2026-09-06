use sdax::prelude::*;
use sdax_tokio::{PlanStart, TokioRuntime};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

struct Conn;

#[test]
fn a_fault_still_runs_the_release_graph() {
    let released = Arc::new(AtomicBool::new(false));
    let flag = released.clone();

    let mut p = Plan::builder("Fault");
    let conn = p
        .resource("Conn")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Conn)) })
        .release(move |_cx, _c: Arc<Conn>| {
            flag.store(true, Ordering::SeqCst);
            async move { Ok(()) }
        });
    p.step("Boom")
        .needs(conn)
        .run(|_cx, _c: Arc<Conn>| async move { Err::<(), _>("boom".into()) });
    let plan = p
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(10)),
            Mode::Finite,
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
    let report = tokio_rt.block_on(async { plan.start(rt.clone(), ()).await });
    assert_eq!(report.outcome, Outcome::Failed);
    assert_eq!(report.faults.len(), 1);
    assert_eq!(report.faults[0].node.leaf(), "Boom");
    assert!(released.load(Ordering::SeqCst), "release still ran");
}
