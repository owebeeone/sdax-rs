# sdax-rs API reference

`sdax` is std-only. `sdax-tokio` is the adapter and run driver.
`sdax-testkit` is `publish = false`. Author surface: `sdax::prelude` +
`sdax_tokio::{TokioRuntime, PlanStart}`. Do not implement
`sdax::host` (`Runtime`, `Clock`, `Observer`, `Machine`).

## Install

Before crates.io publication, install both crates from the same Git repository
(read access is required while it is private):

```toml
[dependencies]
sdax = { git = "https://github.com/owebeeone/sdax-rs", package = "sdax" }
sdax-tokio = { git = "https://github.com/owebeeone/sdax-rs", package = "sdax-tokio" }

[dev-dependencies]
tokio = { version = "=1.53.1", default-features = false, features = ["rt", "time", "test-util"] }
```

Cargo records the common resolved commit in your application's `Cargo.lock`.
The Tokio dev-dependency supports the guide tests. Use registry dependency
coordinates after both Rust crates are published.

`Error` = `Box<dyn std::error::Error + Send + Sync + 'static>`.

A **plan** is immutable, `Send + Sync`. `build` once; each `start` is
a **run** with its own slots. Nodes + typed `needs` edges; eligible
when every need is ready. Release graph is the reverse of `needs`. No
waves, no levels.

1. Resource/effect body returns `Held<T>` — only `cx.hold` /
   `cx.hold_value` mint one.
2. A service is ready when `initialize` returns its handle.
3. `build(policy, shutdown, mode)` and effect `on_ambiguous` have no
   defaults.

## Plan, key, deps

```rust
Plan<Out = (), In = ()>
```

- `Out` — `.export(key)`, else `()`.
- `In` — typed input supplied at start, mount or spawn. `Plan::builder`
  is exactly the `Plan::with_input::<()>` unit-input definition.

```rust
Plan::builder("Name") -> PlanBuilder<(), ()>
Plan::with_input::<In>("Name") -> PlanBuilder<(), In>  // per-run input
input() -> Key<In>                 // present on every builder
port::<T>(name) -> Key<T>          // typed formal import
plan.bind(port: Key<T>, parent: Key<T>) -> Result<Plan<Out, In>, Invalid>
import(parent: Key<T>) -> Key<T>   // concrete ancestor binding
component(name, &Plan<O, I>, input: Key<I>) -> Key<O> // () also accepted for I=()
export(key: Key<T>) -> PlanBuilder<T, In>
pool(name, limit) -> Pool
spawns(service: Key<H>, template: &Template<I>)  // late form
build(self, Policy, Shutdown, Mode) -> Result<Plan<Out, In>, Invalid>
```

`Key<T>` is `Copy`; `needs` yields `Arc<T>`. Unbranded across plans:
foreign key → `V-FOREIGN-KEY`. Exists only after the terminal method
(undeclared/cycle → `E0425`; body/`needs` mismatch → `E0631`).

`Deps`: `()`, one `Key<A>`, or a tuple up to eight. `Deps::Out` is
`()`, `Arc<A>`, or the matching `Arc` tuple.

## Kinds

A node joins only at its terminal. Type state prevents using an unfinished
chain as a key; if such a chain is dropped, `build` reports the missing
terminal as a finding.

| Kind | Construct | Terminal | `cx` | Receives | Returns | Dependents |
|---|---|---|---|---|---|---|
| resource | `.resource(name)` | `.acquire(f).release(g)` | `Acquire` / `Release` | `D::Out` / `Arc<T>` | `Held<T>` / `()` | `Arc<T>` |
| step | `.step(name)` | `.run(f)` | `Run` | `D::Out` | `T` | `Arc<T>` |
| try-step | `.try_step(name)` | `.run(f)` | `Run` | `D::Out` | `T` (failure is a value) | `Arc<Result<T, Error>>` |
| blocking step | `.blocking_step(name).on(pool)` | `.run(f)` sync | `Run` | `D::Out` | `T` | `Arc<T>` |
| service | `.service(name)` | `.initialize(f).serve(g)` | `Start` / `ServingPhase` | `D::Out` / `Arc<H>` | `H` / `()` | `Arc<H>` |
| effect | `.effect(name).on_ambiguous(a)` | `.perform(f).compensate(g)` or `.persistent()` | `Acquire` / `Release` | `D::Out` / `Arc<R>` | `Held<R>` / `()` | `Arc<R>` |
| join | `.join(name, deps)` | the call | — | — | — | `Arc<()>` |
| component | `.component(name, &plan, ())` | the call | — | — | — | `Arc<O>` |
| template | `.template(name, &plan)` | the call | — | — | — | `Template<I>`; node never ready |

Bodies return `Result<…, Error>` except as shown. `release::by_drop()`
is RAII (`inspect` prints `release: drop`). `initialize` returns the stable
handle once; `serve` receives an `Arc` of that handle for every episode.
`.on(pool)` before blocking `run`. `.on_ambiguous(a)` before `perform`.

## Attributes

Configure attributes before the body's terminal method. Around
`.identified_by(key)`, `.needs`, `.within`, `.retry` and `.idempotent` work on
either side. Other attributes belong before identification. `.needs` replaces
ordinary dependencies while retaining the identity dependency; the perform body
still receives `(dependencies, Arc<Identity>)`. Recovery remains mandatory before
compensation or persistence. A finished `Key` is not a configuration builder.

| Method | On | Meaning |
|---|---|---|
| `.needs(deps)` | any | data + eligibility |
| `.within(d)` | prepare/run | bound that body |
| `.retry(Retry)` | prepare | `Retry::attempts(n)` = **total** attempts |
| `.idempotent()` | node | re-execution is safe |
| `.exclusive(res)` / `.shared(res)` | node | lock a resource already in `needs` |
| `.limit(pool)` | non-blocking node | concurrency cap; blocking nodes use `.on(pool)` |
| `.cooperative(grace)` | node | signal `cx.stop()`, poll, then abort |
| `.stop_within(d)` | service | bound stop |
| `.restart(Restart)` | service | re-run serve on `Err` |
| `.terminal()` | service | finishing ends the scope |
| `.spawns(&template)` | service | may `cx.spawn` it |

`Retry::attempts(n).backoff(Backoff::fixed(d) | exponential(initial, factor, cap))`.
`Restart::on_error(backoff).max(n)`.
Default cancel is `Drop`. `cooperative` on a blocking step →
`V-BLOCKING-CANCEL`.

`Ambiguity` (required): `Report` | `Recover` | `Retry`. `Recover` needs
`.idempotent()`, `.identified_by(key)`, and `.recover_unknown(handler)`;
`Retry` needs `.idempotent()`.

## `Cx`

`Cx<P>`: `Acquire`, `Run`, `Start`, `ServingPhase`, `Release`. Wrong-phase ops do not
exist.

Every phase: `stop()`, `is_stopping()`, `until_stop(f)` (`None` if
stop wins), `sleep(d)`, `timeout(d, f)`, `now()`, `deadline()`,
`attempt()` (from 1), `spawn(&template, input) -> Result<Child, SpawnError>`.

`Cx<Acquire>` only (`acquire` / `perform`):

- `hold(|| future)` — consumes the single-use acquisition authority, invokes
  the factory once, and registers its `Ok` value in that observing poll.
  This context cannot be cloned. Save `let shared = cx.shared()` before
  consuming it if later work needs clock/cancellation operations; the shared
  context has no acquisition authority.
- `hold_value(v)` — consumes that authority to register a value already owned.
  Effect-then-`hold_value` is invisible to the engine.

`Held<T>`: no public ctor; `Deref` to `T`.
`Child`: `id()`, `stop()`, `ready()`. Awaiting `ready()` from `initialize`
includes the instance in parent readiness. Not a `needs` edge; no
output to parent.
`SpawnError`: `ForeignTemplate`, `UndeclaredTemplate`,
`ScopeStopping`, `NotRunning`.

Acquire in a resource, not in service `initialize`. No `tokio::spawn` —
use `cx.spawn`.

## Policy

`Policy::FailFast` — first fault stops admits, cancels in-flight
non-services. `Isolate` — skip dependents, aggregate faults.

`Shutdown::within(d)` — settle-and-cleanup budget.
`unbounded()` — every service needs `stop_within`.

`Mode::Finite` — end at settle; **rejected** with a service or
template (`V-MODE`). `Resident` — stay until `shutdown()`,
`cancel()`, `FailFast` fault, or a `terminal` service.

## Findings

`Invalid { checks: Vec<Finding> }` — `rule`, `nodes`, `keys`,
`detail`, `fix`. All at once.

| `Rule` | Id | Cause |
|---|---|---|
| `IncompleteDeclaration` | `V-INCOMPLETE-DECLARATION` | a declaration chain was left unfinished |
| `LiveExport` | `V-LIVE-EXPORT` | a finite plan exports a resource or service handle |
| `Empty` | `V-EMPTY` | no runnable nodes (input / import do not count) |
| `ForeignKey` | `V-FOREIGN-KEY` | key/pool of another plan |
| `DupName` | `V-DUP-NAME` | two nodes, one name, one scope |
| `DupAttr` | `V-DUP-ATTR` | attribute twice, or resource locked twice |
| `ImportScope` | `V-IMPORT-SCOPE` | import a key this plan does not own |
| `SpawnSelfImport` | `V-SPAWN-SELF-IMPORT` | template imports the spawning service |
| `SpawnKind` | `V-SPAWN-KIND` | `spawns` on a non-service |
| `LockNeeds` | `V-LOCK-NEEDS` | lock a resource not in `needs` |
| `IdempotentRequired` | `V-IDEMPOTENT-REQUIRED` | retry, recovery, or retrying an effect without `.idempotent()` |
| `PoolStarve` | `V-POOL-STARVE` | resident holder can starve the pool |
| `UnusedPool` | `V-UNUSED-POOL` | unused pool |
| `ServiceUnbounded` | `V-SERVICE-UNBOUNDED` | `unbounded()` + service, no `stop_within` |
| `TryUnconsumed` | `V-TRY-UNCONSUMED` | try-step with no dependent |
| `BudgetOrder` | `V-BUDGET-ORDER` | inner budget > outer |
| `Mode` | `V-MODE` | `Finite` + service or template |
| `BlockingCancel` | `V-BLOCKING-CANCEL` | `cooperative` on a blocking step |
| `BlockingLimit` | `V-BLOCKING-LIMIT` | both `.limit` and `.on` on a blocking step |
| `RecoveryMissing` | `V-RECOVERY-MISSING` | recovery lacks its required typed identity or handler |

## Inspect / simulate

Pure. `plan.inspect() -> PlanView`: `name`, `semantics` (`sdax/2`),
`mode`, `policy`, `shutdown`, `nodes`, `edges`, `pools`.
`node(path)`, `layers()` (not barriers), `release_order().before(a,b)`,
`why(node)`, `effects()`, `Display`, `diff`.

`plan.simulate(&Script) -> Result<Trace, ScriptError>`. Unspecified
bodies succeed at tick 0. It works for a unit-input plan, including
`Plan::builder`. A plan with a non-unit per-run input uses
`plan.simulate_with_input(input, &script)`. The value is not read
(no body runs); supplying it is what lets the plan run at all.
`Script::new().prepare("N", Body::ok(At::tick(0.0)))`.
`.at(secs, Request::Shutdown | Cancel)`.
`Schedule::Fifo` or `Schedule::order(["A","B"])`.

## Start

`use sdax_tokio::PlanStart;`

```rust
fn start<R: Runtime>(&self, rt: Arc<R>, input: In) -> Running<Out>
fn start_with(&self, rt, input, RunOptions) -> Running<Out>
fn try_start(&self, rt, input) -> Result<Running<Out>, EngineError>
fn try_start_with(&self, rt, input, RunOptions) -> Result<…>
```

On `Plan<Out, In>`. `Plan::builder` → `start(rt, ())`.
`Plan::with_input::<In>` → `start(rt, value)`. The same plan value
built with `Plan::with_input::<In>` is startable as a root with that
input; `cx.spawn(&template, input)` is the instance path.
`RunOptions`: `bodies`, `record`, `schedule`, `observe_states`.
`try_start` refusal (`L-IMPORTS`) → `EngineError`; `start` →
`Failed` report.

No `macros` feature: no `#[tokio::test]` / `#[tokio::main]`. The
test below is a current-thread runtime, the acknowledging
constructor, and one finite start.

```rust,guide:current_thread_start
use sdax::prelude::*;
use sdax_tokio::{PlanStart, TokioRuntime};
use std::sync::Arc;
use std::time::Duration;

#[test]
fn a_current_thread_runtime_starts_a_finite_plan() {
    let mut p = Plan::builder("Startup");
    p.step("Ping").run(|_cx, ()| async move { Ok(()) });
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
    assert!(report.is_clean());
}
```

Neither constructor owns the runtime. `TokioRuntime::new(Handle)` takes
a multi-threaded handle and panics on a `current_thread` one.
`TokioRuntime::current_thread_no_background_drain` is the
acknowledgement: a dropped `Running` cannot drain on that flavour, so
always await shutdown. A multi-threaded handle is accepted there too;
`new` is the constructor that says less.
`.with_clock` / `.with_observer`. `shutdown(budget) -> Result<(), usize>`
(`Err(n)` = n still running). `tracked()`.

Bodies are the closures from `build`. They do not receive a shared
context from `start`. A declared input is a `Key<In>` they `needs`;
the driver seeds that slot before the first body is built. Concurrent
`start`s of one `Plan` are isolated.

## `Running` / `Report`

`Running<Out>`: `#[must_use]`, `Future<Output = Report<Out>>`. Lazy —
first poll spawns. `cancel()` before that: no effect.

| | |
|---|---|
| `await` | report |
| `ready().await` | `&mut self`; starts if needed. `Ok(())` at steady, `Err(outcome)` if ended first |
| `shutdown()` / `cancel()` | normal end / interrupt; release still runs |
| `handle()` | clonable `RunHandle` |
| `snapshot()` | last step |

Drop cancels and leaves one tracked **drainer**. The drainer is a
task: `current_thread` does not progress between `block_on`s, which
is why that flavour is built with
`current_thread_no_background_drain` and every run is awaited. A
blocking body that never returns cannot be aborted and blocks
`Runtime::drop` — use tokio `shutdown_timeout` /
`shutdown_background`. `FaultKind::Panic` needs `panic = "unwind"`.

```rust
Report<Out> {
    outcome: Outcome,  // Ok | Failed | Cancelled
    output: Option<Arc<Out>>,
    faults, cleanup_failures: Vec<Fault>,
    incomplete: Vec<NodeRecord>,  // abandoned at budget
    ambiguous: Vec<NodeRecord>,   // effect started, no hold
    trace: Option<Trace>,
}
```

`is_clean()` = empty lists and `Ok`.
`into_result() -> Result<Option<Arc<Out>>, Report<Out>>`.
Use it when a clean run may legitimately produce no output.

`into_required_output() -> Result<Arc<Out>, RequiredOutputError<Out>>`.
Use it when completed output is required. `Failed(report)` means the run was
unclean; `MissingOutput(report)` means it was clean but produced no output.
Failure takes precedence even if output is absent. Both variants retain the
complete report, typed causes and trace. `error.report()` borrows it and
`error.into_report()` returns it. Neither helper invents a default output.

`Fault`: `node`, `order`, `phase`, `kind`.
`FaultKind`: `Error`, `Panic`, `Timeout`, `DoubleHold`.
`Phase`: `Prepare`, `Run`, `Serve`, `ReleaseBody`, `Compensate`, `Recover`,
`Stop`.

## Templates

1. `Plan::with_input::<In>("Name")` + `.input()` / `.import(k)` + `build`.
2. `let t = parent.template("Name", &child)`.
3. `.service(..).spawns(&t)`.
4. `cx.spawn(&t, input)?; child.ready().await?;`

Imported keys released only after the instance ends. Template must
not import the spawning service (`V-SPAWN-SELF-IMPORT`). Parent that
declares a template cannot be `Finite`.
