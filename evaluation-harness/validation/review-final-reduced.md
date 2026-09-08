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
