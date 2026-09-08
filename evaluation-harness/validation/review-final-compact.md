# AI authoring reference

Use one `Plan` builder to describe values that must live together. Every
declaration ends in a terminal method before `build`. A finite plan exports
completed data; it does not export a live resource or service whose owner is
about to clean it up.

Run a plan through `PlanStart` and keep the complete report until a caller has
made its decision:

```rust
let report = plan.start(runtime, input).await;
match report.into_result() {
    Ok(output) => use_completed_output(output),
    Err(report) => record(
        report.outcome,
        report.faults,
        report.cleanup_failures,
        report.incomplete,
        report.ambiguous,
    ),
}
```

`into_result` returns `Result<Option<Arc<Out>>, Report<Out>>`. Its error is the
entire report, including the original body faults, cleanup failures, abandoned
work, and unresolved external outcomes. Do not replace it with a string or a
single first fault.

## Values and ownership

A plan has no output until you select one with `p.export(result_key)` before
`build`. Merely creating the last step does not export its value. For a finite
function returning `T`, build `Plan<T, Input>` and extract the completed value
from `report.into_result()?.ok_or("missing output")?`; that value is `Arc<T>`.
Do not try to recover a value from a `Plan<()>` report by casting it.

`Key<T>` is a declaration handle, not a live `T`. Dependencies arrive as
`Arc<T>` automatically. Return `T` from a step or initializer; return
`Held<T>` from an acquire/perform body. Returning `Arc<T>` deliberately makes
the node output type `Arc<T>`, so its dependents receive `Arc<Arc<T>>`.

| Expression | Result | Await? |
|---|---|---|
| `cx.hold(|| operation_returning_result_of_T())` | future yielding `Result<Held<T>, Error>` | yes |
| `cx.hold_value(value_of_T)` | `Held<T>` | no; return `Ok(...)` |
| `.needs(key_of_T)` body argument | `Arc<T>` | no |
| `.needs((a, b))` body argument | `(Arc<A>, Arc<B>)` | no |
| `.needs(one_key_of_tuple)` body argument | `Arc<(A, B)>` | no; borrow its fields |

Both hold methods consume `Cx<Acquire>`. Save `let shared = cx.shared()` first
if later work needs its cancellation or clock operations. Put the external
acquisition itself inside the lazy `hold` factory, not before it.

A tuple or struct containing an `Arc<Resource>` is ordinary data: wrapping a
resource in a step output does not transfer its cleanup dependency. Bind the
resource key directly as child input, or declare a formal resource port and
bind that port to the parent resource key. Every child resource using the
borrowed resource must name that input/port in `.needs(...)`.

## Resources and effects

An acquire or perform body must create its externally visible value inside the
single `cx.hold` call. `hold` registers a value in the poll that observes its
future return `Ok`, so cancellation cannot arrive between acknowledged success
and the cleanup obligation. It does not prove that an interrupted remote
operation did not happen; use `on_ambiguous` and recovery for that case. Use
`cx.hold_value(value)` only for a value already owned without an external action.

```rust
let connection = p
    .resource("connection")
    .needs(config)
    .acquire(|cx: Cx<Acquire>, config: Arc<Config>| async move {
        cx.hold(|| async move { connect(&config).await }).await
    })
    .release(|cx: Cx<Release>, connection: Arc<Connection>| async move {
        close(&connection, cx.deadline()).await
    });
```

Declare retries on the node; do not write a loop that tries to reuse or clone
`Cx<Acquire>`. For two total attempts with fixed backoff, put these attributes
before `.acquire(...)` (or before `.identified_by(...)` for recovery effects):

```rust
.idempotent()
.retry(Retry::attempts(2).backoff(Backoff::fixed(Duration::from_millis(1))))
```

Each attempt gets fresh acquisition authority after the preceding attempt and
any registered cleanup settle. `idempotent()` is the application's safety
claim, not proof that repeating its external operation is safe.

Write one resource per independently acquired value. A release body receives
the value registered by its matching acquire body. Effects use the same
acquisition rule and end with either `.compensate(...)` or `.persistent()`.

For recovery effects, write `.needs(...)` and other node attributes before
`.identified_by(operation_key)`. After identification, the next method is
`.perform(...)`. Its body argument is `(ordinary_dependencies, Arc<Identity>)`:
with `.needs(config_key)`, that is `(Arc<Config>, Arc<Identity>)`; without
ordinary dependencies it is `((), Arc<Identity>)`. The recovery handler receives
only `Arc<Identity>`, not the ordinary dependency tuple.

For an external operation that can time out after it may have happened, declare
an identity before the operation and supply reconciliation for the receipt-less
case. Normal compensation receives a real receipt; `recover_unknown` receives
the recorded operation identity.

```rust,guide:unknown_recovery
use sdax::prelude::*;
use sdax_tokio::{PlanStart, TokioRuntime};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[test]
fn recovery_uses_the_recorded_identity_after_an_unknown_outcome() {
    let recovered = Arc::new(Mutex::new(Vec::new()));
    let seen = recovered.clone();
    let mut p = Plan::with_input::<u64>("reserve");
    let operation = p.input();
    p.effect("remote reservation")
        .idempotent()
        .within(Duration::from_millis(5))
        .on_ambiguous(Ambiguity::Recover)
        .identified_by(operation)
        .perform(|cx, ((), operation)| async move {
            cx.hold(|| async move {
                let _ = operation;
                std::future::pending::<Result<u64, Error>>().await
            })
            .await
        })
        .recover_unknown(move |_cx, operation| {
            let seen = seen.clone();
            async move {
                seen.lock().expect("record").push(*operation);
                Ok(Recovery::Resolved)
            }
        })
        .persistent();
    let plan = p
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_millis(20)),
            Mode::Finite,
        )
        .expect("valid plan");

    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .expect("runtime");
    let rt = Arc::new(TokioRuntime::current_thread_no_background_drain(
        tokio_rt.handle().clone(),
    ));
    let report = tokio_rt.block_on(plan.start(rt, 42));

    assert_eq!(*recovered.lock().expect("record"), [42]);
    assert!(
        report.ambiguous.is_empty(),
        "recovery resolved the uncertainty"
    );
    assert!(
        report.cleanup_failures.is_empty(),
        "recovery completed cleanly"
    );
}
```

`Recovery::Resolved` discharges the uncertainty. `Recovery::StillUnknown`, an
error, panic, or expiry leaves the unresolved entry in the report. Recovery is
for reconciliation, not a promise of exactly-once external execution.

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

## Body results

All body errors use `sdax::Error`. When type inference needs help, write an
explicit result such as `Ok::<Handle, Error>(handle)` or
`Err::<(), Error>(cause.into())`. Body factories may be called again for a
declared retry or service restart, so capture cloneable configuration and keep
attempt-specific state inside the returned future.

For finite work, use `.step(...).run(...)`. Use `cx.stop().await` in a resident
service episode that should end cooperatively. Release, compensation, recovery,
and service shutdown have the declared deadline; pass `cx.deadline()` into APIs
that support timeouts.
