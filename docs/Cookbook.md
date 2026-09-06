# Cookbook

Three shapes. The first two are tests already on other pages; the third
is a finite pipeline with a blocking step.

## A request-scoped finite plan

One `Plan`, many `start`s. Write it with `Plan::with_input::<Req>`,
`needs` the key `p.input()` returns, and start each request
`plan.start(rt, req)`. Each run has its own slots; the input is that
run's typed state, not a mutex shared across starts. `Mode::Finite`.
The [Quick Start](QuickStart.md) test is this shape: input, acquire,
work, release, report.

## A resident service graph

`Mode::Resident`, at least one service, `ready()` then `shutdown()`.
The [Running](Running.md) test is this shape. A mesh that accepts
connections is [Instances](Instances.md): the accept loop is the
service; each connection is a template instance.

## A pipeline with a blocking step

Synchronous work goes on a declared pool. `run` is not `async`. A
thread cannot be aborted — do not put `cooperative` on a blocking
step (`V-BLOCKING-CANCEL`). A body that never returns blocks
`Runtime::drop`; see [Running](Running.md). The test below parses on
the pool, then an async step exports `42`. It uses real time, not
`start_paused`: a pool thread cannot advance a paused clock.

```rust,guide:blocking_pipeline
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
```

## When not to use sdax

In-process lifecycle: startup, request wiring, a resident service
graph, a short pipeline with a derived teardown. Not a distributed
job queue, not an unbounded CPU farm, and not anything that wants
waves or levels — those are a different product.
