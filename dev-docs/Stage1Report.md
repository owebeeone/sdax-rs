# Stage 1 report

Date: 2026-09-06. Scope: `sdax-rs` only. Nothing is committed, tagged or pushed;
no network was used, and no dependency was added.

Stage 1 turns the Stage 0 vocabulary into a working engine: a pure, `std`-only
event/effect machine that implements T1–T8 for static plans and components, a
stepping simulator that drives it from a script on a virtual clock, a scripted
driver and trace-level invariant checker in the testkit, and a Monte Carlo suite
that walks random plans through both. **There is still no run driver**: no
`Plan::start`, no `Running`, no tokio adapter loop. Everything below was
executed on this machine; what was not is named as such.

---

## 1. What Stage 1 contains

### `crates/sdax/src/host/engine/` — the machine

| file | lines | what |
|---|---|---|
| `table.rs` | 233 | `Table`: the flattened plan the machine runs on — nodes, scopes, dependents, imports resolved, `RecordOrder.steps` numbered by view position |
| `state.rs` | 407 | `Machine`, `Slot`, `St` (node state), `RunState`, `Cause`, `Purpose` (what a timer is for), timer bookkeeping, `emit`, `record`, `fault`, `outcome` |
| `admit.rs` | 273 | T1: need-readiness, the FIFO queue, atomic all-or-none lock and pool grants, `start`, `check_steady`, `skip_dependents` (T4 under `Isolate`) |
| `faults.rs` | 496 | T3/T4: `NodeOk`/`NodeErr`/`Held`/`Started`/`NodeCancelled`/`ServeEnded`, retries and backoff, `within` deadlines, service restarts, `node_failed`, `component_faulted` |
| `settle.rs` | 233 | T5: `settle` for one scope, cancel modes, `skip_scope`, `interrupt`, the external `shutdown()`/`cancel()` requests |
| `cleanup.rs` | 434 | T6–T8: `owes`/`gate_open`/`blocks_release`, `try_cleanup`, `advance_cleanup`, `open_component`, the shutdown budget (`on_budget_timer`, `on_zero_timer`), `check_end`, `end_root` |
| `machine.rs` | 284 | `Machine::new(&Plan)`, `Machine::begin()`, `Machine::advance(Time)`, `Machine::step(Event) -> Vec<Effect>`, `Machine::take_report()`, `EngineError`, the D1-totality refusals (`Effect::Reject`) |

The machine is a pure function of its events: no clock, no tasks, no I/O. It
emits `Effect`s and consumes `Event`s, both already public in `sdax::host::engine`
from Stage 0. `Event::NodeErr` now carries a `FaultKind`, `Event::ServeEnded`
carries `Option<FaultKind>`, and `Effect::CancelTimer` and `Effect::Reject` are
new — those four vocabulary changes are the only additions to the Stage 0 host
surface.

### `crates/sdax/src/sim/` — the stepping simulator

| file | lines | what |
|---|---|---|
| `script.rs` | 244 | `Script`, `Body`, `Ending`, `Serve`, `Cleanup`, `Request`, `Schedule`, `At` — B's `CanonicalTests.md` § 0 notation as values |
| `simulator.rs` | 498 | the virtual clock, the event queue, `perform` for every `Effect`, and the sequencing rules (an outcome already due when an `Abort` arrives stands; a superseded attempt's outcome is dropped) |

`Plan::simulate(&Script) -> Result<Trace, ScriptError>` is the author-facing
entry (contract § 9, adoption A3). It is **new author surface**, and so are the
script types the crate root and `prelude` now re-export (`Script`, `Body`,
`Ending`, `Serve`, `Cleanup`, `Request`, `Schedule`, `At`). The stepping
`Simulator` itself is host surface, under `sdax::host::sim`, for a harness that
wants to watch every event and effect. `crates/sdax/tests/surface.rs` does not
yet pin any of them — see § 8.

### `crates/sdax-testkit/` — the harness

| file | lines | what |
|---|---|---|
| `src/driver.rs` | 211 | `ScriptedDriver::run(&plan, &script) -> Driven`; the liveness guard (a run that has not ended in 1 000 steps is reported, not ground); `Driven::check` |
| `src/eol.rs` | 284 | `Eol`: read a trace by node and event kind — `ready`, `held`, `fail`, `abandoned`, `cleanup_start`, `pos`, `overlap_count`, `max_concurrent`, `render` |
| `src/invariants/trace.rs` | 736 | the trace-level checker: INV-1…5, 7…12, 15, 18, 20, `orphans: none`, report ⊆ trace ⊆ report, F4 order |
| `src/mc/` | 1 009 | the Monte Carlo generator: `prng.rs` (splitmix64), `keys.rs`, `mutate.rs` (deliberate declaration mistakes), `gen.rs` (the node generator), `script.rs` |
| `tests/conformance/` | 2 285 | suite (c) over `corpus.rs`, in six modules plus `regressions.rs` |
| `tests/monte_carlo.rs` | 633 | the case loop, the coverage histogram with a floor per corner, `monte_carlo_big`, `a_run_seed_reproduces_the_whole_walk`, `every_seed_that_found_a_bug_replays_clean` |

`crates/sdax/tests/surface.rs` still passes: nothing Stage 0 pinned moved.

---

## 2. Gate results, verbatim

Apple M3 Pro, macOS 26.6.2, rustc 1.96.0 (`ac68faa20 2026-05-25`), run from
`/Users/owebeeone/limbo/sdax-wz/sdax-rs` after all the fixes below.

```
$ cargo fmt --check
exit=0

$ cargo test --workspace --locked --offline
test result: ok. 69 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.01s
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 58 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
test result: ok. 3 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.65s
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.75s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
exit=0
```

**174 passed, 0 failed, 2 ignored.** The two ignored are
`planner_view::renders_the_plan` (prints a rendering for eyeballing; not an
assertion) and `monte_carlo_big` (the long walk, run explicitly — see § 5).

```
$ cargo clippy --workspace --all-targets --locked --offline -- -D warnings
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.70s
exit=0

$ cargo package -p sdax --locked --offline --allow-dirty
   Packaging sdax v0.1.0 (…/crates/sdax)
    Packaged 49 files, 391.4KiB (98.7KiB compressed)
   Verifying sdax v0.1.0 (…/crates/sdax)
   Compiling sdax v0.1.0 (…/target/package/sdax-0.1.0)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.32s
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
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.47s
   Generated …/target/doc/sdax/index.html and 2 other files
exit=0
```

All seven gates green.

---

## 3. Measured timings

Apple M3 Pro, macOS 26.6.2, rustc 1.96.0, warm page cache, `/usr/bin/time -p`,
three runs each, from `/Users/owebeeone/limbo/sdax-wz/sdax-rs`, with nothing
else of consequence running. Every number below was measured on 2026-09-06;
none is copied from Stage 0.

| what | command | `real`, three runs |
|---|---|---|
| the fast loop, nothing to rebuild | `cargo test --workspace --locked --offline` | 1.70 / 1.66 / 1.62 s |
| warm incremental: touch a core file, then test | `touch crates/sdax/src/host/engine/machine.rs && cargo test --workspace --locked --offline` | 8.63 / 8.25 / 8.56 s |
| the default MC binary alone (3 000 cases + the two replay tests) | `cargo test --locked --offline -p sdax-testkit --test monte_carlo` | 0.69 / 0.68 / 0.72 s |
| the 50 000-case walk | `SDAX_MC_SEED=20260906 cargo test --locked --offline -p sdax-testkit --test monte_carlo monte_carlo_big -- --ignored` | 10.81 / 10.46 / 10.41 s |
| one package cold | `cargo clean -p sdax && cargo build -p sdax --locked --offline` | 0.97 s (once) |

**A full cold build was not measured**, and this is a deliberate omission: the
machine had about 6.5 GB free for most of the session and a full-workspace cold
build had already filled the disk for an earlier agent. `cargo clean -p sdax`
followed by rebuilding that one package is the honest substitute, and that is
what the last row is: `sdax` has zero normal dependencies, so it is the whole
cold cost of the crate that ships. The workspace's other cold cost is tokio and
its dependencies, which is unchanged from Stage 0.

**The fast loop budget in `AGENTS.md` has moved, and that is a finding, not a
tidy-up.** Stage 0 recorded ~0.55 s of execution with nothing to rebuild and
~3.8 s including the incremental rebuild. Stage 1 measures **~1.65 s** and
**~8.5 s**. The execution half is the Monte Carlo walk: 3 000 cases plus the
seed replays are ~0.68 s of it, and the conformance suite grew from 44 to 58
tests. The rebuild half is the new code: `sdax` gained ~3 100 lines of machine
and simulator, `sdax-testkit` gained ~2 600, and touching `machine.rs` rebuilds
`sdax`, `sdax-testkit`, and every test binary in both. `AGENTS.md` still quotes
the Stage 0 numbers; the owner should decide whether to restate the budget or
to shrink the default walk (`DEFAULT_CASES`, `monte_carlo.rs`) — this report
does not change either on its own initiative.

---

## 4. Lines of code

```sh
for c in sdax sdax-tokio sdax-testkit; do
  find crates/$c/src -name '*.rs' | xargs wc -l | tail -1
  find crates/$c/tests -name '*.rs' | xargs wc -l | tail -1
done
```

| crate | `src/` | `tests/` | total |
|---|---:|---:|---:|
| `sdax` | 10 844 | 578 | 11 422 |
| `sdax-tokio` | 178 | 200 | 378 |
| `sdax-testkit` | 2 608 | 3 074 | 5 682 |
| **workspace** | **13 630** | **3 852** | **17 482** |

Of `sdax`'s `src/`, the Stage 1 additions are `host/engine/` (2 360) and `sim/`
(756) — 3 116 lines. `sdax`'s `src/tests/` (inside the 10 844) includes
`machine.rs`, the 303-line P-16/P-17 unit test written first in row 1 of the TDD
log. `sdax-tokio` is untouched by Stage 1.

Every file is under the ~500-line house limit. The largest are
`invariants/trace.rs` (736, the checker — one function per invariant, and
splitting it would separate an invariant from its neighbours), `crates/sdax-testkit/tests/monte_carlo.rs` (633) and `crates/sdax-testkit/tests/conformance/corpus.rs` (428).
`trace.rs` and `monte_carlo.rs` are over the guideline and are named here rather
than quietly split; both are test-side, and both are one coherent
responsibility.

---

## 5. The Monte Carlo suite, and the coverage histogram

The suite generates a random plan from a case seed, mutates it (sometimes into
a plan `build` must refuse), scripts every body, drives it through the same
`ScriptedDriver` the conformance suite uses, and checks the whole trace and
report against the invariant checker. Each case also asserts INV-14 by replaying
one case in sixteen from its own seed and comparing byte for byte.

The default `monte_carlo` test walks **3 000** cases from a fixed run seed;
`SDAX_MC_SEED` and `SDAX_MC_CASES` override. `monte_carlo_big` is `#[ignore]`
and walks **50 000**.

### Re-captured histogram, default run, after every fix below

```
$ cargo test --locked --offline -p sdax-testkit --test monte_carlo monte_carlo -- --nocapture
coverage over 3000 cases:
     573  = outcome Cancelled
    1541  = outcome Failed
     713  = outcome Ok
     443  abandon during cleanup  (floor 5)
      91  ambiguity Compensate on an interrupted effect  (floor 1)
      82  ambiguity Report on an interrupted effect  (floor 1)
      89  ambiguity Retry on an interrupted effect  (floor 1)
     314  budget expiry abandons a release  (floor 1)
      78  cancel during backoff  (floor 5)
     187  component fault  (floor 5)
      74  cooperative grace completes  (floor 5)
     225  cooperative grace expires  (floor 5)
       8  exclusive contention  (floor 5)
     173  invalid plan refused  (floor 20)
     504  isolate skip propagation  (floor 5)
      48  pool wait  (floor 5)
      68  retry after a held attempt  (floor 5)
     336  second cancel during cleanup  (floor 5)
      23  service restart  (floor 5)
     288  two faults in one tick  (floor 5)
test monte_carlo ... ok
```

**Every floor clears on its own; no floor was lowered.** The thinnest corner is
`exclusive contention` at 8 against a floor of 5 — it needs two nodes that both
declare a lock on a third *and* an overlap in time, which the generator reaches
rarely. It is the one to watch if the generator changes; at 50 000 cases it is
92 and at 200 000 it is 369, so the corner is reached, just sparsely.

### The long walks

```
$ SDAX_MC_SEED=20260906 cargo test --locked --offline -p sdax-testkit \
    --test monte_carlo monte_carlo_big -- --ignored --nocapture
coverage over 50000 cases:
    9803  = outcome Cancelled
   25554  = outcome Failed
   11958  = outcome Ok
    8196  abandon during cleanup  (floor 5)
    1636  ambiguity Compensate on an interrupted effect  (floor 1)
    1496  ambiguity Report on an interrupted effect  (floor 1)
    1499  ambiguity Retry on an interrupted effect  (floor 1)
    5780  budget expiry abandons a release  (floor 1)
    1175  cancel during backoff  (floor 5)
    3180  component fault  (floor 5)
    1511  cooperative grace completes  (floor 5)
    3574  cooperative grace expires  (floor 5)
      92  exclusive contention  (floor 5)
    2685  invalid plan refused  (floor 20)
    8511  isolate skip propagation  (floor 5)
     872  pool wait  (floor 5)
    1218  retry after a held attempt  (floor 5)
    5481  second cancel during cleanup  (floor 5)
     276  service restart  (floor 5)
    4970  two faults in one tick  (floor 5)
test monte_carlo_big ... ok
```

A **200 000**-case walk from the same run seed also passes clean, in 43.3 s
(`SDAX_MC_CASES=200000`). It is not a standing test — nothing runs it in CI —
but it is what found two of the four bugs in § 6, and the report says so rather
than presenting 50 000 as the limit of what was tried.

---

## 6. Every bug the Monte Carlo suite found

**24 defects**, counted as the individually-argued fixes in rows 4–11 of
`dev-docs/Stage1-TDD-Log.md`. **21** are pinned by case seed in
`SEEDS_THAT_FOUND_BUGS` (`crates/sdax-testkit/tests/monte_carlo.rs`), replayed
by `every_seed_that_found_a_bug_replays_clean` on every `cargo test`, and most
also have a hand-written named test in `tests/conformance/regressions.rs`. The
remaining three were fixed in the first rounds, before the seed list existed,
and are recorded in the TDD log by case index instead.

Where the 24 landed: **15 in the machine**, **4 in the Monte Carlo generator**,
**2 in the simulator**, **2 in the invariant checker**, **1 in the testkit
driver**. No invariant, checker rule or coverage floor was weakened to make a
case pass; the two checker fixes are argued below.

| # | case seed | what it was | fixed in |
|---:|---|---|---|
| 1 | `6299039668138085550` | a `Body` whose `held` is later than its own ending fed the machine a `Held` for a body that had already returned | sim |
| 2 | `15604115191173039358` | `cleanup::abandon` dropped a node's accumulated faults, so a node abandoned mid-retry lost its earlier `Fail` and the run ended `Ok` | machine |
| 3 | `3133216432680936309` | `settle::interrupt` settled a component's inner scope but emitted no terminal observation for the component itself | machine |
| 4 | `15845907256369010671` | `table::flatten` numbered `RecordOrder.steps` with the builder's `key.idx`, which counts `import` nodes `PlanView` does not show | machine |
| 5 | `2052455747394511223` | `settle`'s `Pending`/`Waiting` arm dropped a node's earlier faults when the scope settled while it waited for its next attempt | machine |
| 6 | `16408307695250133523` | a service's serve fault was emitted to the trace and then dropped when a restart followed | machine |
| 7 | `4814721321710810844` | the checker demanded a compensation for an ambiguous `on_ambiguous(Compensate)` effect even when it is `persistent()` and has none | checker |
| 8 | `6364352641463191117` | `cleanup::abandon_all` walked one declaration-order pass, so a component's inner `ReleaseStart` could precede the `Abandoned` of a node depending on it (INV-5) | machine |
| 9 | `4529049039084370172` | `component_faulted` left the inner scope `Admitting` under an inner `Isolate`, so a sibling that became ready afterwards started a body after the run settled (T5) | machine |
| 10 | `13115684965286655053` | INV-9's escape knew `Interrupted` but not `Ambiguous`, putting INV-9 and INV-10 in direct contradiction for a panic in a body the engine had already cancelled | checker |
| 11 | `1784653771357370795` | a resident plan with unlimited `Restart::on_error` correctly never ends by itself, so the generator produced a case that could not terminate | generator |
| 12 | `14688502050082075365` | `settle`/`skip_dependents` skipped a component without touching its inner scope, whose `Pending` nodes then held the release gate of everything they import shut | machine |
| 13 | `666241015029950078` | a *ready* component's inner scope was never settled by its parent settling, so `in_flight` counted the component while the component waited for the run to reach cleanup — a deadlock | machine |
| 14 | `13281131733918634243` | `skip_dependents` dropped the parked faults exactly as `settle` did | machine |
| 15 | `3320581108311238895` | an inner scope whose own budget expired went straight to `Cleanup`, so the component's `ReleaseOk` had no `ReleaseStart` and its attempt was an orphan | machine |
| 16 | `6593995201653377501` | `Mutation::PoolStarve` put the starving step on `pools.last()`, so nothing was starved and the plan built | generator |
| 17 | `17502963814378720686` | a queued waiter started after a need stopped being `Ready` | machine |
| 18 | `2796147909731581100` | at the budget, `abandon_inner` emitted a component's `ReleaseStart` unconditionally, opening an inner release before a *dependent component* abandoned in the same instant had finished (INV-5) | machine |
| 19 | `2682709018262434330` | a component whose inner scope settled for a reason of its own (a `terminal` service inside it) never ended its own attempt: `Start(Prepare)` → `ReleaseStart`, an orphan (INV-15) | machine |
| 20 | `2300709931059428012` | `admit` iterated a waiter list taken before the first `start`; a component's inner graph coming up re-enters `admit`, and the outer pass then started a node the nested pass had already started (INV-12, INV-15) | machine |
| 21 | `11779147375297488456` | a timed-out blocking attempt's late thread outcome was credited to the *next* attempt, which reached `Ready` before its own `Started` (T7) | sim |

Not pinned by seed (fixed before the list existed; TDD log rows 4 and 6 name the
case indices): `gen::pool_of` used a random pool after making a new one, so a
fresh pool could end up with no user; `Mutation::DupName` took its name from
`lines[0]`, which can belong to a child plan where repeating it is not a
duplicate; and the driver's liveness guard sat after the quadratic prefix check
and at 100 000 steps, so a non-terminating run ground at 100 % CPU instead of
reporting (case 1946 hung at `attempt 249`, `t=1237s`).

**Rows 18–21 were found and fixed last, on 2026-09-06.** Row 18 was the one
open failure at the start of that session, and the standing diagnosis of it —
that `abandon_inner` emits a component's `ReleaseStart` unconditionally, and
that the transition belongs to the gate-guarded `open_component` instead — was
**confirmed against the trace and implemented as proposed**. Rows 19–21 were
found by continuing the walk: 19 by the 50 000-case walk once 18 was fixed and
the walk could get past case 11489, and 20 and 21 by the 200 000-case walk.
Each was reproduced first, given a failing named test in `regressions.rs`, then
fixed in the machine (or the simulator) and pinned by seed.

**Two checker changes, argued.** Rows 7 and 10 changed the invariant checker
rather than the machine, and both are cases where the checker was stating
something the contract does not say. Row 7: INV-18 says a `persistent()` effect
carries no compensation obligation, so there is nothing for `cleanup_ended_by`
to wait for; the core's `compensates_ambiguity` already excluded persistent and
the checker did not. Row 10: INV-10 says an engine-cancelled body is
`Interrupted`, never a fault; INV-9's escape in the checker knew `Interrupted`
but not `Ambiguous`, which is the same terminal observation for an effect that
had not held (INV-11) — so INV-9 and INV-10 contradicted each other on a panic
in an already-cancelled body. In row 10 a *machine* fix was written first and
then reverted: it is recorded as such in TDD log row 6.

---

## 7. Suite (c): which of B's `C-*` rows ran

B's `sdax-v1/B/CanonicalTests.md` § 4 has **46** suite-(c) rows. **41 are
implemented**, in `crates/sdax-testkit/tests/conformance/` over `corpus.rs`.
With three variant tests B itself lists or the row implies (`C-08b`, `C-12b`,
`C-24b`) that is **44 named test functions**, plus three more variants asserted
inline inside `c29`, `c38` and `c61`. The `regressions.rs` module adds 14 more,
one per bug the Monte Carlo walk turned up, for **58** conformance tests.

```sh
grep -rhoE '^fn c[0-9]+[a-z]*' crates/sdax-testkit/tests/conformance/*.rs | sort -u
```

```
c01 c02 c04 c05 c07 c08 c08b c09 c11 c12 c12b c15 c16 c18 c20 c21 c23 c24
c24b c26 c27 c29 c32 c33 c38 c40 c41 c50 c52 c53 c54 c55 c56 c57 c58 c59
c60 c61 c62 c63 c66 c67 c68 c69
```

**Five rows are omitted**, each for a stated reason:

| row | what it asks for | why not now |
|---|---|---|
| `C-14` | drop of `Running`; `DroppedWhileRunning` in the trace; the drainer joins the service; the report reaches the observer with `outcome: Cancelled` | **Stage 2.** There is no `Plan::start` and no `Running` to drop. Recorded in the module doc of `tests/conformance/cancel.rs`, not silently skipped. The machine half — `cancel()` while running, `RequestDuringCleanup`, `Outcome::Cancelled` — is covered by `C-11`, `C-12`, `C-12b` and `C-55`. |
| `C-30` | a template instance end to end: `cx.spawn`, `Child::ready`, containment (INV-16) | **Stage 3.** The contract's own stage table puts dynamic instances there. `Effect::SpawnInstance` exists in the vocabulary and the machine never emits it. |
| `C-51` | `AcceptLoop` spawning a template registered in another plan → `SpawnError::ForeignTemplate` to the body | **Stage 3**, same reason: it is a `cx.spawn` result. |
| `C-64` | an instance shutting down on its own — `outcome: Ok` for the instance, not a fault | **Stage 3.** |
| `C-65` | `cx.spawn` after `@shutdown` → `SpawnError::ScopeStopping`; no instance created | **Stage 3.** The seam already answers `SpawnError::NotRunning` with no run attached (Stage 0); the *stopping* answer needs a live instance table. |

`C-14` is easy to miscount as implemented, because the four Stage 3 rows are
the obvious omissions and `C-14`'s machine half *is* covered. It is not: the
row asks for the drop of `Running` and the drainer, neither of which exists.
This report counts it as an omission rather than folding it into the
implemented total.

### `S-02` did not run

`S-02` (suite (e)) asks for **exhaustive** schedule enumeration in the scripted
driver for every training plan with ≤ 6 nodes: every interleaving of
simultaneously-ready spawn effects and of scripted completion orders, with
INV-1…16 asserted in each. **No such enumeration exists and none was executed.**

What does exist is a randomised sample of the same space, and it should not be
read as a substitute:

- `Schedule::Order(..)` is an input of every run, so a test can force a
  particular interleaving and get it every time (INV-14). `C-38` asserts the
  same script gives a byte-identical trace, and that another schedule is a
  different but still-valid run.
- The Monte Carlo generator draws a random `Schedule::Order` per case; over
  200 000 cases that is a wide sample of interleavings, but it is a sample, and
  it is over generated plans rather than the training corpus.

Making `S-02` real is a bounded piece of Stage 2 or Stage 3 work — the machine
is pure, so enumeration needs no `loom` — and it is listed in § 10.

---

## 8. Deviations

1. **Two files are over the ~500-line house guideline**, both test-side and
   both named in § 4: `crates/sdax-testkit/src/invariants/trace.rs` (736) and
   `crates/sdax-testkit/tests/monte_carlo.rs` (633). `gen.rs` *was* split when
   it went over (TDD log row 3, into `keys.rs`/`mutate.rs`/`gen.rs`); these two
   were not, because each is one responsibility and splitting would separate an
   invariant from its neighbours.
2. **The fast-loop budget in `AGENTS.md` is stale** (§ 3). Not changed here.
3. **`Effect::SpawnInstance` is in the vocabulary and the machine never emits
   it.** It is Stage 3's, and it is not stubbed: no code path constructs it.
4. **Four vocabulary changes to the Stage 0 host surface**, all recorded in TDD
   log row 1: `Event::NodeErr` carries a `FaultKind`, `Event::ServeEnded`
   carries `Option<FaultKind>`, `Effect::CancelTimer` and `Effect::Reject` are
   new. `sdax::host` carries no stability promise before 1.0 (§ 10 of the
   contract), so this is inside what the contract allows, but it is a change to
   published signatures and is named here.
5. **New author surface is not pinned by `surface.rs`.** Stage 1 adds
   `Plan::simulate` and re-exports eight script types (`Script`, `Body`,
   `Ending`, `Serve`, `Cleanup`, `Request`, `Schedule`, `At`) at the crate root
   and in `prelude`. `crates/sdax/tests/surface.rs` pins the Stage 0 author
   half and was not extended to cover them, so the stability promise on those
   eight names is stated but not witnessed. A one-line addition per name; not
   done here because it is a Stage 0-style surface pin, not Stage 1 behaviour.
6. **MSRV verification is still partial**, unchanged from Stage 0: the oldest
   toolchain on this machine is 1.85, so `rust-version = "1.75"` is enforced by
   code review against the 1.75 feature set, not by a compiler.

---

## 9. Open questions for the owner

1. **The fast-loop budget.** `AGENTS.md` says ~0.55 s / ~3.8 s; the measured
   numbers are ~1.65 s / ~8.5 s (§ 3). Restate the budget, or shrink
   `DEFAULT_CASES` in `tests/monte_carlo.rs` from 3 000? The walk is what has
   found 24 defects, so shrinking it has a real cost.
2. **`S-02`.** Should exhaustive schedule enumeration for the ≤ 6-node training
   plans be built in Stage 2, or does the randomised sample stand? The machine
   is pure, so enumeration is cheap to write; the question is whether the
   combinatorics stay tractable on the larger corpus programs.
3. **`monte_carlo_big` is `#[ignore]` and CI runs nothing long.** Should a
   nightly job run the 50 000-case walk from a clock-derived seed and file the
   seed on failure? Today the long walk only runs when someone asks for it.
4. **The validator accepts `persistent()` with `on_ambiguous(Compensate)`.**
   The pair is meaningless — there is nothing to compensate — and the checker
   now models it as "persistent wins" (see the contract's Decisions table). A
   `V-*` rule refusing the pair at `build` would be a clearer answer; it was
   not added because that is a validator change, not a Stage 1 one.
5. **A stale attempt's outcome has no attempt number.** `Event::NodeOk(RawKey)`
   and `Event::NodeErr(RawKey, _)` name a node, not an attempt, so the machine
   cannot tell a superseded body's late result from the current one; the
   *driver* must drop it, and bug 21 is what happens when it does not. If the
   Stage 2 driver would find it easier to forward everything, the two events
   should carry `attempt: u32`. That is a host-surface change and the owner's
   call.

---

## 10. What Stage 2 needs

The tokio run driver is a loop around `Machine::step`. The reference
implementation is `crates/sdax/src/sim/simulator.rs`: `Simulator::step` picks
the earliest queued item, calls `machine.advance(now)`, then
`machine.step(event)`, then performs every returned effect in order. A real
driver replaces the virtual queue with `tokio::select!` over the task set, the
timer wheel and the request channel; nothing else about the shape changes.

**The machine is single-threaded and owns no state outside itself.** One
`Machine` per run, behind the driver task; effects are performed in the order
returned and never reordered.

### The `Effect`s the driver must execute

| effect | what the driver does |
|---|---|
| `Spawn { node, attempt }` | take the node's prepare/run/start body from `plan::Bodies`, build a `Cx` with a `host::CxInner`, `Runtime::spawn` it, keep the `TaskHandle` **keyed by node**; the handle for a *previous* attempt is dropped, and its result must not be forwarded (§ 9.5) |
| `SpawnBlocking { node, attempt }` | the same on `Runtime::spawn_blocking`, on the node's declared pool; the handle cannot be aborted (T7) |
| `Abort(node)` | `TaskHandle::abort()`. It is a request that lands between polls, so an outcome already due still arrives and stands |
| `Signal(node)` | raise the node's `StopSignal` **without** aborting: this is `cooperative(grace)` and a service's stop |
| `Release(node)` | run the resource's release body with the `Arc<T>` taken from the slot; shielded (INV-7) — never dropped by a request |
| `Compensate(node)` | run the effect's compensation with the receipt; shielded |
| `StopService(node)` | raise the stop signal and await the serve future; the machine sets its own deadline timer and will `Abort` if it expires |
| `Timer { id, at }` | arm a timer on the injected `Clock`; feed `Event::Timer(id)` when it fires |
| `CancelTimer(id)` | forget it. Firing it anyway is harmless: the machine ignores a timer it has forgotten |
| `Emit(TraceEvent)` | hand it to `Observer::event`. Must not block and must not panic |
| `End(outcome)` | the run is over; the report is `Machine::take_report()`. Resolve the `Running` future |
| `SpawnInstance { .. }` | **Stage 3.** The machine never emits it today |
| `Reject(Rejected)` | the driver fed an event that made no sense. A driver bug: log it loudly. The machine never panics on input (D1) |

### The `Event`s the driver must feed back

| event | when |
|---|---|
| `Started(node)` | the spawned body has been polled at least once. Must precede any other event for that attempt |
| `Held(node)` | `CxInner::take_held` saw a registration, **in the poll that observed the effect completing** (T2), before the body's continuation is polled again |
| `NodeOk(node)` | the body returned `Ok`; also a release/compensation/stop body completing |
| `NodeErr(node, FaultKind)` | the body returned `Err`, panicked, or the host's own timeout fired. `hold_count > 1` is `FaultKind::DoubleHold` |
| `NodeCancelled { node, held }` | an aborted task has been **joined** — T5 requires the join before the node counts as settled. `held` decides whether a release is owed |
| `ServeEnded { node, fault }` | a service's serve future returned; `None` for `Ok` |
| `Timer(id)` | a timer the machine armed fired. Call `machine.advance(now)` first |
| `ShutdownRequested` / `CancelRequested` | `Running::shutdown()`, and `cancel()` or a drop of `Running` |
| `TaskJoined { node, joined }` | the general form of the join: `Cancelled` is `NodeCancelled`, `Done` is a no-op, and `Panicked` splits — on a body the engine had already cancelled (`cancelling`, whether it was signalled or aborted) it is the panic *on the way out* of the cancel, so it goes to the trace and the node is `Interrupted`, never a fault (`OD-PANIC-CANCELLED`); anywhere else it is an `Err`. A `NodeErr(_, Panic)` is always a fault: that is the body's own return. Use `TaskJoined` where the runtime gives a `JoinError` rather than a body result. It is refused for a `component` or a `join`, which have no body |
| `InstanceSpawned` / `InstanceEnded` | **Stage 3.** The machine refuses them today with `"template instances are Stage 3"` |

### Two ordering rules the simulator encodes and the driver must keep

1. **An outcome already due when an `Abort` arrives is delivered.** The body
   finished before the abort could land, as it can on a real runtime; only
   outcomes still pending are dropped, and then the join reports the
   cancellation (`simulator.rs`, `Effect::Abort`).
2. **A superseded attempt's outcome is dropped.** When the machine starts
   attempt *k+1*, attempt *k*'s pending result no longer maps to anything. A
   blocking body is the case that forces it: it cannot be aborted (T7), so
   `on_within_timer` fails the attempt and its thread finishes later
   (`simulator.rs`, `Effect::Spawn`/`SpawnBlocking`; bug 21 in § 6).

### The one thing Stage 2 must open first

`plan::Bodies` — the erased prepare/release/blocking bodies, keyed by
declaration index — is still `pub(crate)` and still carries
`#[allow(dead_code)]`: **nothing reads it yet.** Stage 1 never needed it,
because the scripted driver scripts every outcome and runs no body at all. The
Stage 2 driver lives in `sdax-tokio`, a different crate, so `Bodies` and its
accessors must move under `sdax::host` (beside `CxInner`, which is already
there for the same reason) before the driver can spawn anything. That is the
first Stage 2 commit, and it is a host-surface addition, not an author one.

### What Stage 2 does not need to build

`Machine`, `Table`, the release-graph gates, the shutdown budget, the report
order — all of it is done and exercised. The conformance suite re-runs on the
adapter with paused time (the Stage 2 gate) by swapping `ScriptedDriver` for
the tokio driver behind the same `Driven::check`; the invariant checker and
`eol::Eol` are driver-agnostic and need no change.
