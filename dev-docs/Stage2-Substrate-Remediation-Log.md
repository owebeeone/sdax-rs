# Stage 2 substrate review — remediation log

The findings of `dev-docs/Review-Stage2-Substrate.md` (an independent
adversarial review of the tokio adapter, run on a real runtime at `cacf849`;
its six probes are in `dev-docs/review-probes/`), worked the LBT-007 way: the
reviewer's case written as a test **first**, watched failing with the predicted
symptom, then whatever was actually wrong — the adapter, the machine, a test or
a document — changed.

Baseline: `main` at `7c6257c` (Stage 3 landed), tree clean, **302 tests green**,
all seven gates green. After: **310 tests**, all seven gates green.

Where a prediction did not reproduce as stated, the row says so and quotes what
happened instead; nothing was bent to match a wrong prediction. Nothing was
committed, tagged or pushed.

`dev-docs/Review-Stage2-Substrate.md` carries the per-finding disposition in an
appended **Remediation** section; this file carries the evidence.

---

## The table

| id | sev | disposition | what changed | regression |
|---|---|---|---|---|
| S-01 | major | **fixed** | `running.rs`: the refusal latches the readiness state when the handle is built (`OD-REFUSED-READY`) | `s01_ready_on_a_refused_plan_answers_with_the_refusal` |
| S-02 | major | **fixed** | `driver/observer.rs`: every observer callback under a panic boundary; `TraceKind::ObserverPanicked` (`OD-OBSERVER-PANIC`) | `s02_an_observer_panic_in_event_does_not_kill_the_driver` |
| S-03 | major | **fixed** | `driver/observer.rs`: `impl Drop for Driver` — event, report and readiness latch in **both** drop orders (`OD-DRIVER-DROPPED`) | `s03_a_runtime_dropped_under_the_drainer_is_reported_in_both_orders` |
| S-04 | minor | **fixed** | `lib.rs`: `abort()` is a no-op for a blocking `TokioTask` (`OD-BLOCK-ABORT`); `body.rs`: `blocking_job` gains `announce`, `false` for a cleanup | `s04_a_blocking_cleanup_abandoned_at_the_budget_stays_tracked` |
| S-05 | minor | **fixed** | `driver.rs`: `finish` latches and sends before telling the observer, under the same boundary as S-02 | `s05_an_observer_panic_in_report_still_delivers_the_report` |
| S-06 | minor | **fixed** | `body.rs`: `outcome_event` reports `DoubleHold` whatever the body returned, a panic excepted (`OD-DOUBLE-HOLD-ERR`) | `s06_a_double_hold_followed_by_an_error_is_reported_as_a_double_hold` |
| S-07 | minor | **fixed (test)** | `substrate.rs`: `R-07`'s `tracked() == 0` becomes a bounded `shutdown`, the documented orphan check | `r07_concurrent_runs_of_one_plan_share_no_slots_and_no_pools` |
| S-08 | note | **document fixed** | contract `T7d`; `lib.rs` crate docs and `TokioRuntime::shutdown` | — (stated, not executed: see below) |
| S-09 | note | **document fixed** | `running.rs` module and `Running` docs; `lib.rs` crate docs | — |
| S-10 | minor | **fixed (test)** | `multi_thread.rs`: `shutdown()` asserted per case, `running.await` bounded, `stuck` derived from it | `r05_the_invariants_hold_on_two_worker_threads` |
| S-11 | minor | **fixed** | `check_report_with_slack` / `check_trace_with_slack`, `Recorded::clock_slack`; INV-8 asserted at ×20 with 0.5 s of slack (`OD-INV8-CLOCK` amended) | `r05_the_shutdown_bound_holds_on_two_worker_threads_with_measured_slack` |
| S-12 | minor | **fixed** | `driver.rs`: a rejection is `TraceKind::Rejected(what)` as well as a record and a stderr line (`OD-REJECT-VISIBLE`) | `s12_a_rejection_reaches_the_observer_and_the_trace` |
| S-13 | note | **document fixed** | contract § 10; `lib.rs` crate docs | — (not executed; a profile change rebuilds the workspace) |
| S-14 | note | **document fixed** | `OD-REPORT-OBSERVER` amended; `Observer::report` docs; `Running` docs | covered by `C-11` |
| S-15 | note | **document fixed** | `lib.rs`: three call sites, `close_and_wait` gone, `Drop`'s reason corrected | — |
| S-16 | note | **fixed** | `lib.rs`: `with_clock` wraps in `SeenClock`; `Snapshot::live` renamed `contexts` and described | — |
| S-17 | note | **fixed (test)** | `multi_thread.rs`: `REPEATS = 3`, `SDAX_R05_REPEATS` to raise it | `r05_the_invariants_hold_on_two_worker_threads` |
| MC-1 | — | **fixed** | found by the re-run walk, not by the review: `cleanup.rs`, `after_cleanup` re-admits | seed `9106096978137470251` in `SEEDS_THAT_FOUND_BUGS` |

Two of the reviewer's predictions did not reproduce exactly as written; both
rows say what happened instead (S-02's `ready()`, S-04's `tracked()`).

---

## Majors

### S-01 — `ready()` on a refused plan hangs

**Test first.** `s01_…` (`crates/sdax-tokio/tests/review_edges.rs`), the
reviewer's A1: a child plan with one unresolved `import` started as a root
(`L-IMPORTS`), `tokio::time::timeout(3600 s, running.ready())` under paused
time, where a virtual hour elapses the moment nothing is left to do.

**RED**, exactly as predicted:

```
assertion `left == right` failed: ready() on a refused plan must answer, not hang
  left: Err(Elapsed(()))   right: Ok(Err(Failed))
```

**Fix.** `running.rs`, `start_with`'s refused arm: build the `Control`, then
`ctl.ended(Outcome::Failed)` before handing the handle back. Latching rather
than special-casing `ready()` fixes `RunHandle::ready()` in the same move, and
the test asserts both.

### S-02 / S-05 — a panicking observer kills the driver

**Test first.** `s02_…` and `s05_…`
(`crates/sdax-tokio/tests/review_observer.rs`), the reviewer's B7 and A3: a
resource and a service that serves until `cx.stop()`, and an observer that
panics on `TraceKind::Settling` (S-02) or in `report()` (S-05). The bodies
count what they actually did, because the panicking observer's own record is
not evidence.

**RED**, exactly the predicted symptom in both — the awaiter of a run that
ended `Ok` was handed an empty `Cancelled`:

```
S-02: Report { outcome: Cancelled, faults: [], …, trace: None }   left: Cancelled  right: Ok
S-05: Report { outcome: Cancelled, faults: [], …, trace: None }   left: Cancelled  right: Ok
```

**Did not reproduce as written:** the review says `ready()` hangs. It does when
the panic is on `TraceKind::Ready` (the review's A2); in B7's shape — a panic on
`Settling`, after readiness — `ready()` had already returned, and the assertion
`ready == Ok(Ok(()))` passed even at RED. The hang is real and is the same
defect; the test keeps the bound so a regression that moves the panic earlier
still shows as a timeout rather than a stall.

**Fix.** New `crates/sdax-tokio/src/driver/observer.rs` — everything the driver
says to the outside world, moved into a child module, which also brings
`driver.rs` back under the house limit (522 lines before this round, 500 after):

- `notify(&ev)` and `tell_report(&mut report)` wrap `Observer::event` and
  `Observer::report` in `catch_unwind(AssertUnwindSafe(..))`.
- A panic is recorded as `TraceKind::ObserverPanicked`, written immediately
  before the event whose delivery panicked — which keeps `End` the last event
  of the trace (T8) without a special case, because for every call but
  `tell_report` the offending event has not been pushed yet.
- `finish` latches readiness and hands the report to its awaiter **before**
  telling the observer, so `report()` can no longer reach the awaiter at all.
- `TokioRuntime::drop`'s own `event` call is guarded too: a report of a lost
  run must not become a panic out of a `Drop`.

The obligation in contract § 10 stands — the observer's copy of that event is
lost — but the run is not. `OD-OBSERVER-PANIC`.

### S-03 — runtime teardown under a pending drainer is silent

**Test first.** `s03_…` (`review_edges.rs`), the reviewer's A4: `ready()`, then
`drop(running)` inside a `block_on` that returns at once, so the drainer's
`Msg::Dropped` is queued and never processed; then both drop orders.

**RED**, in both orders — no report at all, the adapter-first order included:

```
assertion `left == right` failed: adapter_first=true: exactly one report reaches the observer
  left: 0   right: 1
```

(The review reports `RuntimeDroppedWithLiveRuns` present in the adapter-first
order. It is — the event, not the report; the assertion that fails first is the
report. Both orders were silent about the run itself.)

**Fix.** `impl Drop for Driver`: if `done` has not been taken — which is exactly
"the loop never reached `finish`" — emit `TraceKind::RuntimeDroppedWithLiveRuns`
at `Machine::now()`, latch readiness `Cancelled`, hand the report with the trace
so far to the observer and to `done`. Nothing there touches a tokio API:
`Machine::now()` is the last reading the machine was given, so it is safe on
whatever thread the runtime's shutdown runs on. `OD-DRIVER-DROPPED`.

This is also why `TokioRuntime::drop` no longer has to be the only witness: it
covers one order, the driver's own `Drop` covers both.

---

## Minors

### S-04 — a blocking cleanup's thread escapes the tracker

**Test first.** `s04_…` (`review_edges.rs`), the reviewer's B2: a `BodySource`
whose `cleanup` returns a `Task::Blocking` sleeping 300 ms, `Shutdown::within(50 ms)`,
real clock, `shutdown()` at 10 ms, so T7b aborts the cleanup at the budget.

**RED**, the spurious `Reject` exactly as predicted:

```
a cleanup body announces no Started:
  ["Started for a node with no body in flight — Started(RawKey { plan: 1, idx: 0 })"]
```

**Did not reproduce as written:** the review reads `tracked_after_await = 0`, and
the run's outcome as `Cancelled`. On today's tree the outcome is `Ok` — a
shutdown a `Resident` plan answers is a clean end, with the abandoned release in
`incomplete`, which is T7b working. The tracker claim is the finding and it
held; the outcome in the review's prose is its own case, not this crate's.

**Fix.** Two, both small:

- `lib.rs`: `TokioTask` carries `blocking: bool`, and `abort()` returns without
  doing anything for one. `spawn_blocking` spawns a pool job plus a tracked task
  that awaits it; aborting that *wrapper* cancelled the accounting and not the
  work. `OD-BLOCK-ABORT`.
- `body.rs`: `blocking_job` gains `announce`, like `run_body`'s, and
  `spawn_cleanup` passes `false`.

**GREEN**, with the assertion made sharper than the review's: `tracked() >= 1`
right after the run (two, in fact — the wrapper and the joiner the abort
spawned), `shutdown(50 ms) == Err(..)` while the thread works, then
`shutdown(2 s) == Ok(())` and the thread's own flag `done == true`. The claim
"`tracked() == 0` ⇒ no orphans" is true again on this path.

### S-06 — a double hold followed by `Err` is silent

**Test first.** `s06_…` (`review_edges.rs`), the reviewer's A10:
`hold_value(1); hold_value(2); Err(..)`.

**RED**, exactly as predicted:

```
assertion `left == right` failed: the seam's one-value rule is what was broken
  left: [Error]   right: [DoubleHold]
```

**Fix.** `body.rs`, `outcome_event`: the `hold_count() > 1` arm moves above the
`Ok(Err(e))` arm, so a double hold outranks the body's own error; a panic still
outranks both, because its payload is carried nowhere else. `OD-DOUBLE-HOLD-ERR`.
The test also pins INV-3 on the same run: the value that *was* banked (`2`) is
still discharged by the release.

### S-07 — `tracked() == 0` after `Running` resolves is a race

**Measured first**, the reviewer's B3 on today's tree: 300 runs of a two-node
plan on `new_multi_thread(2)`, `tracked()` read immediately after
`running.await`.

```
S-07: tracked()!=0 right after await in 246/300 runs (max 2)
```

(The review measured 264/300 at `cacf849`; same class, same maximum.) The
measurement was a throwaway binary and is not kept: it asserts a *race*, so as a
test it would be a coin toss.

**Fix, and which of the two the brief offered:** `R-07` **waits for quiescence**
rather than dropping the assertion. `substrate.rs`:
`assert_eq!(rt.tracked(), 0)` becomes
`assert_eq!(tokio_rt.block_on(rt.shutdown(secs(5))), Ok(()))`, with the
measurement in the comment. `shutdown` is the documented orphan check and is a
bound rather than a snapshot; the run's own end is what `running.await` already
proved. `finish()` was *not* made async: it would not help, because the driver
task itself is still tracked when it sends `done`.

### S-10 — `R-05` did not check what its doc claimed

**Fix.** `multi_thread.rs`, in `once` (the renamed per-execution body):

- `running.await` is wrapped in `tokio::time::timeout(LIVENESS, ..)` — 30 real
  seconds, against programs that take under a tenth of that — so a multi-thread
  hang is a failed assertion and not a stalled suite;
- `stuck` is derived from that timeout instead of being hard-coded `false`, and
  a stuck run now fails the case;
- `assert_eq!(rt.shutdown(5 s), Ok(()))` per execution, which is the orphan
  check the module doc claimed and did not have. A trace cannot see a detached
  thread (S-04 is the demonstration), so this is the only place on two workers
  where INV-15 is actually measured.

The module doc no longer claims anything it does not do.

### S-11 — INV-8 on the multi-thread runtime

**Measured first**, the reviewer's E1 on today's machine — a bare
`tokio::time::sleep(20 ms)`, 15 samples:

```
E1 multi_thread(2): slop over 20ms sleep, min=1085us median=5821us max=11111us
```

So 1–11 ms per timer, which at ×200 is 0.2–2.2 **engine seconds** — larger than
some of the budgets `R-05` uses. At ×200 INV-8 is therefore not assertable at
all, and `OD-INV8-CLOCK` is amended to say *absent* rather than tolerant.

**Implemented at a lower compression.** The checker takes the allowance as a
parameter — `check_report_with_slack` / `check_trace_with_slack`, and
`Recorded::clock_slack`, `Duration::ZERO` everywhere the clock is exact, which
is every run the testkit drives itself and every paused adapter run. A second
test, `r05_the_shutdown_bound_holds_on_two_worker_threads_with_measured_slack`,
runs the two budget-relevant programs (`i16`, whose budget expires with a
cleanup that ignores the stop, and `i08`) at ×20 with 0.5 engine seconds of
slack, and checks **every** invariant including INV-8.

**It has teeth**, and the number is measured rather than chosen: with the
allowance set to zero the same run reports

```
i16 budget expiry #0: INV-8: 3.04042166s elapsed from Settling to End,
40.42166ms over the budget 3s and over the clock slack 0ns
```

— so 0.5 s is ×12 the observed error and a sixth of the smallest budget in play.
A driver that waited 3.5 engine seconds against a 3-second budget fails.

**Stability, as the brief asked:** 30 consecutive runs of the new test, each
running 2 programs × 3 repeats — **0 failures in 30 runs (180 executions)**.

Cost: `R-05`'s file goes from 0.28 s to 0.98 s of wall clock, of which ~0.3 s is
this test and ~0.4 s is S-17's repetition.

### S-12 — a `Reject` never reaches a production observer

**Fix.** `driver.rs`, the `Effect::Reject` arm: the rendered rejection is still
pushed to the `RunRecord` (or stderr when none is attached) and is now also a
`TraceKind::Rejected(what)` event — through the same guarded observer path, and
into the trace the report carries. `OD-REJECT-VISIBLE`.

**Where the test lives, and why it is not an integration test.** After S-04 there
is no plan, script or `BodySource` that can make the machine refuse an event this
driver feeds it: that is what the § 10 obligations are for, and the reviewer's
only witness (a blocking cleanup's `Started`) is the thing S-04 removed. Every
other refusal — "the run has ended", "shutdown after End" — is unreachable
because `handle` drops late messages by design. So the test builds a `Driver`
over a real `TokioRuntime` with a recording observer and hands it the rejection
directly: `s12_a_rejection_reaches_the_observer_and_the_trace`, an in-crate unit
test in `driver/observer.rs`, with a `#[cfg(test)] Control::detached` next to it.

**RED** was taken by reverting the emission: the trace was `[]` and the only
sign of the refusal was the line the review complained about —

```
sdax-tokio: the machine refused an event: Started for a node with no body in flight — Started(node)
thread '…' panicked: the report's trace carries it: []
```

### S-16 — two gaps in the host surface

**Fix.** `lib.rs`: `with_clock` wraps the caller's clock in a private
`SeenClock` that records each reading into `seen`, so `TokioRuntime::drop`
timestamps its event on the clock the run actually used instead of `Time::ZERO`.
`running.rs`: `Snapshot::live` is renamed `Snapshot::contexts` and documented
for what it is — "nodes that have ever had a body", never decreasing, because an
entry has to stay for a late message to be matched by epoch. `tracked()` is what
counts what is still running, and the doc says so.

### S-17 — `R-05` ran each program once

**Fix.** `multi_thread.rs`: `REPEATS = 3` (`SDAX_R05_REPEATS` overrides), applied
to both R-05 tests. **In the fast loop, not as an `#[ignore]` job**, because the
whole of `R-05` costs 0.28 s per pass: three passes plus S-11's new test bring
the file to 0.98 s, which is affordable, and a repetition count nobody runs is
not repetition. The env var is how a longer soak is run without a second test.

## Notes — documents that lied about the code

Nothing here changed behaviour; each row says where the true statement now is.

- **S-08** — contract `T7d` (new, next to T7/T7b/T7c): `Abandoned` records that
  the engine stopped waiting, not that the work stopped; a never-returning
  blocking body keeps its thread and blocks `tokio::runtime::Runtime::drop` for
  ever, while `TokioRuntime::shutdown(budget)`, `shutdown_timeout` and
  `shutdown_background` all answer. Repeated in the crate docs and in
  `TokioRuntime::shutdown`'s own rustdoc, which is where `Err(n)` is read.
  **Not executed here:** the review ran it (B1) and the probe is preserved; this
  round only states it.
- **S-09** — `running.rs` module docs and `Running`'s own: the drainer is a task,
  so on a `current_thread` runtime it makes no progress between `block_on`
  calls, and `C-14`'s 60-second virtual sleep is that fact and not a decoration.
  Also in the crate docs, next to the `Runtime::drop` warning it combines with.
- **S-13** — contract § 10 and the crate docs: under `panic = "abort"` every
  `catch_unwind` in the workspace is inert, a body panic aborts the process,
  `FaultKind::Panic` is unreachable and `R-06` is vacuous. Nothing here sets that
  profile and nothing here can detect it. **Not executed** — a profile change
  rebuilds the workspace, and the disk budget for this round did not allow it.
- **S-14** — `OD-REPORT-OBSERVER` amended to "every run **that was launched**",
  with the reason: `start` is lazy, an unpolled handle is not a run, and that is
  `C-11`. The same correction is in `Observer::report`'s rustdoc and in
  `Running`'s.
- **S-15** — `lib.rs`: "the two call sites" becomes three, and names them;
  `close_and_wait` (folded into `shutdown` by `OD-RT-SHUTDOWN`) is gone from the
  `spawn` comment; `Drop`'s claim that tokio's clock panics outside a runtime is
  replaced by the true reason for `seen` — `Drop` can run on a thread with no
  runtime *and* on a clock of the caller's own, where a fresh reading would not
  be on the run's scale.

## MC-1 — found by re-running the walks, not by the review

`monte_carlo_big` at 50 000 cases on seed `1` failed at case 48 702:

```
WHY: S1 is Waiting at t=6s and names no reason
```

**Not mine:** it reproduces identically with this round's work stashed, at
`7c6257c`. The fast loop walks 3 000 cases on a fixed seed and had never reached
it.

**Pinned** as `9106096978137470251` in `SEEDS_THAT_FOUND_BUGS`, where it replays
in 0.05 s.

**The defect.** A service inside a component held `exclusive` on an imported
resource and was abandoned at its `stop_within` (0 s). `abandon` frees the
node's grants with `release_grants` and nothing else — it never reaches
`after_settle`, because the node never settles — and its caller's
`after_cleanup` only ran `sweep`, which advances cleanups and checks ends but
admits nothing. So a step outside the component, whose only blocker was that
lock, stayed `Waiting` for the rest of the run with an empty `on`: the WHY rule
saw the symptom, and the liveness failure was the cause. `admit_all`'s own
rustdoc describes exactly this shape — it was written for a different path into
it.

**Fix.** `cleanup.rs`, `after_cleanup`: `admit_all()` before `sweep()`.
`admit_all` starts nothing in a scope that is not `Admitting` or `Steady`, so
the budget's own `abandon_all` sweep is untouched — the case that made this
visible is precisely the one where an *inner* scope settles while the parent is
still admitting.

---

## Cost, and one thing deferred

The workspace suite's execution time goes from **3.56 s to 4.80 s** (sum of the
per-binary `finished in` lines), of which ~0.7 s is S-17's repetition and
S-11's new test and the rest is the new regressions plus the substrate orphan
checks S-10 added.

`AGENTS.md` still says the fast loop is "~0.55 s of execution". That figure was
already stale before this round — Stage 3's two Monte Carlo walks alone are
2.3 s of it — and correcting a process budget was not this round's to do.
**Deferred to the lane owner:** either re-measure the number in `AGENTS.md` or
say what the budget is now meant to bound. It is the same class of defect as
S-10 and S-15, in the file that sets the rule.

## S-07b — the sibling snapshot assertion, found by CI (2026-09-06)

`R-04`'s `assert_eq!(hw.now, 0)` and `assert_eq!(rt.tracked(), 0)` are the same defect the
S-07 row fixed in `R-07`: an instant-in-time assertion taken while a pool thread may still be
running `exit()`. The S-07 remediation reached `R-07` and not its sibling.

**RED:** the first CI run after the push (`owebeeone/sdax-rs`, run 34003416805, ubuntu-latest,
4 cores) failed with `assertion left == right failed / left: 1 / right: 0` at
`substrate.rs:130`. It had passed on the 12-core development machine every time, including
the remediation's own gate runs — the race needs fewer cores than the test has bodies.

**GREEN:** both snapshots replaced by the documented idiom — a bounded
`rt.shutdown(secs(5)) == Ok(())` quiescence check, after which the pool's occupancy is
meaningfully zero. Verified locally and under `taskset -c 0-3` (5 runs).

**Lesson for the log:** every remaining snapshot assertion of a concurrent counter is a
suspect. The reviewer said so at S-07; the fix was applied to one site rather than to the
class. The other two `tracked()` assertions (`substrate.rs:173`, `:217`) are on
single-threaded paths and were left, deliberately — noted here so the next round does not
have to rediscover why.
