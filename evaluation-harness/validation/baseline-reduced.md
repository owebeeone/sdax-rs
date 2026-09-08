## Components

`Plan::with_input::<T>` gives a reusable definition one typed input. Mount it
with `parent.component(name, &child, input)`. Each mount has independent run
state and can bind a different parent key. Define an extra formal import with
`child.port::<T>(name)`, then bind it for a parent with
`child.bind(port, parent_key)` before mounting.

```rust,guide:components
use sdax::prelude::*;
use sdax_tokio::{PlanStart, TokioRuntime};
use std::sync::Arc;
use std::time::Duration;

fn build<O, I>(p: PlanBuilder<O, I>) -> Plan<O, I> {
    p.build(
        Policy::FailFast,
        Shutdown::within(Duration::from_secs(1)),
        Mode::Finite,
    )
    .expect("valid plan")
}

#[test]
fn repeated_component_mounts_bind_independent_inputs() {
    let mut child = Plan::with_input::<u32>("double");
    let input = child.input();
    let doubled = child
        .step("double")
        .needs(input)
        .run(|_, value: Arc<u32>| async move { Ok(*value * 2) });
    let child = build(child.export(doubled));

    let mut parent = Plan::with_input::<u32>("parent");
    let input = parent.input();
    let offset = parent
        .step("offset")
        .needs(input)
        .run(|_, value: Arc<u32>| async move { Ok(*value + 10) });
    let left = parent.component("left", &child, input);
    let right = parent.component("right", &child, offset);
    let answer = parent
        .step("answer")
        .needs((left, right))
        .run(|_, values: (Arc<u32>, Arc<u32>)| async move { Ok((*values.0, *values.1)) });
    let parent = build(parent.export(answer));

    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("runtime");
    let rt = Arc::new(TokioRuntime::current_thread_no_background_drain(
        tokio_rt.handle().clone(),
    ));
    let report = tokio_rt.block_on(parent.start(rt, 3));
    assert_eq!(
        report.into_result().expect("clean run").as_deref(),
        Some(&(6, 26))
    );
}
```

`Plan::builder(name)` is the unit-input spelling. It still has an explicit
`input()` key and mounts receive that key or `()`; do not use ambient globals to
smuggle a component input across a mount boundary.

## Services

A service has two phases. Initialization establishes readiness and publishes a
stable `Arc<H>` once. Serving owns one episode using that same handle. Put child
spawning and `child.ready().await` work in initialization; put the former
long-running service future in `serve`.

```rust
struct ServiceState;

p.service("accept")
    .idempotent()
    .restart(Restart::on_error(Backoff::fixed(Duration::from_secs(1))).max(1))
    .initialize(|_cx: Cx<Start>, ()| async { Ok(ServiceState) })
    .serve(|cx: Cx<ServingPhase>, _state: Arc<ServiceState>| async move {
        if cx.episode() == 1 {
            Err::<(), Error>("connection lost".into())
        } else {
            cx.stop().await;
            Ok(())
        }
    });
```

`Retry` controls initialization attempts. `Restart` controls serving episodes
after the first one. A serving restart never runs a successful initializer
again, changes the published handle, or reruns dependents. Readiness stays
latched after initialization, so it does not describe current health during
recovery. Use `cx.episode()` when a serving body needs its one-based episode
number.

The example's handle is in-memory state. Service-local external acquisitions do
not gain a resource cleanup obligation. Declare such values as resource
dependencies, or keep them in the serving future so Rust drops them when the
episode ends. Use `cx.spawn` for declared dynamic child templates; do not launch
untracked tasks.
