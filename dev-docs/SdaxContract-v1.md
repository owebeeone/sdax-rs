# `sdax/1` — the contract

Normative for the `sdax` crate. Semantics tag: **`sdax/1`** (`sdax::SEMANTICS`).
A change of meaning changes the tag.

Base: `sdax-v1/B/Proposal.md` (Designer B), adopted by
`sdax-v1/reviews/Comparison.md` § 1, with the adoptions and fixes in § 12 below.
Where this document and the proposal disagree, this document governs.

**Implementation scope.** This document states the whole contract. The crate
today implements the authoring surface, the validator, inspection, the seam, the
host contracts, `host::engine::Machine` (T1–T8 for static plans and components),
`Plan::simulate`, and — from Stage 2 — the tokio run driver: `plan.start(rt)`,
`Running` with its drop guard and drainer, and blocking pools. The driver is in
`sdax-tokio`, because that is the only crate allowed to spawn (A2); `start` is
the extension trait `sdax_tokio::PlanStart` for the same reason (OD-START).
Rows marked *(Stage 3)* are specified and not implemented, and the crate says so
where they are named.

---

## 1. Vocabulary

A **plan** is an immutable, reusable, `Send + Sync` value, built once and (from
Stage 2) started any number of times; each start is a **run** with its own
slots, locks and pools. A plan is a **scope**: its nodes share one fail policy,
one shutdown budget and one run mode. Scopes nest as **components** (a plan used
as one node, instantiated once per parent run) and **templates** (a plan
instantiated at run time by a body, with a per-instance input).

| kind | prepare body | becomes *Ready* when | output to dependents | obligation exists iff | cleanup action |
|---|---|---|---|---|---|
| `resource` | `acquire(cx, deps) -> Result<Held<T>>` | the body returns `Ok` | `Arc<T>` | `cx.hold`/`hold_value` registered a value | `release(cx, Arc<T>)`, or `release::by_drop()` |
| `step` | `run(cx, deps) -> Result<T>` | the body returns `Ok` | `Arc<T>` | never | none |
| `try_step` | `run(cx, deps) -> Result<T>` | the body returns (`Ok` **or** `Err`) | `Arc<Result<T, Error>>` | never | none |
| `blocking_step` | `run(cx, deps) -> Result<T>`, synchronous, on a declared pool | returns `Ok` | `Arc<T>` | never | none |
| `service` | `start(cx, deps) -> Result<Serving<H>>` | the start body returns `Ok(Serving)` | `Arc<H>` | started (a serve future exists) | signal, wait ≤ `stop_within`, abort |
| `effect` | `perform(cx, deps) -> Result<Held<R>>` | the body returns `Ok` | `Arc<R>` (the receipt) | `hold` registered a receipt **and** the effect is not `persistent` | `compensate(cx, Arc<R>)`, reported distinctly from a release |
| `join` | none | every need is Ready | `Arc<()>` | never | none |
| `component` | none (a nested plan) | the inner run reaches steady state | `Arc<Out>` | the inner run started | the inner release graph, as one unit |
| `template` | none (a nested plan factory) | n/a | none | live instances exist | stop every live instance |

Edges and constraints:

| construct | meaning | checked by |
|---|---|---|
| `.needs(deps)` | ordering **and** typed dataflow; the body receives `Arc<T>` per key | compiler (existence, type), `V-FOREIGN-KEY` |
| `import(parent_key)` | a cross-scope need in a child plan; the parent's node is released only after the child is cleaned up | `V-IMPORT-SCOPE` |
| `.exclusive(k)` / `.shared(k)` | arbitration on a resource the node already needs | `V-LOCK-NEEDS` |
| `pool(name, n)`, `.limit(pool)`, `.on(pool)` | a per-run concurrency budget; `.on` is the required pool of a blocking step | `V-POOL-STARVE`, `V-UNUSED-POOL` |
| `.spawns(&template)` | this service may instantiate this template (F1) | `V-SPAWN-KIND`, `V-SPAWN-SELF-IMPORT`, `V-FOREIGN-KEY` |
| `.export(key)` | the plan's typed output | compiler |

Per-node attributes: `.within(d)`, `.retry(Retry)`, `.restart(Restart)`
(services), `.idempotent()`, `.stop_within(d)` (services), `.terminal()`
(services), `.on_ambiguous(Ambiguity)` (effects, **required**),
`.cooperative(grace)`. Per scope, all three **required** at `build`:
`Policy::{FailFast, Isolate}`, `Shutdown::{within(d), unbounded()}`,
`Mode::{Finite, Resident}`.

Every body-carrying kind has a positional constructor that is definitionally the
chain form: `resource_with`, `step_with`, `try_step_with`, `blocking_step_with`,
`service_with`, `effect_with`, `effect_persistent_with`.

Time is `std::time::Duration` on an injected `Clock`; a `Time` is nanoseconds
from that clock's origin.

---

## 2. States

**Node states** (per run; per instance for a template's nodes)

| state | meaning |
|---|---|
| `Pending` | declared; not yet considered |
| `Waiting{on}` | a need, a lock or a pool grant is missing; `on` lists each reason |
| `Running{attempt, held}` | the prepare body is in flight; `held` flips at `Held` |
| `Backoff{attempt, until}` | between failed attempts |
| `Ready` | see § 1; a service is also `Serving` → `Finished` \| `Faulted` |
| `Failed{fault, held}` | attempts exhausted with `Err`/panic/timeout |
| `Interrupted{held}` | the engine cancelled the body; never a fault |
| `Ambiguous` | an effect interrupted or timed out after start and before `hold` |
| `Skipped{because}` | a need ended `Failed`/`Interrupted`/`Ambiguous`/`Skipped`, or the scope stopped admitting |
| `Releasing`/`Stopping`/`Compensating` → `Released`/`Stopped`/`Compensated` \| `ReleaseFailed{fault}` \| `Abandoned` | discharging the obligation; `Abandoned` = the budget expired |

**Run states**: `Planned` → `Admitting` → `Steady` → `Settling` → `Cleanup` →
`Ended(Report)`. Transitions into `Settling`: `shutdown()`; `cancel()` or drop
of `Running`; a fault under `FailFast`; a `terminal` service finishing; or,
under `Mode::Finite`, reaching `Steady`.

`Mode` is per scope, and a nested scope has its own. A `Finite` parent may
contain a `Resident` component; the component becomes Ready when its inner run
is steady and its residency ends with the parent's cleanup. (Owner decision,
2026-09-05.)

## 3. Transitions

| id | rule |
|---|---|
| T1 | *start*: `Waiting → Running` iff every key in `needs ∪ imports` is `Ready` **and** the node's lock and pool grants are taken atomically (all or none, in a fixed global order, FIFO among waiters). Nothing else is required: no wave, no level, no sibling. |
| T2 | *hold*: on `Held(N)` the engine records the `Arc<T>` and the obligation, in the poll that observes the effect completing, before the body's continuation can be polled again. |
| T3 | *ready*: on the body's `Ok`, `Running → Ready`; dependents re-evaluate T1. |
| T4 | *fail*: on `Err`/panic/timeout, `Backoff` if attempts remain (and, if `held`, this attempt's release finishes first), else `Failed`. Under `FailFast` the scope enters `Settling`; under `Isolate` transitive dependents become `Skipped`. |
| T5 | *interrupt*: in `Settling`, every `Running` non-service node is cancelled per its cancel mode (`drop`: abort; `cooperative(g)`: signal, wait `g`, abort); the engine **joins** the aborted task before the node counts as settled. Result `Interrupted{held}`, or `Ambiguous` for an effect not yet held. |
| T6 | *cleanup order*: a node's release starts iff every node that needs or imports it — and every live instance of a template that imports it — has finished its cleanup (released, failed, or abandoned). Unrelated nodes run concurrently. |
| T7 | *bound*: the shutdown budget starts at the transition into `Settling`; on expiry everything still running is aborted (services after their own `stop_within`, if shorter) and recorded `Abandoned`. A blocking body cannot be aborted; it is `Abandoned` while its thread finishes. |
| T8 | *end*: `Ended(Report)` when every obligation is discharged or abandoned and every spawned task is joined or recorded. |

## 4. Invariants

INV-1…16 are Proposal B's, unchanged. INV-17…20 are this contract's.

| id | invariant |
|---|---|
| INV-1 | **Declared edges only.** N starts only after every key in `needs(N) ∪ imports(N)` is Ready; the engine adds no other start ordering except lock and pool arbitration among nodes that declare the same lock or pool. |
| INV-2 | **Readiness is a return.** `Ready(N)` is recorded only when N's prepare body returned `Ok`. Being spawned never implies readiness. |
| INV-3 | **Held ⇒ released.** If `Held(N)` occurred, exactly one release or compensation attempt for N occurs before `End`, unless the budget expires first, in which case `N ∈ incomplete`. |
| INV-4 | **Not held ⇒ no release body.** If `Held(N)` did not occur, no release body runs for N. |
| INV-5 | **Alive-during.** For every M with N ∈ `needs(M) ∪ imports(M)`, the release of N does not start before M's cleanup has ended. |
| INV-6 | **Cleanup concurrency is explicit.** Releases unrelated by INV-5 may overlap; `inspect()` marks them unordered; no sequence is promised. |
| INV-7 | **Shielding.** A cleanup body is never dropped because of a cancel or shutdown request; only the budget can abandon it, and abandonment is recorded. |
| INV-8 | **Bounded shutdown.** From `Settling` to `End`, at most `Shutdown::within(d)` elapses on the engine clock (unless `unbounded()`). |
| INV-9 | **No silent loss.** Every fault, cleanup failure, panic, timeout, abandonment and ambiguity the engine observes is in the `Report`; `into_result()` is `Err` unless the report is clean. |
| INV-10 | **Cancelled ≠ failed.** Engine-interrupted nodes are `Interrupted`, never faults; `Outcome::Cancelled` is reported only for an external `cancel()`/drop before `End`. |
| INV-11 | **Ambiguity.** An effect interrupted after start and before `hold` is `Ambiguous`; it is neither retried nor compensated unless it is `idempotent` with the matching `Ambiguity`. |
| INV-12 | **Attempt bracketing.** A retried node's attempts never overlap; if attempt *k* held, its release finishes before attempt *k+1* starts; backoff uses the engine clock and ends immediately on cancel. |
| INV-13 | **Run isolation.** Two runs of one plan share no slots, no locks and no pools. |
| INV-14 | **Determinism.** Given a script and a schedule, the machine emits the same effect sequence and the same `Report`. |
| INV-15 | **No orphans.** At `End`, every task the engine spawned has been joined or is listed as abandoned; the engine never detaches child work. |
| INV-16 | **Instance containment.** A template instance's nodes can need only its own nodes, its input and imported parent keys; a parent node can never name an instance node; every live instance ends before any key it imports is released. |
| **INV-17** | **Readiness may include instances (F1).** A start body may `cx.spawn` a template it declared and await `Child::ready()` before returning `Serving`; the scope's readiness then includes those instances. A template a service spawns must not import that service's key — the instance could never be ready while the service waits for it — and `V-SPAWN-SELF-IMPORT` rejects that at `build`. |
| **INV-18** | **A persistent effect is still an effect (F2).** An effect declared `.persistent()` carries no compensation obligation: nothing runs for it at shutdown, and no `cleanup_failure` can arise from it. It remains an effect for every other purpose — `on_ambiguous` is still required, it is listed by `Plan::effects()`, and `inspect()` shows it as `effect (persistent)`. |
| **INV-19** | **The run mode is declared (F3).** `Mode` is a required argument of `build`. Nothing about a plan's structure changes when the run ends; `Finite` with a service or a template is rejected (`V-MODE`), and `Resident` with no services is accepted. |
| **INV-20** | **The report has an order (F4).** `faults`, `cleanup_failures`, `incomplete` and `ambiguous` are ordered by `RecordOrder`: node declaration order outermost-first, a template's own record before its instances', instances by id, and attempts within a node in attempt order. `trace` is in observation order and is never reordered. |

**Valid schedules.** Any interleaving satisfying T1–T8 is valid. Promised equal
across all of them: the set of Ready nodes at `Steady`; under `Isolate` the set
of faults; under `FailFast` the first fault and that no node starts after it;
the release set (INV-3/4); the partial order of releases (INV-5); the
clean/unclean status of the report. **Not** promised: the relative order or
overlap of unordered starts and releases; which of two ready exclusive users
runs first; which in-flight siblings of a first fault finish before the cancel
reaches them; wall-clock durations.

---

## 5. The seam

The complete list of ways a body can affect lifecycle state. Everything else a
body does is ordinary Rust and is not the engine's business.

| operation | available on | effect |
|---|---|---|
| `cx.hold(effect_future).await` | `Cx<Acquire>` (resource, effect) | runs the effect under the engine's wrapper; registers the value and the obligation in the poll that observes completion |
| `cx.hold_value(v)` | `Cx<Acquire>` | registers an effect-free value; registration completes before the call returns |
| `Serving::new(handle, serve)` | returned by a start body | hands the serve future over; readiness is the return |
| `cx.stop()`, `cx.until_stop(f)`, `cx.is_stopping()` | all phases | observe the stop request |
| `cx.sleep(d)`, `cx.timeout(d, f)`, `cx.now()`, `cx.deadline()` | all phases | the injected clock |
| `cx.attempt()` | all phases | the attempt number, from 1 |
| `cx.spawn(&template, input)` | all phases | instantiate a declared template; refused while settling *(engine side: Stage 3)* |
| `child.ready()`, `child.stop()`, `child.id()` | on a `Child` | await an instance's readiness (INV-17); ask it to stop *(Stage 3)* |

**The one rule of the body contract.** Perform external effects *inside*
`cx.hold(..)`. A body that performs the effect itself, awaits something else and
then calls `hold_value` has re-created the window `hold` exists to close; the
engine cannot see that, and it is the seam's stated residue, not a promise.

**A service's own acquisitions belong in a resource node, not in `start`.** A
value a start body creates has no ledger entry and no async release. Declare it
as a resource and `needs` it, or keep it in the serve future's locals so its
`Drop` runs (`AdversarialReview.md` F-B6).

**Escapes.** The sanctioned escape is the body itself: hermetic (a body cannot
mutate the plan, the release graph, another node's slot or engine state except
through the operations above) and visible (every seam call is `cx.`). The
unsanctioned escape is a body that reaches another node through a channel or a
global; it cannot change what the engine derives, but it can create a wait the
engine does not know about. A raw `tokio::spawn` in a body is that escape in its
worst form, and `clippy.toml` refuses it in every crate but `sdax-tokio`.

---

## 6. Defaults

**Never defaulted**: the fail policy, the shutdown budget, the run mode, an
effect's ambiguity handling, a blocking step's pool, and a resource's release.
Each is a required argument or a required typestate step.

| omitted | engine value | shown by `inspect()` |
|---|---|---|
| `Policy`, `Shutdown`, `Mode` | **no default; required arguments of `build`** | — |
| `on_ambiguous` on an effect | **required** (typestate) | — |
| `.on(pool)` on a blocking step | **required** (typestate) | — |
| `release` / `compensate` / `persistent` | **required** (typestate) | — |
| `within` | none; the shutdown budget still bounds cleanup | `within —` |
| `stop_within` | the remaining shutdown budget | `stop_within — (bounded by shutdown 10s)` |
| `retry` / `restart` | none; re-execution is never implied | `retry —` |
| cancel mode | `drop` for resource/step/effect; signal-then-deadline for services | `cancel: drop` |

Every value `inspect()` prints is the *resolved* one, and `PlanView::diff` marks
a change as `default_changed` when neither side declared the attribute — so an
upgrade that moves an engine default is reviewed as a diff.

---

## 7. Gate inventory

### Decode — the Rust type system

Each row is witnessed: the doctests in `sdax::compile_fail` fail to compile, and
`scripts/compile-fail.sh` asserts the code.

| id | rejects | mechanism | witness | code |
|---|---|---|---|---|
| D-DANGLING | a key named before its node exists; a cycle | a key exists only after its terminal | W-01 | `E0425` |
| D-HELD | an effect performed outside the wrapper, bare value returned | the terminal's bound requires `Held<T>` | W-02 | `E0271` |
| D-FORGE | a forged `Held` | private fields | W-03 | `E0451` |
| D-PHASE | a seam call in the wrong phase | phase-typed `Cx` | W-04 | `E0599` |
| D-TYPE | a body whose parameters disagree with `needs` | closure signature vs `D::Out` | W-05 | `E0631` |
| D-TERMINAL | a resource with no release | `NeedsRelease` is not a `Key`, and `#[must_use]` | W-06 | `E0308` |
| D-SCOPE | a parent naming a child plan's key | lexical scoping | W-07 | `E0425` |
| D-AMBIG | an effect with no `on_ambiguous` | `Effect<NoAmbiguity>` has no `perform` | W-09 | `E0599` |
| D-POOL | a blocking step with no pool | `Blocking<NoPool>` has no `run` | W-10 | `E0599` |
| D-POLICY | `build` without policy, shutdown or mode | required arguments | W-11 | `E0061` |
| D-ROOT | a template used where a component is required | `In` is part of the type | W-12 | `E0308` |
| D-SEND | a `!Send` body | terminal bound | W-13 | `E0277` |
| D-EFFECT-RECORD | an effect that says nothing about its record | `NeedsCompensate` is not a `Key` | W-16 | `E0308` |
| D-READY | forged readiness | `Serving` has private fields | W-17 | `E0451` |
| D-HOST-SPLIT | a host type named at the crate root (`sdax::CxInner`) | the author API is the root and `prelude`; adapters and the engine use `sdax::host` | S-01 | `E0432` |

### Validate — `PlanBuilder::build`

All findings are returned at once, as values, rule by rule in the order below
and within a rule in declaration order.

| id | decision procedure | test |
|---|---|---|
| `V-EMPTY` | a plan whose only nodes are its input and imports has nothing to run | P-10 |
| `V-FOREIGN-KEY` | for every node, every key in `needs ∪ exclusive ∪ shared ∪ spawns` and every pool it names belongs to this plan; an `import` node's source is exempt. A late `spawns(key, &t)` whose key is not a node of this plan is the same finding, reported after the per-node ones | P-01 |
| `V-DUP-NAME` | node names within one scope are unique | P-02 |
| `V-DUP-ATTR` | no attribute is set twice on one node. Locks are per key: several resources may be locked, but one resource twice — or once `exclusive` and once `shared` — is the same duplication | P-12 |
| `V-IMPORT-SCOPE` | every key a registered child plan imports is a node of the registering plan; a deeper nesting imports at each level | P-07 |
| `V-SPAWN-SELF-IMPORT` | for every service S and template T ∈ `spawns(S)`, no key T imports is S (F1, INV-17) | new |
| `V-SPAWN-KIND` | for every node N with a non-empty `spawns(N)`, N is a service. The chain form is typed; the late form `PlanBuilder::spawns(key, &t)` takes any key, so the kind is decided here | new |
| `V-LOCK-NEEDS` | `exclusive(k)`/`shared(k)` ⇒ `k ∈ needs(N)` | P-08 |
| `V-IDEMPOTENT-REQUIRED` | (`retry` on an effect) ∨ (`restart` on a service) ∨ (`on_ambiguous ∈ {Compensate, Retry}`) ⇒ `.idempotent()` | P-06 |
| `V-POOL-STARVE` | resident holders of a pool (services taking it; unbounded if a template's instances take it) ≥ its limit, while a non-resident node also takes it | P-09 |
| `V-UNUSED-POOL` | every pool has ≥ 1 user | P-10 |
| `V-SERVICE-UNBOUNDED` | `Shutdown::unbounded()` ⇒ every service declares `stop_within` | P-11 |
| `V-TRY-UNCONSUMED` | every `try_step` has ≥ 1 dependent | P-10 |
| `V-BUDGET-ORDER` | `stop_within(d) ≤ Shutdown::within(D)`; a child plan's budget ≤ its parent's | P-05 |
| `V-MODE` | `Mode::Finite` ⇒ no service and no template in this plan (F3, INV-19) | new |

### Load — before any spawn

`L-IMPORTS`: a plan with unresolved imports started as a root is refused with
the import list. `Plan::unresolved_imports()` is the decision procedure; Stage 2's
`start` calls it before any effect. It returns `Vec<NodePath>` — the import
nodes *of this plan*, named the way the report, the trace and `inspect()` name
every node. It cannot name the ancestor key behind each one: that key belongs to
a plan whose declaration is not in scope here.

### First run — signals a run produces *(Stage 2, except `RequestDuringCleanup`)*

`SpawnError::{ForeignTemplate, UndeclaredTemplate, ScopeStopping, NotRunning}`;
`FaultKind::DoubleHold`; `FaultKind::NeverReady`;
`TraceKind::{DroppedWhileRunning, RequestDuringCleanup, RuntimeDroppedWithLiveRuns}`.

Of these, Stage 1's machine emits `TraceKind::RequestDuringCleanup` (a
`shutdown()` or `cancel()` arriving while the run is already settling or
cleaning up — INV-7: it never interrupts a cleanup). The rest belong to the run
driver (`DoubleHold` from `CxInner::hold_count > 1`, `NeverReady`,
`DroppedWhileRunning`, `RuntimeDroppedWithLiveRuns`) or to `cx.spawn`
(`SpawnError`, Stage 3 except `NotRunning`, which the seam answers today).

---

## 8. Report order (INV-20)

`RecordOrder` is `(steps, attempt)` where `steps` is one `(declaration index,
instance)` pair per nesting level. Lexicographic comparison gives exactly:

1. root nodes in declaration order;
2. a component's or template's own record before its inner records;
3. an instance's records grouped, instances by ascending id (`None < Some`);
4. attempts of one node in attempt order.

`Report::sort()` establishes it. `Report::is_clean()` is false unless
`faults`, `cleanup_failures`, `incomplete` and `ambiguous` are all empty **and**
the outcome is `Ok`; `into_result()` is `Err` otherwise. `trace` is observation
order and `sort()` does not touch it.

---

## 9. Inspection

`Plan::inspect() -> PlanView` is pure: it reads the declaration, runs no body,
and performs no effect. It offers `nodes`, `edges` (exactly the declared ones),
`layers()` (earliest start, explicitly **not** barriers), `release_order()` (a
partial order exposing `before`, `unordered` and `unordered_pairs`, plus
`waves()` as a rendering), `why(node)`, `effects()`, `diff(&other)` and
`Display`. Inner nodes of a component or template are addressable by path
(`Net/Transport`).

`Plan::effects()` lists the nodes at or past the ship boundary — resources
(acquire), effects (perform) and persistent effects — in declaration order,
plus the earliest layer holding one.

Inspection is author API: `Plan`, `PlanView` and everything it is made of live
at the crate root and in `sdax::prelude`. Nothing in `sdax::host` is needed to
declare, validate or inspect a plan.

`Plan::simulate(&Script) -> Trace` *(Stage 1, implemented)*: the trace the pure
machine produces for a scripted schedule with no body run, so a counterfactual
is answered pre-ship with the same code that drives production.

---

## 10. Host contracts

**Where they live.** The crate has two surfaces. The **author** surface is the
crate root and `sdax::prelude`: what a plan is written, validated, inspected and
read back against. The **host** surface is `sdax::host`: what a runtime adapter,
a run driver or the engine needs and an author does not — the four traits below,
plus `Joined`, `NoObserver`, `BoxFuture`, `Time`, `Scope`, `ChildControl`,
`InstanceId`, `StopSignal`, `RawKey`, `SEMANTICS`, the driver-facing `CxInner`
(`take_held`, `put_output`, `take_serve`, `hold_count`, and `Cx::new` /
`Cx::inner`), and `sdax::host::engine::{Event, Effect, TimerId, JoinedLabel}`.

**Stability.** Only the author surface carries the crate's stability promise.
`sdax::host` may change in a minor version before 1.0: the run driver is in
`sdax-tokio` and is still to be written, so these signatures are still being
learned — Stage 1 already changed four of them (`Event::NodeErr` and
`Event::ServeEnded` carry their fault; `Effect::CancelTimer` and
`Effect::Reject` are new). A host item that stops resolving at the crate root is the point of the
split, and D-HOST-SPLIT witnesses it.

| trait | required operations |
|---|---|
| `Clock` | `now() -> Time`; `sleep(d) -> BoxFuture<'static, ()>` on *this* clock |
| `Runtime` | `spawn`, `spawn_blocking`, `clock`, `observer`; associated `Task: TaskHandle` |
| `TaskHandle` | `abort()` (a request; it lands between polls); `join() -> BoxFuture<'static, Joined>` |
| `Observer` | `event(&TraceEvent)`; must not block and must not panic |

Futures are boxed in every signature: the MSRV predates `async fn` in traits,
and these must stay `dyn`-usable. The shared `Clock` conformance suite is
`sdax_testkit::invariants::check_clock`.

---

## 11. Stages

| stage | delivers | gate |
|---|---|---|
| **0** *(done)* | surface, `validate`, `inspect`/`why`/`diff`/`effects`, the seam, report and engine types, host contracts, `FakeClock`/`TraceRecorder`/static checker, `TokioRuntime` | suite (a) W-*, suite (b) P-01…P-15 |
| **1** *(done)* | `engine::Machine` (T1–T8, retries, deadlines, policies, components), `Plan::simulate` and the stepping simulator, the testkit's scripted driver, `eol::Eol` and trace-level invariant checker, and a Monte Carlo suite over generated plans | suite (c) on the scripted driver: 41 of B's 46 `C-*` rows; `C-14` is Stage 2 and `C-30`, `C-51`, `C-64`, `C-65` are Stage 3. `S-02` did **not** run. `dev-docs/Stage1Report.md` § 7 |
| **2** *(done)* | the tokio run driver, `PlanStart::start`, `Running` + drop guard + drainer, blocking pools; `Bodies` moved under `sdax::host` (carrying components' bodies and import copies) so the driver in `sdax-tokio` can reach it; `C-14` | suite (c) re-run on the adapter with paused time — **58 rows green**, and the normalised traces equal the pure machine's; suite (d) `R-01`…`R-07` and `S-01`. `S-02` still did **not** run. `dev-docs/Stage2Report.md` |
| 3 | dynamic instances end to end: `cx.spawn`, `Child::ready`, containment; `Effect::SpawnInstance` and `Event::InstanceSpawned`/`InstanceEnded`, which the machine refuses today; `C-30`, `C-51`, `C-64`, `C-65` | C-30, P-07 |

---

## 12. Changes from Proposal B

| id | finding | change |
|---|---|---|
| **A1** | `Comparison.md` § 1.2(a) — declaration size | Positional constructors alongside the chain form, definitionally identical and witnessed by comparing `inspect()`. |
| **A2** | F-A11 / probe P5 — raw spawn is a silent escape | `clippy.toml` `disallowed-methods` for `tokio::{task::spawn, task::spawn_blocking, spawn}`, `tokio::runtime::Handle::{spawn, spawn_blocking}` and `std::thread::spawn`; the two correct call sites in `sdax-tokio` carry a scoped `#[allow]`. Plus `scripts/check-architecture.sh` over `cargo metadata`. |
| **A3** | `Comparison.md` § 1.2(c) — plan-level inspection | `Plan::effects()` (Stage 0). `Plan::simulate` is implemented in Stage 1. |
| **F1** | F-A4 / F-B4 — readiness after dynamic instances (I-34) | `spawns(&template)` on services, visible in `inspect()`; `Child::ready()`; `V-SPAWN-SELF-IMPORT`; INV-17. |
| **F2** | F-A3 / F-B3 — an effect with no compensation | `.persistent()` as the alternative to `.compensate(..)`; listed by `effects()`, shown as `effect (persistent)`; `on_ambiguous` still required; INV-18. |
| **F3** | F-B5 — the derived run mode | `Mode` is a required argument of `build`; `.resident()` and the structure-derived default are removed; `V-MODE`; INV-19. |
| **F4** | F-A12 / F-B12 — report order unstated | `RecordOrder` and `Report::sort()`; INV-20. |
| **F-A7** | no forced output join | Unchanged from B: a plan needs no output node, and `export` is optional. |
| **F-B1** | unbranded keys | Unchanged from B, and worded honestly: a foreign key is a **build-time** finding, not a compile error. Said so in `key.rs`, in the vocabulary table and in the gate inventory. |
| **F-B6** | a service's own acquisitions | Documented as a rule of the seam (§ 5) and in the crate docs. |

Spelling changes forced by Rust, not by design:

- `release::by_drop()` is recognised by the type of the future it returns, so
  `.release(release::by_drop())` reads exactly as B wrote it and still records
  `release: drop`. A trait-based alternative was not coherent.
- `TokioRuntime::with_observer` rather than `.observer(..)`: the `Runtime`
  contract already has an `observer(&self)` reader.
- `PlanBuilder::spawns(service_key, &template)` exists alongside the chain form
  `service(..).spawns(&t)`, for the case the chain form cannot express — a
  template whose `import` names a key declared after it. That is the only way
  `V-SPAWN-SELF-IMPORT` is reachable; see `dev-docs/Stage0Report.md`. Unlike the
  chain form it takes any key, so the declaration is recorded whatever it names
  and `build` decides: `V-SPAWN-KIND` for a node that is not a service,
  `V-FOREIGN-KEY` for a key of another plan. Neither is dropped silently.

---

## 13. Decisions

Questions this document once left open, and the answer. A decision here is
normative; the question it closes is annotated rather than deleted in
`dev-docs/Stage0Report.md` § 8, so the reasoning stays readable.

| id | question | decision | date | reason |
|---|---|---|---|---|
| **OD-2** | the seam's error type | `Error = Box<dyn std::error::Error + Send + Sync + 'static>`, fixed rather than generic | 2026-09-05 | `?` infers it inside a body with no annotation, which the W-14 witness exercises (`let _port: u16 = "9000".parse()?`); typed errors are not lost — they survive as `downcast_ref` targets on `FaultKind::Error`. A generic error parameter would spread through every terminal, every `Deps` impl and every host trait for a gain the seam does not need. |
| **OD-5** | `try_step`'s value type | `try_step(..).run(f) -> Key<Result<T, Error>>`; dependents receive `Arc<Result<T, Error>>` | 2026-09-05 | The failure is the value, so it must be the dependent's to read; `V-TRY-UNCONSUMED` already refuses the shape where nobody reads it. |
| **OD-MODE** | `Mode::Finite` and a nested `Resident` component | Allowed: `V-MODE` looks only at the plan's own nodes; the component becomes Ready when its inner run is steady, and its residency ends with the parent's cleanup (§ 2) | 2026-09-05 | A scope owns its own mode. The alternative — forcing a parent to be `Resident` because something inside it is — would make `Mode` a derived property, which F3 exists to remove. |
| **OD-IMPORTS** | what `Plan::unresolved_imports()` returns | `Vec<NodePath>`, naming this plan's import nodes | 2026-09-05 | `RawKey` is the engine's node address and is now host API; a refusal an author reads should name nodes the way the report, the trace and `inspect()` do. |
| **OD-PANIC-CANCELLED** | a body the engine had already cancelled panics: INV-9 says every panic the engine observes is in the report; INV-10 says an engine-interrupted node is never a fault | **INV-10 wins.** The node's terminal state is `Interrupted` (or `Ambiguous` for an effect that had not held), the panic is in the trace, and neither is a fault in the report | 2026-09-06 | The engine asked for the cancel; a body that panics on the way out of a cancel it was given is not a failure of the work the plan declared. INV-9's "the engine observes" is satisfied by the trace, which is where the panic is visible. Stage 1 found the two invariants in direct contradiction on this case (MC seed `13115684965286655053`); a machine fix was written and reverted in favour of this reading. `Ambiguous` is the same terminal observation for an effect interrupted before `hold` (INV-11), so the escape names both. |
| **OD-PERSIST-AMBIG** | `.persistent()` with `.on_ambiguous(Ambiguity::Compensate)`: `on_ambiguous` is required on every effect (INV-18), so the pair is declarable, and it asks for a compensation that does not exist | **Persistent wins.** Nothing runs for the effect at shutdown or on an ambiguity, and no cleanup failure can arise from it; the ambiguity is recorded and nothing is compensated | 2026-09-06 | INV-18 is unconditional: a persistent effect carries no compensation obligation. `Ambiguity` selects *among* the discharges an effect has, and a persistent effect has none, so there is nothing for `Compensate` to select. `build` accepts the pair today; refusing it with a new `V-*` rule would be a clearer surface and is an open question in `dev-docs/Stage1Report.md` § 9. |
| **OD-START** | `Plan::start` is named in this contract, but `sdax` has zero dependencies and `sdax-tokio` is the only crate allowed to spawn (A2), so the driver cannot be an inherent method | **An extension trait**: `sdax_tokio::PlanStart` gives `Plan<Out>` the methods `start`, `start_with`, `try_start` and `try_start_with`. The call site reads `use sdax_tokio::PlanStart;` then `plan.start(rt)` | 2026-09-06 | The alternative was a runtime-agnostic driver inside `sdax`, which would have needed a hand-rolled async channel and would have put the run loop in the crate whose whole point is that it executes nothing. The trait keeps the spelling the contract asks for, and `sdax::Start` — the service phase marker — keeps its name. `start` is total: a plan the machine refuses (`L-IMPORTS`, templates) yields a `Failed` report carrying the refusal, and `try_start` is the checked form |
| **OD-RT-SHUTDOWN** | Stage 0 left open whether `TokioRuntime::close_and_wait` should fold into a `shutdown` | **Folded.** `TokioRuntime::shutdown(budget) -> Result<(), usize>` is the one name; `close_and_wait` is gone | 2026-09-06 | There is one operation — stop accepting, wait, say how many are left — so there is one name. It is the *runtime's* shutdown, not a run's: a run ends by `Running::shutdown()`, by `cancel()`, or by being dropped |
| **OD-SERVE-ARM** | when does a service's serve future start: on the start body's `Ok`, or on the machine's `Ready`? | **On `Ready`.** The driver takes the serve future out of the body's context when the body returns, holds it, and spawns it only if the machine then says the node is `Ready`; otherwise it is dropped | 2026-09-06 | A start body that returns *after* the engine signalled it is `Interrupted`, not ready (T5). Arming on `Ok` alone left a serve task nobody owned (INV-15) and fed the machine a `ServeEnded` for a node that was not serving (D1). `R-05` found it on a multi-threaded runtime; `r05_regression_a_signalled_start_body_never_starts_its_serve_future` pins it |
| **OD-REPORT-OBSERVER** | a dropped `Running` has nobody to hand its report to, and INV-9 says nothing is lost | **`Observer::report(&Report<()>)`**, a default-no-op method on the host `Observer` trait, called on every run whether it was awaited or dropped | 2026-09-06 | `C-14` asks for the report of a dropped run to reach the observer, and the trace alone is not the report. A default method leaves an events-only observer untouched |
| **OD-BODY-SOURCE** | where the driver gets a node's code, given that a component's child plan has its own bodies and a child scope's `import` needs the ancestor's value in *its* slot table | **`host::BodySource`**, with `host::bodies_of(&plan)` as the plan's own. `Bodies` carries `children` (one per component) and an `ErasedImport` per import node, recorded where the type is still known | 2026-09-06 | `PlanBuilder::component` recorded only the child's *declaration*, so before this a component's inner nodes had no runnable bodies at all. The indirection also lets a harness run a `Script` on the real driver, which is what makes suite (c) a differential check rather than a second suite |
| **OD-INV8-CLOCK** | INV-8 says at most `Shutdown::within(d)` elapses from `Settling` to `End` **on the engine clock**; is that exact on a real clock? | **Exact only on an exact clock.** It is asserted under `start_paused`, where the engine clock is virtual; on a real clock a timer fires at or after its deadline, so the engine's own measurement always overshoots by the scheduler's jitter | 2026-09-06 | `R-05` compresses a real clock by 200×, which multiplies a 7 ms scheduling hiccup into 1.5 engine seconds. The invariant is about the engine not *waiting* longer than the budget, not about the substrate's timer resolution; the honest place to assert it is where the clock is exact |
| **OD-BACKSTOP** | a generated `Mode::Resident` plan whose services declare `Restart::on_error` with no `max` never ends by itself; is that a defect to fix or a plan to bound? | **Neither: the plan is correct and the *script* must end it.** The Monte Carlo generator appends a backstop `Request::Shutdown` at 20–25 s, well past the 0–15.5 s window its random requests are drawn from | 2026-09-06 | Unlimited restart on a resident plan is exactly what the author asked for, and an engine that stopped it anyway would be wrong. What was broken was the test: a case with no terminating request is not a hang in the machine, and a walk that treats it as one hides real hangs. The driver's liveness guard (1 000 steps) catches genuine non-termination separately. |
