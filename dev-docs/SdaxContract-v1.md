# `sdax/1` — the contract

Normative for the `sdax` crate. Semantics tag: **`sdax/1`** (`sdax::SEMANTICS`).
A change of meaning changes the tag.

Base: `sdax-v1/B/Proposal.md` (Designer B), adopted by
`sdax-v1/reviews/Comparison.md` § 1, with the adoptions and fixes in § 12 below.
Where this document and the proposal disagree, this document governs.

**Stage 0 scope.** This document states the whole contract. The crate today
implements the authoring surface, the validator, inspection, the seam and the
host contracts. It has no engine: no `Machine`, no `Plan::start`, no `Running`,
and no stub of them. Rows marked *(Stage 1)* or *(Stage 2)* are specified and
not implemented, and the crate says so where they are named.

---

## 1. Vocabulary

A **plan** is an immutable, reusable, `Send + Sync` value, built once and (from
Stage 1) started any number of times; each start is a **run** with its own
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
| `.spawns(&template)` | this service may instantiate this template (F1) | `V-SPAWN-SELF-IMPORT`, `V-FOREIGN-KEY` |
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
| `cx.spawn(&template, input)` | all phases | instantiate a declared template; refused while settling *(engine side: Stage 1)* |
| `child.ready()`, `child.stop()`, `child.id()` | on a `Child` | await an instance's readiness (INV-17); ask it to stop *(Stage 1)* |

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

### Validate — `PlanBuilder::build`

All findings are returned at once, as values, rule by rule in the order below
and within a rule in declaration order.

| id | decision procedure | test |
|---|---|---|
| `V-EMPTY` | a plan whose only nodes are its input and imports has nothing to run | P-10 |
| `V-FOREIGN-KEY` | for every node, every key in `needs ∪ exclusive ∪ shared ∪ spawns` and every pool it names belongs to this plan; an `import` node's source is exempt | P-01 |
| `V-DUP-NAME` | node names within one scope are unique | P-02 |
| `V-DUP-ATTR` | no attribute is set twice on one node | P-12 |
| `V-IMPORT-SCOPE` | every key a registered child plan imports is a node of the registering plan; a deeper nesting imports at each level | P-07 |
| `V-SPAWN-SELF-IMPORT` | for every service S and template T ∈ `spawns(S)`, no key T imports is S (F1, INV-17) | new |
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
the import list. `Plan::unresolved_imports()` is the decision procedure; Stage 1's
`start` calls it before any effect.

### First run — signals a run produces *(Stage 1)*

`SpawnError::{ForeignTemplate, UndeclaredTemplate, ScopeStopping, NotRunning}`;
`FaultKind::DoubleHold`; `FaultKind::NeverReady`;
`TraceKind::{DroppedWhileRunning, RequestDuringCleanup, RuntimeDroppedWithLiveRuns}`.

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

`Plan::simulate(&Script) -> Trace` *(Stage 1)*: the trace the pure machine
produces for a scripted schedule with no body run, so a counterfactual is
answered pre-ship with the same code that drives production. It is declared
here and deliberately not stubbed.

---

## 10. Host contracts

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
| 1 | `engine::Machine` (T1–T8, retries, deadlines, policies, components), `Plan::simulate`, the testkit's scripted driver and trace-level invariant checker | suite (c) C-01…C-41 on the scripted driver |
| 2 | the tokio run driver, `Plan::start`, `Running` + drop guard + drainer, blocking pools | suite (c) re-run on the adapter with paused time; suite (d) R-* |
| 3 | dynamic instances end to end: `cx.spawn`, `Child::ready`, containment | C-30, P-07 |

---

## 12. Changes from Proposal B

| id | finding | change |
|---|---|---|
| **A1** | `Comparison.md` § 1.2(a) — declaration size | Positional constructors alongside the chain form, definitionally identical and witnessed by comparing `inspect()`. |
| **A2** | F-A11 / probe P5 — raw spawn is a silent escape | `clippy.toml` `disallowed-methods` for `tokio::{task::spawn, task::spawn_blocking, spawn}`, `tokio::runtime::Handle::{spawn, spawn_blocking}` and `std::thread::spawn`; the two correct call sites in `sdax-tokio` carry a scoped `#[allow]`. Plus `scripts/check-architecture.sh` over `cargo metadata`. |
| **A3** | `Comparison.md` § 1.2(c) — plan-level inspection | `Plan::effects()` (Stage 0). `Plan::simulate` is declared for Stage 1 and not stubbed. |
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
  `V-SPAWN-SELF-IMPORT` is reachable; see `dev-docs/Stage0Report.md`.
