# Cleanup

A resource or effect body must return `Held<T>`. Only `cx.hold` and
`cx.hold_value` can mint one, and both register the value with the
engine *before* they hand it back. That is the whole contract: there is
no await point between "the effect happened" and "the engine owns the
cleanup".

- **`cx.hold(effect)`** — run an external effect under the engine's
  wrapper. The poll that sees the effect complete writes the record and
  *then* returns `Ready`. Use this to open a socket, take a lock, write
  a row you will compensate.
- **`cx.hold_value(v)`** — register a value you already own. Registration
  happens before this returns. Doing the effect yourself, awaiting
  something else, and then calling `hold_value` reopens the window by
  hand. The engine cannot see that.

A service's own acquisitions belong in a resource node, not in its
`start` body. A value a start body creates has no ledger entry and no
async release.

The release graph is the reverse of `needs`. Dropping a live `Running`
still cancels the run and leaves one tracked task to finish that graph
inside the shutdown budget. Compensation of an effect is a distinct
record from a resource release. A `persistent` effect is never undone.

A fault does not skip cleanup. The test below is a resource, a step
that fails, and a flag that the release body sets — the report is
`Failed`, the single fault names `Boom`, and the flag is set.

```rust,guide:fail_and_release
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
```
