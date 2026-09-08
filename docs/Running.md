# Running

`use sdax_tokio::PlanStart;` then `plan.start(rt, input)` gives a
`#[must_use]` `Running<Out>`. It is a `Future` for the `Report`. A
`Plan::builder` plan takes `input = ()`. A `Plan::with_input::<In>`
plan takes one `In` per run; the driver puts it in that run's slots
before any body is built. Bodies read it by `needs` on `p.input()`.

`start` is **lazy**. Nothing is spawned until the `Running` is first
polled. `cancel()` before that first poll ends the run with no effect.

| Call | What it does |
|---|---|
| `await running` | drive the run to the report |
| `running.ready().await` | wait until the root scope is steady, or the run ended first (`Err(outcome)`) |
| `running.shutdown()` | a normal end from wherever the run is |
| `running.cancel()` | interrupt in-flight bodies; the release graph still runs |
| `running.handle()` | a clonable handle so something else can `shutdown` / `cancel` while you await |
| `running.snapshot()` | where the run was at its last step |

Dropping a live `Running` cancels it and leaves **one tracked drainer**
to finish the release graph inside the shutdown budget. The drainer is
a task, so it needs a driver thread. On a `current_thread` runtime it
makes no progress between `block_on` calls: drop a `Running` inside a
`block_on` that then returns and nothing is released until the next
one. That is why a current-thread handle is built with
`TokioRuntime::current_thread_no_background_drain` — the name is the
acknowledgement. Await the run (and then `TokioRuntime::shutdown`)
inside the `block_on` that started it. A multi-threaded handle goes
through `TokioRuntime::new`.

Two tokio facts a supervisor has to know:

- A blocking body that never returns cannot be aborted. The engine
  abandons the node at the budget and says so, but
  `tokio::runtime::Runtime`'s own `Drop` waits for pool work without a
  budget. `shutdown_timeout` and `shutdown_background` do not.
- Panics are caught only if the profile unwinds. Under `panic = "abort"`
  a body panic aborts the process.

One `Plan` may be `start`ed many times at once. Each run has its own
slots. Put per-request state in the value you pass to `start`, or in
the values nodes return.

A resident plan stays up until you shut it down:

```rust,guide:resident_service
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
```
