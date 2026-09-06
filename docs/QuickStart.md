# Quick Start

Add the crates, write a plan, start it on tokio, read the report.

```sh
cargo add sdax sdax-tokio
```

`sdax` is std-only. `sdax-tokio` is the adapter **and** the run driver:
`use sdax_tokio::PlanStart;` then `plan.start(rt, input)`. A plan built
with `Plan::builder` takes `()`. A plan built with `Plan::with_input`
takes one typed value per run.

The program below is a test. Build a tokio runtime by hand — this crate
has no `macros` feature, so there is no `#[tokio::test]`. A resource
`hold`s a value; a step needs that value and the run's input; `build`
takes the fail policy, the shutdown budget, and the run mode as
arguments, not defaults. No service, so `Mode::Finite` is allowed.

The `Plan` is the reusable value. You build it once; each `start` is a
run with its own slots and its own input. There is no shared context
object and no Python `process_tasks(ctx)`. Per-run data is the value
you pass to `start`, plus what the bodies return. The test starts the
same plan twice with two request ids and exports a different value
each time.

```rust,guide:simple_hold
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
```

`hold_value` is for a value you already own. If the acquire *is* an
external effect (open a socket, take a lock), put that effect inside
`cx.hold(...)` so the engine records the obligation in the same poll
that sees the effect complete. [Cleanup](Cleanup.md) is the page that
exists so you do not reopen that window.

A fault still runs the release graph — [Errors](Errors.md) quotes that
test. A plan that stays up is `Mode::Resident`; see [Running](Running.md).
