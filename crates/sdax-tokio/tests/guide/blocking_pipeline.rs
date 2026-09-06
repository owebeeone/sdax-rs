use sdax::prelude::*;
use sdax_tokio::{PlanStart, TokioRuntime};
use std::sync::Arc;
use std::time::Duration;

struct Raw;
struct Parsed(u32);

#[test]
fn a_blocking_step_runs_on_a_declared_pool() {
    let mut p = Plan::builder("Pipeline");
    let raw = p
        .resource("Raw")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Raw)) })
        .release(|_cx, _r: Arc<Raw>| async move { Ok(()) });
    let pool = p.pool("cpu", 2);
    let parsed = p
        .blocking_step("Parse")
        .needs(raw)
        .on(pool)
        .run(|_cx, _r: Arc<Raw>| Ok(Parsed(7)));
    let report = p
        .step("Report")
        .needs(parsed)
        .run(|_cx, v: Arc<Parsed>| async move { Ok(v.0 * 6) });
    let plan = p
        .export(report)
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(10)),
            Mode::Finite,
        )
        .expect("valid");

    // Real time, not `start_paused`: a pool thread cannot advance a paused
    // clock, and the auto-advance would fire the shutdown budget while the
    // blocking body worked.
    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("runtime");
    let rt = Arc::new(TokioRuntime::current_thread_no_background_drain(
        tokio_rt.handle().clone(),
    ));
    let report = tokio_rt.block_on(async { plan.start(rt.clone(), ()).await });
    assert_eq!(report.output.as_deref(), Some(&42));
    assert!(report.is_clean());
}
