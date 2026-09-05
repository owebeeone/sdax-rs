# Stage 2 report

Date: 2026-09-06. Scope: `sdax-rs` only. Nothing is committed, tagged or pushed;
no network was used. One dependency **feature** was added (`tokio/sync`, plus
`rt-multi-thread` as a dev-dependency feature only) and is justified in § 9; no
dependency was added.

Stage 2 makes the engine run. The pure machine and the stepping simulator are
unchanged in behaviour; what is new is a tokio run driver that executes every
`Effect` through the `Runtime` port and feeds the `Event`s back, `plan.start(rt)`
and `Running` with its drop guard and drainer, blocking pools, and the suites
that make the claim checkable — above all suite (c) re-run against the adapter
from the **same sources**, with the traces compared against the pure machine's.

Everything below was executed on this machine; what was not is named as such.

---

## 1. What Stage 2 contains

### `crates/sdax/src/host/bodies.rs` — the stated blocker, cleared

`plan::Bodies` was `pub(crate)` with `#[allow(dead_code)]`. It is now
`sdax::host::bodies`, public, documented as host API and pre-1.0 unstable like
the rest of `host`, with the `allow` gone. The move exposed two real gaps, both
fixed here rather than worked around in the driver:

| gap | what it meant | fix |
|---|---|---|
| `PlanBuilder::component` recorded only the child's `PlanIr` | a component's inner nodes had **no runnable bodies at all** — the parent's declaration does not carry the child plan's code | `Bodies` carries `children: Vec<Arc<Bodies>>`, one per component, walked alongside the IR |
| a child scope's `import` node had no way to receive the ancestor's value | `Deps::fetch` reads *this* plan's `Slots` by the import node's own index, and an erased `Arc<T>` cannot be cloned without `T` | `import<T>` records an `ErasedImport` closure where `T` is still known; the source refreshes a scope's imports before it builds a body |

`BodySource` is the driver's whole view of "what code belongs to this node":
`body`, `cleanup`, `store`, `export`. `bodies_of(&plan)` is the one a plan
carries — one per run, because the slot tables it owns are per run (INV-13). A
harness can supply another, and that is what makes suite (c) a differential
check rather than a second suite.

### `crates/sdax-tokio/` — the run driver

| file | lines | what |
|---|---:|---|
| `src/body.rs` | 212 | what the driver spawns: the `Msg` inbox tagged by attempt epoch, the panic-guarded and hold-watching body wrapper, the blocking job, the serve task, the join-after-abort task |
| `src/driver.rs` | 473 | the loop: `advance(now)` → `step(event)` → perform every effect in order; one handle per node; timers as tasks; the report hand-off |
| `src/running.rs` | 448 | `PlanStart`, `Running` (`Future`, `ready`, `shutdown`, `cancel`, `snapshot`, `handle`, `#[must_use]`, `Drop`), `RunHandle`, `RunOptions`, `RunRecord`, `Snapshot`, `Control` |
| `src/lib.rs` | 234 | `TokioRuntime` (now `shutdown`, `with_clock`, and a `Drop` that reports live runs), `TokioClock`, `TokioTask`, `FnObserver` |

The shape is the simulator's, with the virtual queue replaced by a real task
set, real timers and a real request channel. The machine is single-threaded and
lives behind one task; effects are performed in the order returned.

**The two ordering rules, and where they live.**

1. *An outcome already due when `Abort` arrives is delivered.* The body task
   sends its outcome into the inbox before the abort can land; the inbox is
   FIFO, so the outcome is consumed first and the join that follows reports
   `Joined::Done` and sends nothing (`body.rs::join_aborted`).
2. *A superseded attempt's outcome is dropped.* Each node has one `Live` entry
   with an **epoch**; every message carries the epoch of the attempt that
   produced it, and `Driver::handle` drops a message whose epoch is not the
   node's current one. `Event::NodeOk(RawKey)` names a node, not an attempt, so
   without this a timed-out blocking body's late thread outcome would be
   credited to the attempt that replaced it — Stage 1's bug 21, on a real
   runtime.

**The third constraint (TDD log row 13 of Stage 1): never deliver a body
outcome for a `Kind::Component`.** The machine pushes no `Spawn` for one, so the
driver never creates a task for one; `spawn_body` also carries a
`debug_assert_ne!` on the kind, so a future change that broke it fails loudly in
a debug build rather than silently giving a component a fault vector the engine
assumes is empty.

**`Held` is reported in the poll that observed it (T2).** The body future is
wrapped: each poll of the inner future is followed, in the same `poll`, by a
check of `CxInner::hold_count()`; on the 0→1 transition the driver sends
`Event::Held` before returning, so the machine has it before the body's
continuation can be polled again. `hold_count() > 1` at completion is
`FaultKind::DoubleHold`.

**Panics never unwind into the runtime.** Every body — async, blocking, serve
and cleanup — is polled inside `catch_unwind(AssertUnwindSafe(..))`, and the
payload becomes `FaultKind::Panic`.

### `crates/sdax-testkit/src/scripted.rs` — a `Script` as bodies

`ScriptedBodies` is a `BodySource` that runs a `Script` on the engine's injected
clock: the same `Body`/`Serve`/`Cleanup`/`At` values the simulator reads, turned
into futures that `cx.sleep`, `cx.hold_value`, return, fail or panic at the
scripted instants. Under `start_paused(true)` a scripted run costs no wall clock
at all. `quiet_scripted_panics()` stops the default hook printing the deliberate
panics, and swallows only that exact payload.

---

## 2. Gate results, verbatim

Apple M3 Pro, macOS 26.6.2, rustc 1.96.0 (`ac68faa20 2026-05-25`), run from
`/Users/owebeeone/limbo/sdax-wz/sdax-rs` after every fix below.

```
$ cargo fmt --check
exit=0

$ cargo test --workspace --locked --offline
test result: ok. 69 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 58 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
test result: ok. 3 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.64s
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 62 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.29s
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.10s
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.11s
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.50s
test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.86s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
exit=0
```

**253 passed, 0 failed, 2 ignored** (174 at the end of Stage 1; **280** after the
Stage 1 semantics review's remediation, `dev-docs/Stage1-Review-Remediation-Log.md`).
The two ignored
are unchanged: `planner_view::renders_the_plan` and `monte_carlo_big`.

```
$ cargo clippy --workspace --all-targets --locked --offline -- -D warnings
    Checking sdax-tokio v0.1.0 (…/crates/sdax-tokio)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.26s
exit=0

$ cargo package -p sdax --locked --offline --allow-dirty
   Packaging sdax v0.1.0 (…/crates/sdax)
    Packaged 51 files, 410.6KiB (105.0KiB compressed)
   Verifying sdax v0.1.0 (…/crates/sdax)
   Compiling sdax v0.1.0 (…/target/package/sdax-0.1.0)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.86s
exit=0

$ ./scripts/check-architecture.sh
  sdax           role: pure/contract
  sdax-testkit   role: harness
  sdax-tokio     role: implementation

ARCHITECTURE GATE PASSED
exit=0

$ ./scripts/compile-fail.sh
  … PASS 14_S_01.rs: rejected with E0432
== 15 witnesses
exit=0

$ RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --offline
 Documenting sdax v0.1.0 / sdax-testkit v0.1.0 / sdax-tokio v0.1.0
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.81s
   Generated …/target/doc/sdax/index.html and 2 other files
exit=0
```

All seven gates green. The architecture gate passes with the adapter's new
dependency **features**: `sdax-tokio`'s normal dependencies are still exactly
`sdax`, `tokio`, `tokio-util`, and `sdax` still has none at all.

---

## 3. Measured timings

Apple M3 Pro, macOS 26.6.2, rustc 1.96.0, warm page cache, `/usr/bin/time -p`,
three runs each unless stated, nothing else of consequence running. Every
number was measured on 2026-09-06.

| what | command | `real` |
|---|---|---|
| the fast loop, nothing to rebuild | `cargo test --workspace --locked --offline` | 2.53 / 2.55 / 2.52 s |
| warm incremental: touch a core file, then test | `touch crates/sdax/src/host/engine/machine.rs && cargo test --workspace --locked --offline` | 11.65 / 11.78 / 11.82 s |
| the adapter suite (c), 62 tests | `cargo test -p sdax-tokio --test conformance` | 0.33 s |
| suite (d), the driver half | `cargo test -p sdax-tokio --test driver` | 0.04 s |
| suite (d), the substrate half (`R-04`/`R-06`/`R-07`) | `cargo test -p sdax-tokio --test substrate` | 0.53 s |
| `S-01`, 300 cases (the default) | `cargo test -p sdax-tokio --test spike` | 0.10 s |
| `S-01`, 20 000 cases | `SDAX_S01_CASES=20000 …` | 5.23 s |
| the adapter Monte Carlo, 250 cases (the default) | `cargo test -p sdax-tokio --test monte_carlo` | 0.10 s |
| the adapter Monte Carlo, 20 000 cases | `SDAX_MC_CASES=20000 …` | 5.98 s |
| the adapter Monte Carlo, 120 000 cases | `SDAX_MC_CASES=120000 …` | 37.17 s (once) |
| one package cold | `cargo clean -p sdax && cargo build -p sdax --locked --offline` | 0.88 s (once) |

**A full cold build was not measured**, for the same reason as Stage 1: the
machine had 16–19 GB free for this session and a full-workspace cold build had
already filled the disk for an earlier agent. `cargo clean -p sdax` and rebuild
is the honest substitute for the crate that ships.

**The fast-loop budget in `AGENTS.md` is now doubly stale**: it says ~0.55 s /
~3.8 s, Stage 1 measured ~1.65 s / ~8.5 s and Stage 2 measures **~2.53 s** and
**~11.7 s**. The execution half grew by the adapter's suites (~1.0 s: the
conformance re-run, `R-04`'s real-time pool test at 0.5 s, `S-01`, the adapter
walk); the rebuild half grew because touching the machine now rebuilds four more
test binaries. `AGENTS.md` was **not** changed here — Stage 1 raised the same
question and the owner has not ruled; § 11 asks again.

---

## 4. Lines of code

| crate | `src/` | `tests/` | total |
|---|---:|---:|---:|
| `sdax` | 11 197 | 634 | 11 831 |
| `sdax-tokio` | 1 367 | 1 825 | 3 192 |
| `sdax-testkit` | 2 998 | 3 079 | 6 077 |
| **workspace** | **15 562** | **5 538** | **21 100** |

Stage 2 adds ~3 600 lines: `sdax-tokio` grew from 178/200 to 1 367/1 825,
`sdax` gained `host/bodies.rs` (237) and lost the equivalent from `plan.rs`,
`sdax-testkit` gained `scripted.rs` (278) and `Driven::from_recorded`.

Every file Stage 2 wrote is under the ~500-line house limit; `tests/driver.rs`
reached 545 and was split into `tests/driver.rs` (332) and
`tests/substrate.rs` (261). The two files Stage 1 left over the guideline —
`sdax-testkit/src/invariants/trace.rs` (736) and
`sdax-testkit/tests/monte_carlo.rs` (633) — are untouched and still over it.

---

## 5. Which rows are implemented, and which are not

### Suite (c) on the adapter — `R-01`, the point of the stage

**All 58 conformance tests re-run against the tokio adapter and pass**, from the
same sources. `crates/sdax-tokio/tests/conformance.rs` includes every module of
`crates/sdax-testkit/tests/conformance/` by `#[path]` and compiles it a second
time against its own `Drv`; each module's one import line changed from
`use sdax_testkit::ScriptedDriver;` to `use crate::Drv as ScriptedDriver;`, so
all 65 call sites are byte-identical and there is no second copy of the suite
(LBT-009).

That is the expectations. The stronger claim is `conformance/differential.rs`:
**fourteen `(plan, script)` cases run on both drivers and their traces and
reports are compared.** They are equal. The one normalisation is that the events
of a single engine instant are compared as a sorted multiset, because the
contract promises nothing about the order of unordered pairs; a different event,
node, time or count still fails.

| row | where | state |
|---|---|---|
| `R-01` | `tests/conformance.rs` (58) + `conformance/differential.rs` (3) | **done** |
| `R-02` | `tests/driver.rs::r02_…` | **done** — the body's continuation never runs, `Interrupted{held:true}` precedes `ReleaseStart`, the release runs |
| `R-03` | `tests/driver.rs::r03_…` ×4 | **done** — `shutdown` returns `Ok(())` after a finished run and `Err(1)` over a task that will not end; a dropped `TokioRuntime` with live tasks emits `RuntimeDroppedWithLiveRuns`, and one with nothing running is silent |
| `R-04` | `tests/substrate.rs::r04_…` | **done** — pool limit 2, four blocking steps, high-water mark measured **by the bodies** and asserted `== 2` |
| `R-05` | `conformance/multi_thread.rs` | **partly** — see below |
| `R-06` | `tests/substrate.rs::r06_…` | **done** — a panicking body is a `Panic` fault, a panicking release is a `Panic` cleanup failure, neither is re-raised, and a second run on the same runtime is unaffected |
| `R-07` | `tests/substrate.rs::r07_…` | **done** — four concurrent runs on `new_multi_thread(2)`; the pool is declared **limit 1**, so a shared pool would show a high-water mark of 1 and it shows 4; the four exports are four distinct slot values |
| `C-14` | `tests/driver.rs::c14_…` | **done** — dropping a live `Running` emits `DroppedWhileRunning`, stops the service, runs the release, hands a `Cancelled` report with `incomplete: 0` to the observer, and leaves `tracked() == 0` |
| `S-01` | `tests/spike.rs` | **done** — 300 cases by default, 20 000 walked clean |
| adapter Monte Carlo | `tests/monte_carlo.rs` | **done** — 250 cases by default, 120 000 walked clean |

**`R-05` is partial, and the part not done is named.** It runs eight programs —
startup, faults under `Isolate`, a service restart, a budget expiry, locks, a
component — on `new_multi_thread(2)` with a **compressed** clock (one engine
second per 5 ms of real time), and asserts every invariant the checker knows
**except INV-8**. Two things are not claimed:

- it is eight programs, not all 58. The full suite at second-scale on a real
  clock would take minutes of wall clock, and `start_paused` is a
  current-thread facility, so there is no virtual clock to fall back on. The
  eight were chosen so every mechanism appears at least once.
- INV-8 is not asserted, and the reason is the clock rather than the engine: a
  real timer fires at *or after* its deadline, so the engine's own measurement
  of "budget to End" always overshoots by the scheduler's jitter, and the 200×
  compression turns a 7 ms hiccup into 1.5 engine seconds. It is asserted
  exactly under paused time (`R-01`, and `C-18`/`C-56`/`C-69` on the scripted
  driver). Recorded as decision **OD-INV8-CLOCK**.

Trace equality is not asserted for `R-05` either, as the brief allows: with two
workers the unordered pairs really do interleave differently.

### What Stage 2 did *not* do

| row | why |
|---|---|
| `C-30`, `C-51`, `C-64`, `C-65` | **Stage 3.** Dynamic instances; the machine still refuses a plan with a template, and `Effect::SpawnInstance` is still emitted by nothing. |
| `S-02` | Still not run. Exhaustive schedule enumeration for the ≤ 6-node training plans is unchanged from Stage 1's § 7; the randomised sample (the Monte Carlo walks, now on both drivers) is a sample, not the enumeration. |
| the adversarial review | The owner runs it after Stage 3, as the brief says. |
| `from_json`, brands | Out of scope, untouched. |

---

## 6. Bugs the adapter suites found

Four, all in the adapter or the harness; **none in the machine**. The pure
machine's behaviour is unchanged by Stage 2, and the differential check is the
evidence: the same traces, event for event.

| # | found by | what it was | fix |
|---:|---|---|---|
| 1 | suite (c) on the adapter — 43 of 58 rows | the driver announced `Event::Started` for *cleanup* bodies too; the machine has no body in flight for a node whose release it just opened, so it refused them (D1) | `run_body` gained an `announce` flag; only a prepare attempt announces |
| 2 | `R-05`, `i07 restart`, on `new_multi_thread(2)`, 1 run in 6 | **a real orphan.** The serve future was spawned on the start body's `Ok` alone. A start body that returns *after* the engine signalled it is `Interrupted`, not ready (T5) — so the serve task belonged to nobody (INV-15) and its `ServeEnded` reached the machine for a node that was not serving (D1) | the driver holds the serve future and arms it only if the machine then says the node is `Ready`, dropping it otherwise (**OD-SERVE-ARM**). Pinned under paused time by `r05_regression_a_signalled_start_body_never_starts_its_serve_future`, RED-checked |
| 3 | the adapter Monte Carlo, `SDAX_MC_SEED=6746427589533237250` index 365, **case seed `5722278272308672243`** | two requests at `t=0`: the first ended the run and the driver fed the second to a machine that had ended, which rightly refused it | `Driver::handle` returns early once `End` has been seen. The simulator had the same guard (`Simulator::step` stops at `ended`); the driver did not |
| 4 | `S-01`, first 40 cases | the harness's own: a `Running` dropped **before its first poll** has started nothing, so there was no drainer and no report — which is the laziness working, not a defect | the spike launches the run (`ready()`) before dropping it, and its request is made from the awaiting future rather than a spawned task, so `tracked()` counts the engine and not the harness |

Bug 2 is the one worth keeping: it needed a real runtime *and* two threads to
show up, it appeared in one run in six, and it is exactly the class the drop
guard and the tracker exist to catch.

### What the walks reached

```
$ cargo test -p sdax-tokio --test spike -- --nocapture
S-01 over 300 cases: [("Cancelled", 132), ("Failed", 85), ("Ok", 83)]

$ SDAX_S01_CASES=20000 cargo test -p sdax-tokio --test spike -- --nocapture
S-01 over 20000 cases: [("Cancelled", 9442), ("Failed", 5587), ("Ok", 4971)]
```

`S-01` asserts four things per case, each against something that can fail:
`TokioRuntime::tracked() == 0` (**no orphans**, from the substrate, not the
trace); the bodies' own count of holds against discharges plus abandonments
(**no held slot without a release**); the transport body's own record of having
returned `Err` against the report naming it (**every fault in the report**); and
the whole invariant checker, INV-8 included and exact because time is paused
(**bounded shutdown**). Its bodies are the plan's own — `Deps::fetch`,
`cx.hold_value`, `Serving`, the release graph — so it is the only suite that
exercises the whole path with nothing scripted.

**The Monte Carlo generator drives the adapter cheaply, so it does.**
`crates/sdax-tokio/tests/monte_carlo.rs` walks the testkit's own generator
against the run driver: same seeds, same plans, same scripts, invariants only.
It is 250 cases by default rather than the pure walk's 3 000, because a case
here builds a tokio runtime and spawns a task per attempt and costs roughly a
hundred times a pure case (0.10 s for 250; 37 s for 120 000).

---

## 7. Decisions

Added to the contract's Decisions table, dated 2026-09-06, with their reasons:

| id | decision |
|---|---|
| **OD-START** | `Plan::start` is the extension trait `sdax_tokio::PlanStart` (`start`, `start_with`, `try_start`, `try_start_with`), not an inherent method on `sdax::Plan`. |
| **OD-RT-SHUTDOWN** | `TokioRuntime::close_and_wait` folds into `shutdown(budget)` — Stage 0's open question 7, answered. |
| **OD-SERVE-ARM** | A service's serve future is armed when the machine says `Ready`, never on the start body's `Ok` alone. |
| **OD-REPORT-OBSERVER** | `Observer::report(&Report<()>)`, a default-no-op host method, called on every run whether awaited or dropped. |
| **OD-BODY-SOURCE** | The driver takes its code from a `BodySource`; `Bodies` carries components' bodies and imports' value copies. |
| **OD-INV8-CLOCK** | INV-8 is exact only on an exact clock and is asserted under paused time. |

Three of these deserve their reasoning restated where it matters most:

**`Plan::start` could not be an inherent method.** The contract names
`Plan::start`, but `sdax` has *zero* dependencies and `sdax-tokio` is the only
crate allowed to spawn (A2), so the crate that owns `Plan` cannot own the driver.
The alternative — a runtime-agnostic driver inside `sdax` — was considered and
rejected: it needs a hand-rolled async channel (a `Waker` implementation nobody
reviews), it puts the run loop in the crate whose whole point is that it executes
nothing, and it makes the `Bodies` move to `host` pointless. The extension trait
keeps the call site the contract asks for — `use sdax_tokio::PlanStart;` then
`plan.start(rt)` — and leaves `sdax::Start`, the service phase marker, its name.
`start` is total: a plan the machine refuses (`L-IMPORTS`, or a template until
Stage 3) yields a `Failed` report carrying the refusal rather than a panic, and
`try_start` is the checked form for a caller who wants the `Result`.

**A blocking body's wait is abandoned and recorded, exactly as the contract
says.** `Effect::SpawnBlocking` goes to `Runtime::spawn_blocking`, bounded by the
declared pool — the machine's own grant, measured for real in `R-04`. The
machine never emits `Abort` for a `BlockingStep` (`interrupt` and `abandon` both
skip the kind, and `on_within_timer` fails the attempt instead), so the driver
never tries; the thread runs on, its late outcome is dropped by epoch, and the
node is `Abandoned` in `incomplete`.

**`Task::Async` vs `Task::Blocking` is the source's call, not the driver's.**
The plan's own bodies always answer a blocking step with `Blocking`. A scripted
source answers `Async` even for a blocking step, because a scripted body's whole
behaviour is a wait on the engine clock and a pool thread cannot wait on virtual
time. The driver honours whichever it is given. The real blocking path is
covered by `R-04`, with real bodies on a real clock.

---

## 8. Deviations

1. **`Plan::start` is `PlanStart::start`** (OD-START), and `Running` lives in
   `sdax-tokio`. The contract's § 1 and § 11 wording is updated to say so.
2. **`R-05` is eight programs, not 58, and does not assert INV-8** — § 5.
3. **A scripted run reports `output: None`.** `ScriptedBodies` registers
   placeholder values, so there is no typed export to hand back; it says so
   rather than fabricating one. Every test that asserts an export
   (`r00_a_plan_runs_to_ok_on_the_adapter`, `R-07`) uses the plan's own bodies.
4. **A `Joined::Panicked` after an abort is reported as `NodeCancelled`.** Our
   wrapper catches every panic inside the body, so the only way a joined task
   reports a panic is a panicking `Drop` of the body's locals during the abort —
   which is the engine's cancel taking effect, and OD-PANIC-CANCELLED says
   `Interrupted` wins. It is unreachable in practice and is documented at the
   call site rather than left implicit.
5. **`Driven::from_recorded` runs the prefix check over every prefix of the
   finished trace**, not after each step, because a real driver hands back a
   trace rather than a stepping machine. That is a superset of the per-step
   checks, not a weaker one, and it is quadratic — acceptable at these trace
   lengths and named here rather than hidden.
6. **MSRV verification is still partial**, unchanged from Stage 0 and 1: the
   oldest toolchain on this machine is 1.85, so `rust-version = "1.75"` is
   enforced by review against the 1.75 feature set. Stage 2 used nothing newer:
   no `LazyLock`, no `Waker::noop`, no `#[expect]`, no `async fn` in a trait
   (`BodySource` returns boxed futures inside `Task`), and `let … else` is 1.65.
7. **`AGENTS.md`'s fast-loop budget was not changed** — § 3, § 11.

---

## 9. The one dependency change, justified

`sdax` is unchanged: still `std` only, zero normal dependencies, MSRV 1.75,
`unsafe_code = "forbid"`. `sdax-testkit` still depends only on `sdax`.
`sdax-tokio`'s normal dependencies are still exactly `sdax`, `tokio` and
`tokio-util`. Two **features** changed:

| feature | where | why |
|---|---|---|
| `tokio/sync` | normal | The driver needs an async multi-producer inbox (`mpsc::unbounded_channel`: one receiver in the run loop, one sender per body task) and a report hand-off (`oneshot`). A run loop cannot be written without an async channel, and hand-rolling one is a `Waker` implementation nobody reviews. |
| `tokio/rt-multi-thread` | **dev only** | `R-05` and `R-07` need genuine parallelism. The library never builds a runtime of its own, so a consumer of `sdax-tokio` never compiles the multi-threaded scheduler because of us. |

Still **no `macros`**: there is no `#[tokio::test]` and no `select!` anywhere in
the workspace; the driver's loop is a plain `while let Some(msg) = rx.recv()`
because everything else — timers, joins, requests — sends into the same inbox.

The raw-spawn gate is unchanged and not widened: `clippy.toml` still refuses
`tokio::{task::spawn, task::spawn_blocking, spawn}`,
`tokio::runtime::Handle::{spawn, spawn_blocking}` and `std::thread::spawn`
everywhere, and `sdax-tokio` still has exactly **three** scoped
`#[allow(clippy::disallowed_methods)]` call sites, all in
`Runtime for TokioRuntime`, each with a comment saying why. The driver never
spawns raw: it spawns through the port.

---

## 10. Open questions for the owner

1. **The fast-loop budget** (§ 3), now ~2.5 s / ~11.7 s against `AGENTS.md`'s
   ~0.55 s / ~3.8 s. Restate it, or shrink the default walks? The candidates are
   `SDAX_MC_CASES` on the adapter (250, ~0.10 s), `SDAX_S01_CASES` (300,
   ~0.10 s), the testkit's `DEFAULT_CASES` (3 000, ~0.68 s) and `R-04`'s
   real-time pool test (~0.5 s, and it cannot use paused time). Stage 1 asked
   the same question and it is still open.
2. **`R-05`'s coverage.** Eight programs, or all 58 with a compressed clock and
   INV-8 excluded? The second costs perhaps 3–4 s of wall clock and buys a much
   wider parallel sweep; it also makes the suite's timing sensitivity a standing
   CI risk.
3. **Should a nightly job run the long walks?** `S-01` at 20 000 (5.2 s), the
   adapter Monte Carlo at 120 000 (37 s) and `monte_carlo_big` all pass and none
   of them runs in CI. A clock-derived seed printed on failure would turn them
   into a real net.
4. **`Snapshot::nodes` is opt-in** (`RunOptions::observe_states`) because it
   costs one state read per node per step. Is that the right default for a
   production supervisor, or should the snapshot always carry node states?
5. **`Running::ready` takes `&mut self`** because it must launch a lazy run. A
   `RunHandle::ready()` exists that takes `&self` but cannot launch. If the
   laziness is not worth that asymmetry, `start` could launch eagerly and both
   could take `&self` — at the cost of C-11's "cancel before the first poll
   costs nothing", which the contract does not actually require of the adapter.
6. **`S-02`** is still not run (Stage 1's § 9 question 2, unchanged).

---

## 11. What Stage 3 needs

Stage 3 is dynamic instances end to end: `cx.spawn`, `Child::ready`,
containment (INV-16), and `C-30`, `C-51`, `C-64`, `C-65`.

**What exists for it already.**

- `Effect::SpawnInstance { template, id }` and
  `Event::{InstanceSpawned, InstanceEnded}` are in the vocabulary; the machine
  refuses them today with `"template instances are Stage 3"`, and nothing
  constructs the effect.
- `Scope`, `ChildControl`, `Child`, `InstanceId` and `SpawnError` are in
  `sdax::host`; `CxInner::with_scope` attaches a run's scope to a body context,
  so `cx.spawn` reaches the run rather than answering `SpawnError::NotRunning`.
- `Table::flatten` already carries `Option<InstanceId>` in every `RecordOrder`
  step, and `Report::sort` already orders instances by id (F4). `Table::build`
  refuses templates in one place.
- `Bodies` already carries a **child plan's own bodies** (`children`) and the
  **import copies** a nested scope needs. A template's plan is a child plan of
  exactly that shape, so an instance's bodies are already reachable; what is
  missing is a `Slots` table *per instance* rather than per plan.

**What Stage 3 has to build.**

1. **The machine's instance table.** `Table` is flat and static: one node per
   declaration. An instance adds nodes at run time, so either the table grows or
   the machine keeps a per-instance overlay. Every derived structure — needs,
   dependents, the release gate (`owes`/`gate_open`), `in_flight`, the scope
   states — is indexed by flat position today.
2. **`Scope::spawn_instance` on the run.** The driver must implement it: refuse
   an undeclared or foreign template (`UndeclaredTemplate`, `ForeignTemplate`),
   refuse while settling (`ScopeStopping`), and otherwise hand back a `Child`
   whose `ready()` resolves when the instance's own scope reaches steady state
   (INV-17). That is a second readiness gate alongside `Running::ready`, and the
   `Control`/`StopSignal` latch this stage built is the shape to reuse.
3. **Per-instance slot tables.** `PlanBodies` keeps one `Slots` per *plan*; an
   instance needs one per *instance*, keyed by `InstanceId`, with the import
   refresh reaching the instantiating scope. `BodySource` will need the instance
   id in its signatures, which is a host-surface change and the reason
   `sdax::host` still carries no stability promise.
4. **`C-14`'s instance half.** The drainer stops every live instance before any
   key it imports is released (INV-16). The gate logic exists
   (`blocks_release`); the live-instance set does not.
5. **A `V-*` decision that is already noted**: the validator accepts
   `persistent()` with `on_ambiguous(Compensate)` (Stage 1 § 9 question 4,
   settled as OD-PERSIST-AMBIG but not refused at `build`).

Nothing in the driver blocks any of this. The loop, the epoch filter, the
join-before-settled rule, the drop guard and the drainer are all indifferent to
where a node came from; the work is in the machine's table and in the run's
scope object.
