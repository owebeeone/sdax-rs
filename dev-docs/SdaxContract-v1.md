# `sdax/1` — the contract

Normative for the `sdax` crate. Semantics tag: **`sdax/1`** (`sdax::SEMANTICS`).
A change of meaning changes the tag.

Base: `sdax-v1/B/Proposal.md` (Designer B), adopted by
`sdax-v1/reviews/Comparison.md` § 1, with the adoptions and fixes in § 12 below.
Where this document and the proposal disagree, this document governs.

**Implementation scope.** This document states the whole contract, and from
Stage 3 the crate implements all of it: the authoring surface, the validator,
inspection, the seam, the host contracts, `host::engine::Machine` (T1–T8,
components **and** template instances), `Plan::simulate`, the tokio run driver
(`plan.start(rt)`, `Running` with its drop guard and drainer, blocking pools),
and `cx.spawn` with `Child::{ready, stop, id}`. The driver is in `sdax-tokio`,
because that is the only crate allowed to spawn (A2); `start` is the extension
trait `sdax_tokio::PlanStart` for the same reason (OD-START).

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
| | *a panic or a `within` timeout is a **fault**, not a value: only a returned `Err` is the value* | | | | |
| `blocking_step` | `run(cx, deps) -> Result<T>`, synchronous, on a declared pool | returns `Ok` | `Arc<T>` | never | none |
| | *never aborted: signalled at the settle (T5), and its `within` faults the attempt while the thread keeps its grants until it returns (T7)* | | | | |
| `service` | `start(cx, deps) -> Result<Serving<H>>` | the start body returns `Ok(Serving)` | `Arc<H>` | started (a serve future exists) **and** the start body returned before the stop request — a `Serving` returned after the signal is the answer to the cancel, is `Interrupted{held: false}`, and is dropped unpolled | signal, wait ≤ `stop_within`, abort |
| `effect` | `perform(cx, deps) -> Result<Held<R>>` | the body returns `Ok` | `Arc<R>` (the receipt) | `hold` registered a receipt **and** the effect is not `persistent` | `compensate(cx, Arc<R>)`, reported distinctly from a release |
| `join` | none | every need is Ready | `Arc<()>` | never | none |
| `component` | none (a nested plan) | the inner run reaches steady state | `Arc<Out>` | the inner run started | the inner release graph, as one unit |
| `template` | none (a nested plan factory) | n/a | none | live instances exist | stop every live instance |

Edges and constraints:

| construct | meaning | checked by |
|---|---|---|
| `.needs(deps)` | ordering **and** typed dataflow; the body receives `Arc<T>` per key | compiler (existence, type), `V-FOREIGN-KEY` |
| `import(parent_key)` | a cross-scope need in a child plan; the parent's node is released only after the child is cleaned up | `V-IMPORT-SCOPE` |
| `.exclusive(k)` / `.shared(k)` | arbitration on a resource the node already needs. The grant is held **from the start of the prepare/run body to its `Ready`, fault or interrupt** — a service keeps it while it serves; release and compensate bodies, including the between-attempts release of a retried resource, run unlocked. `k` may be an `import`, and then the lock is one lock shared by both scopes | `V-LOCK-NEEDS` |
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
| `Waiting{on}` | a need, a lock or a pool grant is missing; `on` lists each reason, and is never empty — a node refused only because an earlier waiter for the same grant has not been served names that waiter (`Reason::QueuedBehind`) |
| `Running{attempt, held}` | the prepare body is in flight; `held` flips at `Held` |
| `Backoff{attempt, until}` | between failed attempts |
| `Ready` | see § 1; a service is also `Serving` → `Finished` \| `Faulted` |
| `Failed{fault, held}` | attempts exhausted with `Err`/panic/timeout |
| `Interrupted{held}` | the engine cancelled the body; never a fault |
| `Ambiguous` | an effect interrupted or timed out after start and before `hold` |
| `Skipped{because}` | a need ended `Failed`/`Interrupted`/`Ambiguous`/`Skipped`, or the scope stopped admitting. `because` names the node whose outcome did it, and is `None` when the run itself ended the node's eligibility — a request, a `Finite` scope reaching steady state, a `terminal` service finishing. The trace event is emitted either way |
| `Releasing`/`Stopping`/`Compensating` → `Released`/`Stopped`/`Compensated` \| `ReleaseFailed{fault}` \| `Abandoned` | discharging the obligation; `Abandoned` = the budget expired |

**Run states**: `Planned` → `Admitting` → `Steady` → `Settling` → `Cleanup` →
`Ended(Report)`. Transitions into `Settling`: `shutdown()`; `cancel()` or drop
of `Running`; a fault under `FailFast`; a `terminal` service **finishing** —
its serve future returning `Ok`, which a serve that returned `Err` is not, and
that is a fault under the scope's policy like any other; or,
under `Mode::Finite`, reaching `Steady`.

`Mode` is per scope, and a nested scope has its own. A `Finite` parent may
contain a `Resident` component; the component becomes Ready when its inner run
is steady and its residency ends with the parent's cleanup. (Owner decision,
2026-09-05.) A **child** plan's `Mode` is a declaration only: `V-MODE` reads it,
and at run time an inner scope stays `Steady` whatever its mode until the
parent's cleanup opens it. A child plan's `Policy`, by contrast, does govern its
own scope at run time (`OD-INNER-POLICY`).

## 3. Transitions

| id | rule |
|---|---|
| T1 | *start*: `Waiting → Running` iff every key in `needs ∪ imports` is `Ready` **and** the node's lock and pool grants are taken atomically (all or none, in a fixed global order, FIFO among waiters **for that grant**). A waiter is never held behind a waiter for a *different* lock or pool. Nothing else is required: no wave, no level, no sibling. |
| T2 | *hold*: on `Held(N)` the engine records the `Arc<T>` and the obligation, in the poll that observes the effect completing, before the body's continuation can be polled again. |
| T3 | *ready*: on the body's `Ok`, `Running → Ready`; dependents re-evaluate T1. |
| T4 | *fail*: on `Err`/panic/timeout, `Backoff` if attempts remain (and, if `held`, this attempt's release finishes first), else `Failed`. Under `FailFast` the scope enters `Settling`; under `Isolate` transitive dependents become `Skipped`. |
| T5 | *interrupt*: in `Settling`, every `Running` non-service node is cancelled per its cancel mode (`drop`: abort; `cooperative(g)`: signal, wait `g`, abort); the engine **joins** the aborted task before the node counts as settled. Result `Interrupted{held}`, or `Ambiguous` for an effect not yet held. A **blocking** body is signalled and never aborted — a thread cannot be dropped — so `cx.is_stopping()` is how it learns, and only the budget (T7) bounds the wait; `cooperative` on one is refused at `build` (`V-BLOCKING-CANCEL`) because its grace could never be spent. A component's inner scope settles with it, and nothing starts inside a settled inner scope. |
| T6 | *cleanup order*: a node's release starts iff every node that needs or imports it — and every live instance of a template that imports it — has finished its cleanup (released, failed, or abandoned). Unrelated nodes run concurrently. |
| T7 | *bound*: the **root**'s shutdown budget starts at the transition into `Settling`; on expiry everything still running is aborted (services after their own `stop_within`, if shorter) and recorded `Abandoned`. A blocking body cannot be aborted; it is `Abandoned` while its thread finishes. |
| T7a | *nested bound*: a **nested** scope's budget starts when its release graph may open (T6) — not when the inner scope settles, which INV-5 can precede by the parent's whole life — and is capped by the parent's remaining budget, so it never outlives it (`V-BUDGET-ORDER`). Until then the inner scope is bounded by the parent's deadline alone. |
| T7b | *post-budget releases*: after the budget the remaining gated releases are **started in order to be abandoned**: a driver sees `Release(k)` immediately followed by `Abort(k)`, and the node is `Abandoned` and listed in `incomplete`. |
| T7c | *a blocking `within`*: the deadline records a `Timeout` fault for the attempt at once, but the thread keeps its grants and the node stays `Running` until it reports; only then are the grants released and the next attempt started. Attempts therefore never overlap (INV-12), the pool is never over-subscribed, and a thread that never returns is `Abandoned` at the budget like any other. |
| T7d | *what abandonment does not do*: `Abandoned` records that the engine stopped waiting; it does not stop the work. A blocking body that never returns keeps its thread for the life of the process, and on tokio it also blocks `tokio::runtime::Runtime::drop`, which waits for pool work without a budget — so a supervisor that drops its runtime after such a run hangs there, while `TokioRuntime::shutdown(budget)`, `shutdown_timeout` and `shutdown_background` all answer. `Abandoned` plus `incomplete` is the engine's honest report of exactly this. |
| T8 | *end*: `Ended(Report)` when every obligation is discharged or abandoned and every spawned task is joined or recorded. |

## 4. Invariants

INV-1…16 are Proposal B's, unchanged. INV-17…20 are this contract's.

| id | invariant |
|---|---|
| INV-1 | **Declared edges only.** N starts only after every key in `needs(N) ∪ imports(N)` is Ready; the engine adds no other start ordering except lock and pool arbitration among nodes that declare the same lock or pool — and a lock named through an `import` is the same lock in both scopes, so releasing it re-admits every scope waiting for it. |
| INV-2 | **Readiness is a return.** `Ready(N)` is recorded only when N's prepare body returned `Ok`. Being spawned never implies readiness. |
| INV-3 | **Held ⇒ released.** If `Held(N)` occurred, exactly one release or compensation attempt for N occurs before `End`, unless the budget expires first, in which case `N ∈ incomplete`. |
| INV-4 | **Not held ⇒ no release body.** If `Held(N)` did not occur, no release body runs for N. |
| INV-5 | **Alive-during.** For every M with N ∈ `needs(M) ∪ imports(M)`, the release of N does not start before M's cleanup has ended. |
| INV-6 | **Cleanup concurrency is explicit.** Releases unrelated by INV-5 may overlap; `inspect()` marks them unordered; no sequence is promised. |
| INV-7 | **Shielding.** A cleanup body is never dropped because of a cancel or shutdown request; only the budget can abandon it, and abandonment is recorded. |
| INV-8 | **Bounded shutdown.** From `Settling` to `End`, at most `Shutdown::within(d)` elapses on the engine clock (unless `unbounded()`). |
| INV-9 | **No silent loss.** Every fault, cleanup failure, panic, timeout, abandonment and ambiguity the engine observes is in the `Report` — except a panic in a body the engine had already cancelled, which is in the **trace** and is not a fault (`OD-PANIC-CANCELLED`), and so is recorded nowhere when no observer is attached. `into_result()` is `Err` unless the report is clean. |
| INV-10 | **Cancelled ≠ failed.** Engine-interrupted nodes are `Interrupted`, never faults; `Outcome::Cancelled` is reported only for an external `cancel()`/drop before `End`. |
| INV-11 | **Ambiguity.** An effect interrupted after start and before `hold` is `Ambiguous`; it is neither retried nor compensated unless it is `idempotent` with the matching `Ambiguity`. Under `Ambiguity::Retry` the record travels with the attempt's faults: a retry that reaches `Ready` absorbs it and the run can still be clean, while the trace keeps the `Ambiguous`. |
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
| `cx.spawn(&template, input)` | all phases | instantiate a declared template; refused with `ForeignTemplate`, `UndeclaredTemplate` or `ScopeStopping`. `input` is `Send + Sync` (OD-SPAWN-INPUT) |
| `child.ready()`, `child.stop()`, `child.id()` | on a `Child` | await an instance's readiness (INV-17); ask it to stop |

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
| `within` | none; the shutdown budget still bounds cleanup. When declared, the deadline is a **hard abort** whatever the node's cancel mode: a `cooperative` node gets its grace on `cancel()`/`shutdown()`, never at its own deadline, and a blocking node's deadline follows T7c instead. A cleanup body has no `within`, and a *mid-run* one — the between-attempts release of a retried resource — has no budget either until the scope settles (INV-7), so a hung one holds a `Finite` run at `Admitting` until a request arrives | `within —` |
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
| `V-BLOCKING-CANCEL` | a `blocking_step` does not declare `cooperative(g)`: it is signalled and never aborted (T5), so the grace can never be spent and `cx.is_stopping()` works without it | new |
| `V-PERSIST-AMBIG` | `release == persistent` ⇒ `on_ambiguous ≠ Compensate`: a persistent effect has nothing to compensate, and the pair also forces a meaningless `.idempotent()` through `V-IDEMPOTENT-REQUIRED` | new |

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
(`SpawnError`; `NotRunning` is the seam's answer when no run is attached).

---

## 8. Report order (INV-20)

`RecordOrder` is `(steps, attempt)` where `steps` is one `(declaration index,
instance)` pair per nesting level. Lexicographic comparison gives exactly:

1. root nodes in declaration order;
2. a component's or template's own record before its inner records;
3. an instance's records grouped, instances by ascending id (`None < Some`);
4. attempts of one node in attempt order.

`Outcome::Ok` means only that every node that started settled without a fault:
cleanup failures, abandonments and ambiguities do not move it, so a trace can
end `End(Ok)` while the report is unclean. Read `is_clean()`/`into_result()`.

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
(`take_held`, `put_output`, `take_serve`, `hold_count`, `spawn_instance`, and
`Cx::new` / `Cx::inner`), and
`sdax::host::engine::{Event, Effect, TimerId, JoinedLabel, SpawnTable}`.

**Stability.** Only the author surface carries the crate's stability promise.
`sdax::host` may change in a minor version before 1.0: these signatures are
still being learned — Stage 1 changed four of them (`Event::NodeErr` and
`Event::ServeEnded` carry their fault; `Effect::CancelTimer` and
`Effect::Reject` are new), and Stage 3 changed six more for instances (every
`BodySource` method takes an `Option<InstanceId>` and two are new;
`Event::InstanceSpawned` carries the spawner; `Event::InstanceEnded` became
`Event::StopInstance`, `OD-INSTANCE-EVENTS`; `Effect::SpawnInstance` carries
the parent instance; `EngineError::Templates` became
`EngineError::TemplateAsScope`). A host item that stops resolving at the crate
root is the point of the split, and D-HOST-SPLIT witnesses it.

| trait | required operations |
|---|---|
| `Clock` | `now() -> Time`; `sleep(d) -> BoxFuture<'static, ()>` on *this* clock |
| `Runtime` | `spawn`, `spawn_blocking`, `clock`, `observer`; associated `Task: TaskHandle` |
| `TaskHandle` | `abort()` (a request; it lands between polls); `join() -> BoxFuture<'static, Joined>` |
| `Observer` | `event(&TraceEvent)`, `report(&Report<()>)`; must not block and must not panic — and a driver contains a panic anyway (obligation 6) |

**What a run driver owes the machine.** Beyond performing the effects in order:

1. A body event (`Started`, `Held`, `NodeOk`, `NodeErr`, `NodeCancelled`,
   `TaskJoined`) is delivered only for a key the machine asked it to spawn. A
   `component` and a `join` have **no body**; the machine refuses a body event
   for one (`Effect::Reject`), and a driver that synthesised one — from a
   per-node `NeverReady` check, say, or from an inner report — would give a
   component a fault vector the engine's exit helpers assume is always empty.
2. An outcome already due when an `Abort` arrives is delivered; a later one is
   dropped and the join reports the cancellation.
3. A superseded attempt's outcome is the driver's to drop: the machine credits
   whatever arrives to the attempt in flight.
4. A cleanup body is never aborted by the driver (INV-7). The machine refuses a
   `NodeCancelled` for a node that is `Releasing`, `Compensating` or `Stopping`.
5. `cx.spawn` is answered from the `SpawnTable` the driver publishes after
   every step, not from the driver's own opinion, so both drivers refuse
   alike. The instance's slots are opened by `Effect::SpawnInstance` **before**
   the effects that spawn its bodies, and dropped when the trace says the
   instance ended. A `Child` handed out before a settle is still valid: the
   machine ends that instance at once, so `Child::ready()` answers.
6. **A run never ends in silence, and nothing outside the engine may end it.**
   Three cases, none of which the machine can see:
   - An `Observer` callback that panics is caught, recorded as
     `TraceKind::ObserverPanicked`, and stepped over. The callbacks run on the
     task that owns every live body; an unguarded panic there detaches all of
     them, which is INV-15's failure mode reached without a word.
   - An `Effect::Reject` is a driver bug and reaches the run's own record of
     itself: `TraceKind::Rejected(what)`, so it is in the trace the report
     carries and in whatever the observer is doing with events. Writing it to
     stderr is not telling anyone.
   - A driver whose own future is dropped before `End` — the runtime torn down
     under a live run or a pending drainer — reports what it had: a
     `TraceKind::RuntimeDroppedWithLiveRuns` and a `Cancelled` report, and it
     latches readiness so nothing waits on a run that no longer exists.

**What the substrate adds.** Two facts about tokio that these rules do not
imply, stated here because they are what a supervisor gets wrong: a
never-returning blocking body blocks `Runtime::drop` for ever (T7d), and a
drainer left by a dropped `Running` is a task, so on a `current_thread`
runtime it makes no progress between `block_on` calls. A third is about the
build, not the runtime: every `catch_unwind` in this workspace — the body
wrapper, the observer boundary — is inert under `panic = "abort"`, where a
body panic aborts the process and `FaultKind::Panic` is unreachable. Nothing
here sets that profile and nothing here can detect it.

Futures are boxed in every signature: the MSRV predates `async fn` in traits,
and these must stay `dyn`-usable. The shared `Clock` conformance suite is
`sdax_testkit::invariants::check_clock`.

---

## 11. Stages

| stage | delivers | gate |
|---|---|---|
| **0** *(done)* | surface, `validate`, `inspect`/`why`/`diff`/`effects`, the seam, report and engine types, host contracts, `FakeClock`/`TraceRecorder`/static checker, `TokioRuntime` | suite (a) W-*, suite (b) P-01…P-15 |
| **1** *(done)* | `engine::Machine` (T1–T8, retries, deadlines, policies, components), `Plan::simulate` and the stepping simulator, the testkit's scripted driver, `eol::Eol` and trace-level invariant checker, and a Monte Carlo suite over generated plans | suite (c) on the scripted driver: 41 of B's 46 `C-*` rows; `C-14` is Stage 2 and `C-30`, `C-51`, `C-64`, `C-65` are Stage 3. `S-02` did **not** run. `dev-docs/Stage1Report.md` § 7 |
| **2** *(done)* | the tokio run driver, `PlanStart::start`, `Running` + drop guard + drainer, blocking pools; `Bodies` moved under `sdax::host` (carrying components' bodies and import copies) so the driver in `sdax-tokio` can reach it; `C-14` | suite (c) re-run on the adapter with paused time — **72 rows green** (58 at the Stage 2 gate, plus the Stage 1 review's remediation rows), and the normalised traces equal the pure machine's; suite (d) `R-01`…`R-07` and `S-01`. `S-02` still did **not** run. `dev-docs/Stage2Report.md` |
| **3** *(done)* | dynamic instances end to end: `cx.spawn`, `Child::{ready, stop, id}`, per-instance scopes and slot tables, INV-16 containment, `Effect::SpawnInstance` and `Event::InstanceSpawned`/`StopInstance`; the instance-aware invariant checker and a Monte Carlo walk over templates | suite (c) on both drivers — **74 rows on the scripted driver, 78 on the adapter** — including `C-30`, `C-51`, `C-64`, `C-65`, `C-14`'s INV-16 half and `I-34`; 200 000 pure and 20 000 adapter Monte Carlo cases. `dev-docs/Stage3Report.md` |

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
| **OD-PERSIST-AMBIG** | `.persistent()` with `.on_ambiguous(Ambiguity::Compensate)`: `on_ambiguous` is required on every effect (INV-18), so the pair is declarable, and it asks for a compensation that does not exist | **Persistent wins.** Nothing runs for the effect at shutdown or on an ambiguity, and no cleanup failure can arise from it; the ambiguity is recorded and nothing is compensated | 2026-09-06 | INV-18 is unconditional: a persistent effect carries no compensation obligation. `Ambiguity` selects *among* the discharges an effect has, and a persistent effect has none, so there is nothing for `Compensate` to select. The open question — whether to refuse the pair outright — is now closed **yes**: `V-PERSIST-AMBIG` (2026-09-06) refuses it at `build`, so the run-time reading below applies only to a plan built before that rule existed. |
| **OD-START** | `Plan::start` is named in this contract, but `sdax` has zero dependencies and `sdax-tokio` is the only crate allowed to spawn (A2), so the driver cannot be an inherent method | **An extension trait**: `sdax_tokio::PlanStart` gives `Plan<Out>` the methods `start`, `start_with`, `try_start` and `try_start_with`. The call site reads `use sdax_tokio::PlanStart;` then `plan.start(rt)` | 2026-09-06 | The alternative was a runtime-agnostic driver inside `sdax`, which would have needed a hand-rolled async channel and would have put the run loop in the crate whose whole point is that it executes nothing. The trait keeps the spelling the contract asks for, and `sdax::Start` — the service phase marker — keeps its name. `start` is total: a plan the machine refuses (`L-IMPORTS`, templates) yields a `Failed` report carrying the refusal, and `try_start` is the checked form |
| **OD-RT-SHUTDOWN** | Stage 0 left open whether `TokioRuntime::close_and_wait` should fold into a `shutdown` | **Folded.** `TokioRuntime::shutdown(budget) -> Result<(), usize>` is the one name; `close_and_wait` is gone | 2026-09-06 | There is one operation — stop accepting, wait, say how many are left — so there is one name. It is the *runtime's* shutdown, not a run's: a run ends by `Running::shutdown()`, by `cancel()`, or by being dropped |
| **OD-SERVE-ARM** | when does a service's serve future start: on the start body's `Ok`, or on the machine's `Ready`? | **On `Ready`.** The driver takes the serve future out of the body's context when the body returns, holds it, and spawns it only if the machine then says the node is `Ready`; otherwise it is dropped | 2026-09-06 | A start body that returns *after* the engine signalled it is `Interrupted`, not ready (T5). Arming on `Ok` alone left a serve task nobody owned (INV-15) and fed the machine a `ServeEnded` for a node that was not serving (D1). `R-05` found it on a multi-threaded runtime; `r05_regression_a_signalled_start_body_never_starts_its_serve_future` pins it |
| **OD-REPORT-OBSERVER** | a dropped `Running` has nobody to hand its report to, and INV-9 says nothing is lost | **`Observer::report(&Report<()>)`**, a default-no-op method on the host `Observer` trait, called on every run **that was launched**, whether it was then awaited or dropped | 2026-09-06 | `C-14` asks for the report of a dropped run to reach the observer, and the trace alone is not the report. A default method leaves an events-only observer untouched. *Amended 2026-09-06 (substrate review S-14): "every run" read as "every handle", and a `Running` dropped before its first poll produces neither report nor event. That is `C-11` — `start` is lazy and an unlaunched handle is not a run — so the wording is the thing that was wrong, not the behaviour* |
| **OD-BODY-SOURCE** | where the driver gets a node's code, given that a component's child plan has its own bodies and a child scope's `import` needs the ancestor's value in *its* slot table | **`host::BodySource`**, with `host::bodies_of(&plan)` as the plan's own. `Bodies` carries `children` (one per component) and an `ErasedImport` per import node, recorded where the type is still known | 2026-09-06 | `PlanBuilder::component` recorded only the child's *declaration*, so before this a component's inner nodes had no runnable bodies at all. The indirection also lets a harness run a `Script` on the real driver, which is what makes suite (c) a differential check rather than a second suite |
| **OD-INV8-CLOCK** | INV-8 says at most `Shutdown::within(d)` elapses from `Settling` to `End` **on the engine clock**; is that exact on a real clock? | **Exact on an exact clock; asserted with measured slack on a real one.** Under `start_paused` the engine clock is virtual and the bound is exact. On a real clock a timer fires at or after its deadline, so the engine's own measurement overshoots by the scheduler's jitter — which the compression multiplies — and the bound is asserted against `budget + slack`, with `slack` a measurement of this substrate rather than a taste | 2026-09-06 | `R-05` compresses a real clock by 200×, which multiplies an 11 ms scheduling hiccup into 2.2 engine seconds — larger than some of the budgets in play, so at ×200 INV-8 is **absent**, not tolerant. *Amended 2026-09-06 (substrate review S-11): "assert nothing" was more than the measurement supported. `check_report_with_slack` takes the allowance as a parameter (`Duration::ZERO` everywhere the clock is exact), and `r05_the_shutdown_bound_holds_on_two_worker_threads_with_measured_slack` asserts INV-8 on two worker threads at ×20 with 0.5 engine seconds of slack — ×12 the 40 ms overshoot measured with the allowance set to zero, and a sixth of the smallest budget, so a driver that waited even 3.5 seconds against a 3-second budget fails. 0 failures in 30 runs (180 program executions)* |
| **OD-INNER-POLICY** | a node inside a component faults: does the child plan's declared `Policy` govern, or does any inner fault fail the component? | **The child's policy governs its own scope.** Under a child `FailFast` an inner fault settles the inner scope and faults the component, as before. Under a child `Isolate` the fault skips the failed node's dependents and the inner run continues; the component faults only when the export the parent waits for can no longer arrive — the export itself failed, or was skipped as a dependent of what failed. A child that exports nothing is `Ready` on its inner steady state. The faults are in the report either way | 2026-09-06 | § 1 says a plan is a scope whose nodes "share one fail policy", and `Policy` is one of three arguments to a child's `build`. The machine used to override it: `component_faulted` settled the inner scope on the first inner fault whatever the child declared, so an `Isolate` child's completed work was thrown away and its declaration meant nothing at run time. With `Mode` already inert for a child scope (§ 2), documenting a second argument as inert would have made `build`'s signature a claim the engine does not keep. Found by the Stage 1 semantics review, F-03 |
| **OD-NESTED-BUDGET** | when does a nested scope's shutdown budget start? T7 was written for the root and said nothing about a component | **When its release graph may open (T6)**, capped by the parent's remaining budget; not when the inner scope settles | 2026-09-06 | An inner scope settles by itself in three ways that do not involve the parent — a `terminal` service finishing, an inner fault under an inner `FailFast`, a `component_faulted` — and INV-5 can hold its releases shut for the parent's whole life afterwards. Arming the clock at the settle spent the entire budget waiting for a gate, and when the parent's cleanup finally reached the component every inner release was emitted and abandoned in the same instant with the parent's budget untouched. `V-BUDGET-ORDER` promises the child's budget nests in the parent's; it now does in time as well as in size. Found by the Stage 1 semantics review, F-02 |
| **OD-BLOCK-WITHIN** | `within` expires on a blocking step whose thread cannot be aborted: free the pool grant and retry beside the thread, or hold the grant until it returns? | **Hold it.** The deadline records the `Timeout` fault at once; the slot stays `Running` and keeps its grants until the thread reports, and only then is the next attempt started (T7c) | 2026-09-06 | The alternative had to be paid for by weakening two invariants: INV-12 ("a retried node's attempts never overlap") and INV-15 ("every task the engine spawned has been joined or is listed as abandoned") were both false for a timed-out blocking attempt, and a declared `pool(cpu, 1)` ran two threads — measured at 2 by `r05_a_blocking_within_does_not_oversubscribe_its_pool` before the fix. A thread cannot be taken back, so the honest reading of `within` on a blocking step is "fault now, retry when the thread is back". Found by the Stage 1 semantics review, F-05 |
| **OD-BLOCK-SIGNAL** | a blocking body cannot be aborted; is it told to stop at all? | **Yes: signalled at the settle, always, whatever its cancel mode**, and never aborted. `cooperative(g)` on a blocking step is refused at `build` (`V-BLOCKING-CANCEL`) | 2026-09-06 | § 5 lists `cx.is_stopping()` as available in all phases and T5 says every `Running` non-service node is cancelled per its cancel mode. A blocking body has a `Cx` and could poll `is_stopping()`, and it read `false` until the run ended: the request never reached it, and `.cooperative(g)` was accepted and ignored. Signalling unconditionally makes `is_stopping()` mean the same thing everywhere; the grace has nothing to bound, because no abort follows, so the attribute is refused rather than left as a no-op. Found by the Stage 1 semantics review, F-04 |
| **OD-BACKSTOP** | a generated `Mode::Resident` plan whose services declare `Restart::on_error` with no `max` never ends by itself; is that a defect to fix or a plan to bound? | **Neither: the plan is correct and the *script* must end it.** The Monte Carlo generator appends a backstop `Request::Shutdown` at 20–25 s, well past the 0–15.5 s window its random requests are drawn from | 2026-09-06 | Unlimited restart on a resident plan is exactly what the author asked for, and an engine that stopped it anyway would be wrong. What was broken was the test: a case with no terminating request is not a hang in the machine, and a walk that treats it as one hides real hangs. The driver's liveness guard (1 000 steps) catches genuine non-termination separately. |
| **OD-INSTANCE-EVENTS** | `Event::InstanceEnded` was in the vocabulary: is "the instance ended" something the host tells the machine? | **No.** The machine owns an instance's lifecycle, so its end is the machine's own observation (`TraceKind::InstanceEnded`) and never an input. The event the host needs is the *request*: `Event::StopInstance(id)`, which is what `Child::stop()` sends. `Event::InstanceSpawned` gains the **spawner**, so the template handle is resolved in the scope that declared it | 2026-09-06 | A host that could say "this instance ended" could end one whose release graph was still running, which is the one thing INV-16 exists to prevent. The spawner is needed because a `Template` handle carries a *declaration* key: a template declared inside a template's plan has one node per instance, and only the spawning node says which |
| **OD-SPAWN-INPUT** | `Cx::spawn<I>` required `I: Send`; the input has to live somewhere every body of the instance can read | **`I: Send + Sync`**, and `Scope::spawn_instance` takes `Box<dyn Any + Send + Sync>` | 2026-09-06 | The per-instance input is stored in the instance's own slot table like any other node's value, and a slot holds `Arc<T>` shared across the instance's bodies. `Plan::template::<In>` already requires `In: Send + Sync`, and `Template<I>` has no other constructor, so no handle that can be built is excluded — the bound was simply missing from the call site |
| **OD-SPAWN-EARLY** | may a body instantiate a template whose own `import`s are not `Ready` yet? | **Yes.** The instance is created and its nodes wait under T1 like any other node; only a settling scope (`ScopeStopping`), an undeclared template or a foreign handle is refused | 2026-09-06 | The alternative is a fourth refusal for a state that resolves itself, and it would make `cx.spawn` depend on an ordering the author did not declare: `V-SPAWN-SELF-IMPORT` already rules out the one case that could never resolve. A template that admitted an instance before its own imports were ready is `Live` from that moment, so its obligation — stopping that instance — is owed however the node itself ends |
| **OD-INSTANCE-FAULT** | does a fault inside an instance fail the parent? | **No.** The instance's own `Policy` governs its scope; the instance ends `Failed`, its faults are in the run's report in F4 order, and the parent is untouched | 2026-09-06 | § 1 gives a template's obligation as "stop every live instance" and nothing else, and INV-16 forbids a parent node to name an instance node — so there is no edge along which the fault could travel. A component is the opposite case and says so (`OD-INNER-POLICY`): a component *is* one node of the parent, and its export is what the parent waits for. "Unless the declaration says so" has no spelling today; a `spawns(..).propagate()` attribute is the shape it would take |
| **OD-OBSERVER-PANIC** | § 10 forbids an `Observer` to panic. What does a driver do when one does anyway? | **Contain it.** Every observer callback runs under a panic boundary; a panic is recorded as `TraceKind::ObserverPanicked` in the trace and the run continues. The obligation stands — the observer's copy of that event is lost — but the run is not | 2026-09-06 | The callbacks run on the driver task, which owns every live body's `JoinHandle`. An unguarded panic unwound it: dropping a `JoinHandle` detaches rather than aborts, so a serving service was never signalled, no release ran, no report was produced, `ready()` waited on a latch nobody would set, and the awaiter was handed an empty `Cancelled`. That is precisely INV-15's failure mode, reached in silence by a defect the contract had already assigned to someone else. A user obligation is a reason to report a violation, not a reason to lose the run. Found by the substrate review, S-02/S-05 |
| **OD-REJECT-VISIBLE** | a machine `Reject` is "a driver bug: log it loudly" (D1). Loudly where? | **In the trace**: `TraceKind::Rejected(what)`, so it reaches the observer and travels with the report, in addition to the `RunRecord` a harness attaches and the stderr line when neither is listening | 2026-09-06 | The rejection was visible only to a test harness or to whoever was reading a library's stderr. A production consumer — the observer, the report — saw a run that looked clean, which is the opposite of loud. The trace is the run's own record of itself and is where every other thing the engine noticed already lives. Found by the substrate review, S-12 |
| **OD-DRIVER-DROPPED** | the tokio runtime is torn down under a live run or a pending drainer: the driver's future is dropped and no `End` is ever reached | **The driver reports on its way out**: `TraceKind::RuntimeDroppedWithLiveRuns`, a `Cancelled` report with the trace so far, and the readiness latch closed | 2026-09-06 | `TokioRuntime`'s own `Drop` covered one order — the adapter dropped before the runtime. In the other order the runtime empties the tracker first, so the adapter's `Drop` finds nothing and says nothing: no release, no report, not even the `DroppedWhileRunning` that was queued and never processed. The crate promised "reported, never silent" and delivered it for one of two orders. The driver's own `Drop` is the one place that sees both, and it needs no runtime: `Machine::now()` is a stored reading. Found by the substrate review, S-03 |
| **OD-REFUSED-READY** | `start` is total on a plan the machine refuses. Is `ready()`? | **Yes**: the refusal latches the readiness state when the handle is built, so `ready()` answers `Err(Outcome::Failed)` at once, and so does `RunHandle::ready()` | 2026-09-06 | `start` was made total so that a refused plan is an outcome rather than a panic (`OD-START`), and `ready()` is the call a supervisor makes before deciding anything. It awaited a signal that only a running driver could request, and a refused run has no driver: the one path `start`'s totality exists for hung for ever. Found by the substrate review, S-01 |
| **OD-DOUBLE-HOLD-ERR** | a body registers twice and then returns `Err`. Which is the attempt's fault? | **`DoubleHold`.** It outranks the body's own `Err`; a panic outranks both | 2026-09-06 | Only the `Ok` path checked `hold_count() > 1`, so a double hold followed by an error was reported as a plain `Error` and the first value's obligation was dropped with the body's locals — no release, no record, and INV-9 ("no silent loss") none the wiser. The seam's one-value rule was broken whatever the body then returned, and only one fault kind can be carried: the engine's own observation that the seam broke is worth more than the body's opinion of its work, whereas a panic payload is carried nowhere else. Found by the substrate review, S-06 |
| **OD-BLOCK-ABORT** | `TaskHandle::abort()` on a blocking task | **A no-op**, and the task stays on the tracker until its thread returns | 2026-09-06 | A thread cannot be taken back (T5, T7). The tokio adapter spawns a pool job and a tracked task that awaits it; aborting that *wrapper* cancelled the accounting and not the work, so `tracked()` read zero and `shutdown()` answered `Ok` while the thread ran on — the single claim INV-15 rests on, false, and reachable through the host API by a `BodySource` whose `cleanup` returns `Task::Blocking` (T7b aborts a cleanup at the budget). Honouring the abort request was the lie; dropping it is the truth the engine already assumes. Found by the substrate review, S-04 |
