use sdax::prelude::*;
use sdax_tokio::{PlanStart, TokioRuntime};
use std::sync::Arc;
use std::time::Duration;

struct Conn;

#[test]
fn a_plan_is_built_once_and_started_with_two_requests() {
    let mut p = Plan::with_input::<u32>("Startup");
    let request = p.input();
    let conn = p
        .resource("Conn")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Conn)) })
        .release(|_cx, _c| async move { Ok(()) });
    let handle = p
        .step("Handle")
        .needs((request, conn))
        .run(|_cx, d: (Arc<u32>, Arc<Conn>)| async move { Ok(*d.0) });
    let plan = p
        .export(handle)
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
    let first = tokio_rt.block_on(async { plan.start(rt.clone(), 123u32).await });
    let second = tokio_rt.block_on(async { plan.start(rt.clone(), 456u32).await });
    assert_eq!(first.output.as_deref(), Some(&123));
    assert_eq!(second.output.as_deref(), Some(&456));
    assert!(first.is_clean() && second.is_clean());
}
