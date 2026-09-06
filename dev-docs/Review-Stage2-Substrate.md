<!-- Provenance: independent adversarial review, substrate axis, by a Fable agent on 2026-09-06.
     Reviewed commit cacf849 in a disposable detached worktree with its own target dir, so it could
     build and run probes while Stage 3 proceeded in the main tree. The worktree was removed after
     the report and its six probe files were rescued; the probes are in dev-docs/review-probes/.
     Manager confirmed S-01 and S-02 by reading running.rs:241 and driver.rs:305 at cacf849. -->

# Substrate review of `sdax-rs` at `cacf849` — the tokio adapter on a real runtime

Reviewer: independent, adversarial, substrate axis only. Worktree
`/Users/owebeeone/limbo/sdax-wz/.review-worktree` pinned at `cacf849`; nothing
committed. Every probe below lives in `crates/sdax-tokio/tests/probe_{a,b,c,d,e,f}.rs`
in that worktree (untracked, disposable) and every number was produced by running
it on this machine (Apple M3 Pro, macOS 26.6, rustc 1.96.0, tokio 1.53.1) on
2026-09-06. The baseline suite passes: `cargo test --workspace --locked --offline`,
280 tests, plus the six probe binaries.

**Found by running** means a probe was written that would fail if the claim were
false, and it was executed. **Found by reading** means the code path was read and
the case is argued but not executed; where such a case was later executed the entry
says both.

## 0. Verdict

**The adapter delivers what the machine assumes on the paths the machine
exercises.** The two ordering rules hold on a two-worker runtime under direct
attack (F1, B5, C1: an outcome due in the very poll in which the abort is
requested is delivered, a hold registered in that poll is banked, and nothing
stale is ever credited); `hold` is atomic against cancellation on the
multi-thread runtime, not only under paused time; the blocking-pool bound
survives `within` timeouts, retries and mid-run cancellation on two workers
(D2, worst high-water 2 of 2 over 30 runs); the drop guard drains and reports
with no orphan on the multi-thread runtime (D1, 600 random drops, orphans
measured by `TokioRuntime::shutdown`, one report each); and 600 random
cancel/shutdown cases with real bodies, exclusive contention, a restarting
service, a component and a blocking step produced no invariant violation, no
rejection and no orphan (C1). Every panic site the brief lists is contained
(A6–A9, B7 for the observer).

**What is not right is the driver's edges, not its centre.** Two findings are
*major*: an observer callback that panics kills the driver task and orphans
every live body for ever while the `Running` resolves to an empty `Cancelled`
report (B7 — the contract does forbid the observer to panic, but the failure mode
is the one INV-15 exists to prevent and is silent), and tearing down the tokio
runtime while a drainer is pending loses the run *without any signal at all*
unless the adapter happens to be dropped before the runtime (A4 — R-03's witness
passes because of its drop order). One more *major* is a plain hang:
`Running::ready()` on a plan the machine refused never returns (A1), on the very
path `start` is documented to make total. The rest is minor: a blocking cleanup
supplied through the host API is aborted at the budget in a way that detaches its
thread from the tracker so `tracked()` reports zero orphans while a thread runs
(B2), plus a spurious `Reject` on the same path; a double hold followed by `Err`
is not reported; `tracked() == 0` immediately after `Running` resolves is a race
on the multi-thread runtime (non-zero in 264 of 300 runs — B3) that `R-07` asserts
anyway; and several substrate facts the docs should state and do not (a
never-returning blocking body blocks `Runtime::drop` for ever; on
`current_thread` the drainer only progresses inside a `block_on`; `panic =
"abort"` is not mentioned anywhere although the brief believed it was).

On the declared limits: `R-05`'s INV-8 exclusion is *honest but total* — no
bounded-shutdown check of any kind runs on the multi-thread runtime, and a
tolerance-based assertion would cost nothing; its "no orphan — so that is what is
checked" sentence is **false** (nothing reads `tracked()` there); and running
each of eight programs exactly once on two workers is thin for a class of bug the
authors themselves saw at one run in six. None of this hides a defect I could
find: 30 back-to-back runs of `R-05` and 25 of `R-07` were clean, and my own
repetition found nothing the checker missed.

**Verdict: not a blocker for Stage 3; three majors to fix before the adapter is
called done, each with a small fix named below.**

## 1. Findings

| id | sev | one line | where (`cacf849`) | how found |
|---|---|---|---|---|
| S-01 | **major** | `Running::ready()` on a refused plan (unresolved import; a template until Stage 3) hangs for ever — the refused `Running` never latches its readiness signal | `crates/sdax-tokio/src/running.rs:241-249`, `:424-446` | reading, confirmed by running (A1) |
| S-02 | **major** | An `Observer::event` that panics kills the driver task: live bodies are orphaned for ever (a serving service is never stopped), no `Observer::report`, `ready()` hangs, `Running` resolves to an empty `Cancelled` report | `crates/sdax-tokio/src/driver.rs:305-308`, `running.rs:314-317` | reading, confirmed by running (A2, B7) |
| S-03 | **major** | Dropping the tokio runtime while a drainer is pending loses the run silently: no release, no report, not even `DroppedWhileRunning`; `RuntimeDroppedWithLiveRuns` fires only if the adapter is dropped *before* the runtime | `driver.rs:453-472` (no `Drop for Driver`), `lib.rs:106-123` | reading, confirmed by running (A4) |
| S-04 | minor | A host-supplied blocking cleanup (`Task::Blocking` from `BodySource::cleanup`) aborted at the budget (T7b) has its thread **detached from the tracker**: `tracked() == 0` and `shutdown() == Ok` while the thread runs; and every blocking cleanup announces `Started`, which the machine rejects | `lib.rs:136-146`, `:162-168`; `driver.rs:388-390`; `body.rs:171-176` | reading, confirmed by running (B2) |
| S-05 | minor | An observer panic in `report()` turns a finished `Ok` run into an empty `Cancelled` report for the awaiter (`done` is sent after the observer is called) | `driver.rs:466-471` | reading, confirmed by running (A3) |
| S-06 | minor | A body that holds twice and then returns `Err` is reported as a plain `Error`; the first value's obligation is dropped silently (only the `Ok` path checks `hold_count > 1`) | `body.rs:87-95`; `cx.rs:166-169` | reading, confirmed by running (A10) |
| S-07 | minor | `tracked() == 0` immediately after `Running` resolves is a race on `new_multi_thread`: non-zero in **264 of 300** runs (the aborted budget timer, and the driver task itself, not yet reaped); `R-07` asserts it anyway (0 of 25 failures, because its result crosses a oneshot first) | `driver.rs:454-456`; `tests/substrate.rs:305` | running (B3; R-07 loop) |
| S-08 | minor | A never-returning blocking body leaks a thread **and blocks `tokio::runtime::Runtime::drop` for ever**; nothing in the contract, the report or the crate docs says so | T7/T7c wording; `lib.rs` docs | running (B1) |
| S-09 | minor | On `current_thread` a dropped `Running`'s drainer makes no progress between `block_on`s; the C-14 test compensates with a 60 s virtual sleep but users are not told | `running.rs:8-11`, `:207-211` | running (A5) |
| S-10 | minor | `R-05`'s module doc says "no orphan — so that is what is checked"; nothing there reads `tracked()`, `running.await` has no liveness guard, and `stuck: false` is hard-coded | `tests/conformance/multi_thread.rs:13-14`, `:102`, `:117` | reading |
| S-11 | minor | INV-8 is not merely unasserted with tolerance on the multi-thread runtime — no bound at all is checked there; measured timer slop is 7–10 ms per timer on this substrate, so a slack-based assertion at a lower compression factor is feasible | `multi_thread.rs:16-23`, `:121-125`; OD-INV8-CLOCK | running (C3, E1) |
| S-12 | minor | A machine `Reject` (a driver bug by definition) reaches only stderr or a harness `RunRecord`; a production observer and the report never see it | `running.rs:103-111`; `driver.rs:310` | reading, seen in B2's output |
| S-13 | note | `panic = "abort"` is mentioned nowhere in the contract or the reports; under it every `catch_unwind` in `body.rs` is inert and a body panic aborts the process | grep of `dev-docs/` and `crates/` | reading |
| S-14 | note | `OD-REPORT-OBSERVER` says the report reaches the observer "on every run whether it was awaited or dropped"; a `Running` dropped before its first poll produces no report and no event | `running.rs:339-348`; Stage2Report § 6 bug 4 | running (A11) |
| S-15 | note | `lib.rs` doc rot: "the two call sites" (there are three), a comment naming `close_and_wait` (gone), and `Drop`'s claim that tokio's clock panics outside a runtime (it falls back to `std`) | `lib.rs:5-6`, `:130`, `:110-112` | reading |
| S-16 | note | `with_clock` does not rewire `seen`, so `RuntimeDroppedWithLiveRuns` under a custom clock is stamped `Time::ZERO`; `Snapshot::live` counts `Live` entries that are never removed, so it is "nodes that ever had a body", not live tasks | `lib.rs:63-66`, `:117`; `driver.rs:357-365`; `running.rs:52-53` | reading |
| S-17 | note | `R-05` runs each of its eight programs exactly once on two workers; the one multi-thread bug Stage 2 found appeared one run in six | `multi_thread.rs:155-200` | reading; 30 back-to-back runs clean |

Nothing here re-files a remediated Stage 1 item; F-04/F-05/F-07/F-10/F-11 were
checked on the adapter and hold (D2, C1, A9, B2's Reject is the F-10 guard
working).

## 2. Findings in detail

Each entry: the code path, the concrete case, what was run, what happened, the
smallest fix. Probe names (`A1`, `B7`, …) are the test functions in
`crates/sdax-tokio/tests/probe_*.rs`; run one with
`cargo test -p sdax-tokio --test probe_a --offline -- --nocapture a1_`.

### S-01 — major — `Running::ready()` hangs on a refused plan

`running.rs:424-446`: `start_with` on a plan the machine refuses builds a
`Running { state: Done, refused: Some(..) }` with a fresh `ready_sig` and
`ready_state: None`, and nothing ever latches them. `running.rs:241-249`:
`ready()` calls `launch()` — a no-op when `refused.is_some()` — and then awaits
`Stop::on(ready_sig)`, which is never requested.

Case (A1): a child plan with one `import`, started as a root (`L-IMPORTS`).
`tokio::time::timeout(3600 s, running.ready())` under paused time →
`Err(Elapsed)` — the virtual hour auto-advanced with nothing to do, which is a
hang. Awaiting the same `Running` directly resolves to `Failed` as documented,
so `start` is total but `ready` is not.

Fix: in the refused arm set `ready_state: Mutex::new(Some(Err(Outcome::Failed)))`
and call `ready_sig.request()`; or have `ready()` return
`Err(Outcome::Failed)` when `self.refused.is_some()`. One regression test.

### S-02 — major — a panicking observer orphans the run

`driver.rs:305-308`: `Effect::Emit` calls `self.rt.observer().event(&ev)` on
the driver task with no guard. A panic there unwinds the driver task: tokio
catches it, the `Driver` is dropped, every `TokioTask` in `live` is dropped
(dropping a `JoinHandle` detaches, it does not abort), the `done` sender is
dropped, and `running.rs:314-317` turns that into `Report::empty(Cancelled)`.
`Control::ended` is never called, so `ready()` waiters block for ever;
`Observer::report` is never called.

Case (B7): a service serving until `cx.stop()`, `Shutdown::within(200 ms)`,
real clock; an `FnObserver` that panics on `TraceKind::Settling`; `shutdown()`
after `ready()`. Result: `Running` resolved `(Cancelled, 0 faults)`,
`stopped = 0` (the service was never signalled), `tracked() = 1` for ever,
`rt.shutdown(300 ms) = Err(1)`. A2 (panic on `Ready`) additionally shows
`ready()` timing out.

The contract (§ 10) says the observer "must not block and must not panic", so
this is a user-obligation violation — but the consequence is exactly INV-15's
failure ("the engine never detaches child work"), it is silent, and the fix is
one line: wrap the two observer calls (`event` at `driver.rs:306`/`:447`,
`report` at `:466`) in `catch_unwind(AssertUnwindSafe(..))`, and on a panic
record a trace event (`Emit` of a new `TraceKind`, or reuse
`RequestDuringCleanup`-style signal) and continue. If the owner prefers not to
guard, the `Running` result should at least carry a fault naming the observer
rather than an empty `Cancelled`.

### S-03 — major — runtime teardown under a pending drainer is silent

`driver.rs:453-472`: the report reaches the observer only from `finish()`,
which runs only when the loop reaches `End`. There is no `Drop for Driver`.
When the tokio runtime is dropped (or `shutdown_timeout`/`shutdown_background`
is called) while the drainer is pending, tokio drops the driver future: no
`End`, no `finish`, nothing.

Case (A4, `current_thread`, paused): `ready().await` then `drop(running)`
inside a `block_on` that returns immediately, then drop the tokio runtime.
Result, both drop orders: `released = 0`, `stopped = 0`, `reports = 0`, and
**`DroppedWhileRunning` is absent** (the `Msg::Dropped` was queued but never
processed). `RuntimeDroppedWithLiveRuns` appears only when the adapter is
dropped *before* the runtime (`tracked_before = 2`: driver + serve task); with
the runtime dropped first the tracker is already empty and the adapter's `Drop`
is silent. `lib.rs:106-108` promises "reported, never silent"; that holds for
one drop order.

Fix: `impl Drop for Driver` — if `self.ended.is_none()`, push
`TraceKind::RuntimeDroppedWithLiveRuns` (or a new `DriverDropped`) at
`self.machine.now()` to the observer and call `observer.report(&Report::empty(Cancelled))`
with the trace so far. `Machine::now()` is pure and no tokio API is needed, so
this is safe on the shutdown thread. Also state in `Running`'s docs that the
drainer needs the runtime to stay alive for the budget (see S-09).

### S-04 — minor — a blocking cleanup's thread escapes the tracker

`lib.rs:136-146`: `spawn_blocking` returns the handle of a tracked *wrapper*
task that awaits the untracked `spawn_blocking` `JoinHandle`. `lib.rs:162-168`:
`abort()` aborts the wrapper; dropping the inner `JoinHandle` detaches the
thread. `driver.rs:388-390`: `spawn_cleanup` accepts `Task::Blocking` from a
host `BodySource`. T7b makes the machine emit `Abort` for a cleanup at the
budget, so the wrapper is aborted and the thread keeps running, now counted by
nobody. Separately `body.rs:171-176`: `blocking_job` sends `Started`
unconditionally, and the machine refuses `Started` for a node in `Releasing`
(D1).

Case (B2): the plan's own bodies wrapped in a `BodySource` whose `cleanup`
returns a `Task::Blocking` sleeping 300 ms; `Shutdown::within(50 ms)`, real
clock, `shutdown()` at 10 ms. Result: stderr
`the machine refused an event: Started for a node with no body in flight`,
`incomplete = ["R"]`, `tracked_after_await = 0`, `rt.shutdown(50 ms) = Ok(())`
while the body's own flag says `inside = true, done = false`; the thread
finished 300 ms later. `tracked() == 0 ⇒ no orphans` is false on this path.

The plan's own bodies never produce a blocking cleanup, so this is host-API
only. Fix: give `TokioTask` a `blocking: bool` and make `abort()` a no-op for
it (the wrapper then stays tracked until the thread returns, and
`join_aborted` sends a `NodeCancelled` the machine ignores for an `Abandoned`
node); give `blocking_job` an `announce` flag like `run_body`. Or refuse
`Task::Blocking` in `spawn_cleanup` with a `Reject`.

### S-05 — minor — an observer panic in `report()` corrupts the awaiter's report

`driver.rs:466-471`: `observer.report(&report)` runs before `done.send`. A
panic there drops `done`; `running.rs:314-317` answers `Report::empty(Cancelled)`.

Case (A3): a `res_and_service` plan shut down cleanly at 1 s; an observer whose
`report` panics. The release and the stop ran (`released = 1, stopped = 1`),
the trace has `Settling`, and the awaiter got `(Cancelled, 0 faults)` for a run
that ended `Ok`. Fix: send `done` first (clone what the observer needs, or
call the observer under `catch_unwind` as in S-02).

### S-06 — minor — a double hold followed by `Err` is silent

`cx.rs:166-169`: `register` overwrites `held` and bumps `holds`. `body.rs:87-95`:
only `Ok(Ok(()))` is checked for `hold_count() > 1`.

Case (A10): `hold_value(1); hold_value(2); Err(..)`. Result:
`faults = [Error]`, one `Held` trace event, the release received `2`, and the
`Arc<u32>` for `1` was dropped with the body's locals with no release and no
record. The seam's one-value rule was broken and INV-9 ("no silent loss") did
not notice. Fix: in `outcome_event`, when `hold_count() > 1` report
`DoubleHold` whatever the body returned (or push both: the body's own error is
still a fault of the attempt).

### S-07 — minor — `tracked() == 0` after `Running` resolves is a race on two workers

`driver.rs:453-456`: `finish()` aborts the timer tasks but does not join them,
then sends `done` (`:469-471`) while the driver task itself is still tracked.
`Running` therefore resolves *before* the driver's own tidy-up is reaped.

Case (B3): `tiny()` on `new_multi_thread(2)`, 300 runs, `rt.tracked()` read
immediately after `running.await`: **non-zero in 264 of 300** (max 2);
`rt.shutdown(500 ms)` was `Ok(())` every time. On `current_thread` the same
read is reliable only because the scheduler drains the run queue (the aborted
timer) before re-polling the `block_on` future — an accident of
`event_interval`, not a property. `tests/substrate.rs:305` (`R-07`) asserts
`tracked() == 0` after a multi-thread `block_on`; 25 back-to-back runs passed,
because each run's report crosses a oneshot and a task exit first. Fix: make
`finish()` async — `for t in timers { t.abort(); t.join().await }` — so that
"`Running` resolved" implies "every driver-owned task except abandoned bodies
is gone"; keep `TokioRuntime::shutdown` as the documented orphan check.

### S-08 — minor — a never-returning blocking body blocks `Runtime::drop`

Case (B1): a blocking step looping on an external flag, `Shutdown::within(100 ms)`,
real clock, `cancel()` at 20 ms. The run ended `Cancelled` with
`incomplete = ["B"]`, `tracked() = 1`, `rt.shutdown(200 ms) = Err(1)` — all
honest. Then `drop(tokio_rt)` on a helper thread: **still blocked after
500 ms**, and it returned only when the body did. tokio's `Runtime::drop`
waits indefinitely for `spawn_blocking` work; `shutdown_timeout` /
`shutdown_background` do not. Neither T7 ("`Abandoned` while its thread
finishes"), T7c, the Stage 2 report, nor `lib.rs` says that a supervisor which
drops its runtime after such a run hangs. One sentence in the contract's T7c
and in `TokioRuntime::shutdown`'s docs closes it.

### S-09 — minor — the `current_thread` drainer only runs inside a `block_on`

Case (A5): drop a live `Running` inside a `block_on` that then returns.
`released = 0, stopped = 0, tracked = 2` after the `block_on`; unchanged after
100 ms of wall time; `released = 1, tracked = 0, reports = 1` only after a
second `block_on`. `running.rs:8-11` says the drainer is "one engine-owned
task"; it is, and on `current_thread` a task needs a driver thread. The C-14
test (`tests/driver.rs:152-154`) knows this and sleeps 60 virtual seconds
inside its `block_on`; a user is not told. Combined with S-03 this is the
realistic silent-loss path: `block_on(... drop(running) ...)` then
`drop(runtime)`.

### S-10 — minor — `R-05` does not check what its doc says it checks

`tests/conformance/multi_thread.rs:13-14`: "and no orphan — so that is what is
checked". Nothing in `parallel()` reads `TokioRuntime::tracked()` or calls
`shutdown()`; the trace-level INV-15 rule cannot see a substrate orphan (B2 is
the demonstration: a detached thread with a clean trace). `:102`
`running.await` has no timeout, so a multi-thread hang stalls the suite rather
than failing it; `:117` `stuck: false` is hard-coded. Fix: after
`running.await`, `assert_eq!(rt.shutdown(ms(2000)).await, Ok(()))`; wrap the
await in `tokio::time::timeout`; set `stuck` from it.

### S-11 — minor — INV-8 on the multi-thread runtime: unasserted rather than tolerant

`multi_thread.rs:121-125` filters *every* INV-8 violation. The reason given
(a real timer fires at or after its deadline, ×200 compression) is true and
measured here: a bare `tokio::time::sleep(20 ms)` on this substrate took
21–30 ms over 15 samples on both scheduler flavours (E1), and the driver's own
budget timer fired 7 ms late in C3 (`Settling@31 ms`, `Abandoned@138 ms` for
a 100 ms budget). At ×200 that is 1.4–2 engine seconds against 3–10 s budgets,
which is why the check was dropped. But the exclusion is total: a driver that
waited 2× the budget on two workers would pass `R-05`. At ×10 the same slop is
0.1 s; INV-8 could be asserted with, say, 0.5 s of slack at ×10, costing
roughly 0.2 s of wall clock per program. Recommend a `slack` parameter on
the INV-8 checker rule and a lower `FACTOR`, and reword OD-INV8-CLOCK from
"exact only on an exact clock" to "asserted with slack `s` on a real clock".

### S-12 — minor — a `Reject` never reaches a production observer

`running.rs:106-111`: with no `RunRecord` attached a rejection is
`eprintln!`ed; with one attached it is pushed to `rejections`. Neither the
`Observer` nor the `Report` sees it. The contract calls a `Reject` "a driver
bug: log it loudly". B2 shows what "loudly" is: one line on stderr from a
library. Fix: also `Emit` a trace event (a new `TraceKind::Rejected(String)`
is a contract change; until then, `observer.event` with `RequestDuringCleanup`
is wrong and should not be reused) — or at minimum add the rejection count to
`Snapshot`.

### S-13 — note — `panic = "abort"` is unstated

`grep -rn 'panic = ' dev-docs crates` finds nothing. The brief expected the
contract to mention it. Under `panic = "abort"` the four `catch_unwind` sites
in `body.rs` (`:65`, `:177`) and tokio's own task guard are inert: a body panic
aborts the process, R-06 is vacuous, and `FaultKind::Panic` is unreachable.
One sentence in contract § 10 or the `sdax-tokio` crate docs.

### S-14 — note — OD-REPORT-OBSERVER's "every run" excludes an unpolled `Running`

Case (A11): `plan.start(rt)` then `drop` with no poll: `reports = 0`,
`events = 0`, `tracked = 0`. Stage2Report § 6 bug 4 calls this "the laziness
working"; it is, and it is the right behaviour (C-11), but the decision text
says the report reaches the observer "on every run whether it was awaited or
dropped". Reword: "on every run that was launched".

### S-15 — note — `lib.rs` doc rot

`lib.rs:5-6` "the two call sites here carry a scoped `#[allow]`" — there are
three (`:131`, `:139`, `:141`; Stage2Report § 9 says three). `lib.rs:130` "so
`close_and_wait` can account for it" — folded into `shutdown` by
OD-RT-SHUTDOWN. `lib.rs:110-112` "`Drop` can run outside the runtime, where
tokio's own clock panics" — with `test-util` `Instant::now()` falls back to
`std` outside a runtime, and without it `tokio::time::Instant` *is* `std`; the
`seen` mechanism is harmless but the stated reason is wrong.

### S-16 — note — two small semantic gaps in the host surface

`lib.rs:63-66` `with_clock` replaces the clock but not `seen`, so under a
custom clock `RuntimeDroppedWithLiveRuns` is stamped `Time::ZERO` (or the
last reading of a clock no longer in use). `driver.rs:357-365` inserts a
`Live` entry per spawn and never removes one; `running.rs:52-53` documents
`Snapshot::live` as "how many nodes the driver has a task or a context for",
which a reader will take as a liveness count. It is "nodes that ever had a
body". Either remove entries on terminal events or rename the field.

### S-17 — note — `R-05` runs each program once

`multi_thread.rs:155-200`: eight `parallel(..)` calls, one execution each, on
a runtime whose interleavings differ run to run. The one multi-thread bug
Stage 2 found (OD-SERVE-ARM) appeared "1 run in 6". Thirty consecutive runs of
the test were clean here, as were C1 (600) and D1 (600) with real bodies; the
point is not that something is hidden but that a single execution of a race
witness is not a witness. A `PROBE_CASES`-style repetition count (default 3)
would keep the cost under a second.

## 3. The attack, item by item: what was probed, what passed

Each row names the probe that would have failed if the claim were false.

### 3.1 The two ordering rules

*Rule 1, an outcome already due when `Abort` arrives is delivered.* By reading:
`run_body` (`body.rs:103-130`) awaits `Guarded` and sends the outcome in the
**same poll** — there is no yield between the body's `Ready` and `tx.send`, so
tokio's deferred abort either drops the future before that poll (no outcome,
`join` → `Cancelled` → `NodeCancelled`) or after it (outcome sent, `join` →
`Done`, `join_aborted` sends nothing, `body.rs:205`). The gap the brief asks
about — "a body that completes between the abort and the join" — is exactly
the second case, and it is the case the machine's `on_ok` accepts for a
cancelling node (`faults.rs:82-86`: after an `Abort` the body's outcome stands;
after a `Signal` it is the answer to the cancel). By running (**F1**, two
workers, 20 runs): a resource body whose second poll holds, sleeps 40 ms
*inside the poll* and returns `Ready(Ok)`, cancelled at 10–29 ms — every run
was `Start > Held > Ready > ReleaseStart > ReleaseOk`, `released = 1`, no
rejection, `shutdown() == Ok`. The abort was requested mid-poll in every case
and lost to the outcome every time, as the rule requires.

*Rule 2, a superseded attempt's outcome is dropped.* By reading: every
`Msg::Body` carries the epoch of the `Live` entry that spawned it and
`driver.rs:165-167` drops a mismatch. I looked for a sender that could hold a
*matching* epoch for the wrong attempt: `spawn_cleanup` (`driver.rs:369-372`)
**reuses** the prepare attempt's epoch for the cleanup, so a late message from
the prepare task would be accepted while its cleanup runs — but every path to
`Release`/`Compensate` goes through the prepare's terminal event or its join, and
after either the prepare task has nothing left to send (`Held` precedes the
outcome inside one poll; `join_aborted` sends once). The one message that does
reach the machine with a reused epoch is `Started` from a **blocking** cleanup
(S-04), which the machine refuses. A blocking attempt that timed out (T7c) stays
`Running` with its epoch until the thread reports, so its late outcome is
current, not stale — **D2** (30 runs, `within` + retry + cancel on two workers,
pool of 2, six bodies) produced no rejection and never exceeded the bound.
**C1** (600 runs) and the adapter Monte Carlo (250 default) saw no rejection.

### 3.2 Abort is a deferred drop (H3)

By reading: `driver.rs:418-429` aborts and immediately spawns `join_aborted`;
the driver touches nothing of the node until the `NodeCancelled` the joiner
sends, and `settled()` calls `take_held()` only then. The one place the driver
"acts" before a join is `finish()` after an `abandon`: the machine emits
`Abort` and `End` in one batch (T7/T7b), and `finish()` delivers the report
while the aborted future may not yet be dropped. That is the contract's
abandonment: the node is in `incomplete`, its held value is dropped with the
`Driver` afterwards with no release body, its pool grant was released by the
machine at `abandon` (`cleanup.rs:246-262`). A body's late `Drop` therefore
cannot reach the ledger (already banked or abandoned), the grant (already
released), or the report (already sealed). **A9** (a body local whose `Drop`
panics during the abort): `Interrupted{held: true}`, release ran, no fault, no
cleanup failure — OD-PANIC-CANCELLED on the real substrate. **B5 bounded**:
a hold registered in the poll during which the *budget* fired — `Abandoned`,
`incomplete = ["R"]`, `released = 0`, no rejection (the driver had ended, so
the late `Held` was dropped rather than refused). **R-02** already pins
"no release before the join" and passed.

### 3.3 `hold` atomicity on the multi-thread runtime

By reading: `Hold::poll` (`cx.rs:389-401`) registers in the poll that observes
`Ready`; `Guarded::poll` (`body.rs:63-83`) reads `hold_count()` after the
inner poll and sends `Held` **before returning**, so the message is in the
inbox before tokio can drop the future. By running (**B5 unbounded**, two
workers): a body that holds and then sleeps 40 ms inside the same poll while
the driver aborts it — `Held` event present, `Interrupted{held: true}`,
`released = 1`, no rejection. The abort landed as that poll returned, i.e. the
exact window T2 promises does not exist. `Ready` on first poll: same code path,
no await, trivially the same (A10's `hold_value` is that case). Effect future
panics (**A8**): `Fail(Prepare, Panic)`, no `Held`, no compensation, not
`Ambiguous` — a panic is a fault (§ 1). Holds twice: `DoubleHold` on `Ok`
(C-50 is a machine-side test; the driver's mapping is `body.rs:93`), silent on
`Err` (S-06).

### 3.4 The drop guard and the drainer

**C-14** passed; **D1** (600 random drops on two workers, real bodies): every
run reported exactly once to the observer, `DroppedWhileRunning` present,
`held == released + incomplete` by the bodies' own count, and
`TokioRuntime::shutdown(2 s) == Ok(())` — orphans measured on the substrate,
not read from the trace. The guard firing inside a body's own task is safe by
construction: `Running::drop` (`running.rs:339-348`) only sends a message.
The drainer being aborted (= the runtime dropping the driver task) and
`Runtime::shutdown` racing it are S-03; the `current_thread` freeze is S-09.

### 3.5 Blocking pools

**R-04**, **R-05's** `r05_a_blocking_within_…` and **D2** (30 runs, two
workers, pool 2, six bodies, three of them overrunning `within(3 ms)` with
`retry(2)`, cancelled at 2–7 ms): high-water never above 2, no rejection, no
orphan. Never-returning body: S-08. Panicking body (**A6**):
`Fail(Run, Panic)`, `shutdown() == Ok`. Abandoned at the budget while its
thread continues (**B6**): `incomplete = ["B"]`, `tracked() == 1` right
after the run, `shutdown(1 s) == Ok` once the thread returned — honest.

### 3.6 Panics

| site | probe | result |
|---|---|---|
| prepare body (async) | R-06 | `Fail(Run, Panic)`, runtime reusable |
| release body | R-06 | cleanup failure `Panic` |
| blocking body | A6 | `Fail(Run, Panic)`, no escape |
| serve future | A7 | `Fail(Serve, Panic)`, `tracked() == 0` |
| `hold`'s effect future | A8 | `Fail(Prepare, Panic)`, no hold, no compensate |
| body local's `Drop` during abort | A9 | `Interrupted{held}`, release ran, no fault |
| observer `event` | A2, B7 | **driver dies, orphans, empty `Cancelled`** (S-02) |
| observer `report` | A3 | **empty `Cancelled` for an `Ok` run** (S-05) |
| the drainer itself | — | it is the driver task; only the runtime can kill it (S-03) |
| `panic = "abort"` | — | unstated (S-13); not executed (a profile change rebuilds the workspace, and the answer is known: no `catch_unwind` can catch) |

### 3.7 Multi-threading

**C1** — 600 cases on `new_multi_thread(2)`, real clock, real bodies: one
lock with three exclusive users (0–2 ms bodies and releases), a service
restarting on error every 2 ms with `stop_within(10 ms)`, a `Finite`
component with an inner resource and step, a blocking step on `pool(cpu, 1)`
that reads `is_stopping()`, `FailFast`/`Isolate` at random, `cancel` or
`shutdown` at 0–13 ms. Checked per case: every checker rule except INV-8, no
rejection, `shutdown(2 s) == Ok`, `held == released + incomplete`,
`stopped ≤ serving`. Result: 0 failing cases. Coverage over the 600 (cases
containing the kind): `Interrupted{held:true}` 184, `Interrupted{held:false}`
495, `Fail(Serve,Error)` 439 (restarts), `Fail(Stop,Error)` 39, `Skipped`
281, `StopRequested` 126, `RequestDuringCleanup` 76, `Held` 600. **Not
reached by C1: `Abandoned`** (40 ms budget, nothing slow enough) — the
multi-thread budget path is covered once each by B5-bounded, B6, D2 and R-05's
`i16`. **D1** — 600 drops, above. **R-05** ×30 and **R-07** ×25 back to back:
clean. I did not find a multi-thread defect the eight-program subset misses;
what the subset misses is *repetition* and the orphan check (S-10, S-17).

### 3.8 Timers and the clock

By reading: `TimerId`s are allocated from a monotonic counter and never reused
(`state.rs:374-379`), so a re-armed deadline is a new id and a stale fire
cannot hit it; a forgotten id is `Ok` (`machine.rs:174-180`); `CancelTimer`
aborts the task (`driver.rs:300-304`) and a message already queued is the
forgotten case. By running (**C2**): `within(1 s)` and the body's `Ok` at the
same virtual instant, 50 runs — `Ok` every time, no rejection (the body's
sleep was registered first and the timer wheel fires in that order; a tie
going the other way would be `Timeout`, and both are valid schedules). What
paused time hides that a real clock shows: (i) a poll in progress when the
abort is requested — impossible on `current_thread`, the substance of B5/F1;
(ii) S-07's race; (iii) timer slop of 1–10 ms per timer (E1, C3), which is
what OD-INV8-CLOCK is about; (iv) `Started` from a blocking cleanup arriving
after the node left `Running` (S-04). None of (i)–(iii) is a defect; all four
are outside what the paused suite can produce.

### 3.9 The driver's obligations (§ 10)

1. *Body events only for spawned keys; never for a component or a join.*
   `driver.rs:321` `debug_assert_ne!` on `Component` (debug builds only);
   `Machine::body_node` refuses at run time (F-10). A driver that broke it
   would see a `Reject` — on stderr (S-12). Not `debug_assert`ed for `Join`.
2./3. *Due outcomes delivered, stale ones dropped.* § 3.1.
4. *A cleanup is never aborted by the driver.* The driver aborts exactly what
   the machine asks; the machine asks only at the budget (T7b) and moves the
   node to `Abandoned` first, so the joiner's `NodeCancelled` is ignored
   (`faults.rs:487`). A different adapter aborting a cleanup on its own gets
   the F-11 `Reject`. Verified in B2 (blocking cleanup at the budget: no
   INV-7 rejection, only the `Started` one).

The obligations are enforced by the machine's refusals, documented in § 10,
and `debug_assert`ed in one place. The failure for a foreign adapter is loud
only if someone reads stderr (S-12).

## 4. The declared limits, judged (item 10)

| declared limit | where | prudent or hiding something |
|---|---|---|
| `R-05` is eight programs, not 58 | Stage2Report § 5 | **prudent as a size choice, thin as a witness**: each program runs once (S-17); the eight do reach every mechanism. The cheap upgrade is repetition, not breadth. |
| INV-8 not asserted on the compressed clock (OD-INV8-CLOCK) | Stage2Report § 5, § 7 | **honest, but the exclusion is total** (S-11). The slop is real and measured (E1: 1–10 ms per timer); at ×10 with slack it is assertable. As written, a driver that overshoots its budget on two workers passes. |
| trace equality not asserted for `R-05` | `multi_thread.rs:10-14` | prudent: unordered pairs do interleave differently; the differential check covers equality under paused time. |
| "no orphan — so that is what is checked" in `R-05` | `multi_thread.rs:13-14` | **false** (S-10): no substrate orphan check exists on the multi-thread path except `R-07`'s racy `tracked()` read. D1 shows it is cheap to add and passes. |
| `S-02` (exhaustive schedules) not run | Stage2Report § 5 | prudent and stated; semantics axis. |
| a full cold build not measured | Stage2Report § 3 | prudent (disk). |
| MSRV 1.75 verified by review only | Stage2Report § 8.6 | prudent and stated. |
| `Snapshot::nodes` opt-in | Stage2Report § 10.4 | fine; but `Snapshot::live` is mis-described (S-16). |
| a `Running` dropped before its first poll reports nothing — "the laziness working" | Stage2Report § 6 bug 4 | correct behaviour, but OD-REPORT-OBSERVER's wording over-claims (S-14). |
| `R-07` is 4 runs, not the canon's 1 000 | Stage2Report § 5 | stated honestly; the isolation assertion (pool of 1 per run, high-water 4) is the right one. The `tracked() == 0` at the end is the racy part (S-07). |
| `R-04`'s "process-wide `blocking_budget` caps across runs" (canon) | CanonicalTests § 5 | not claimed by Stage 2 and not tested; fine, but say so in the `R-04` row. |
| "a scripted run reports `output: None`" | Stage2Report § 8.3 | prudent. |
| `Joined::Panicked` after an abort reported as `NodeCancelled` | Stage2Report § 8.4 | correct: A9 exercises exactly this and gets `Interrupted`. |
| `R-03`: "a dropped `TokioRuntime` with live tasks emits `RuntimeDroppedWithLiveRuns`" | Stage2Report § 5 | true for the test's drop order only (S-03); the other order is silent. |

Nothing in the "what Stage 2 did not do" table hides a defect; the two
limits that do hide something are the ones that were phrased as checks
(S-10) or as exact-vs-nothing (S-11).

## 5. What I could not settle, and the probe that would

1. **`panic = "abort"`.** Not executed: it needs a profile change and a full
   rebuild of the workspace (disk). The answer is not in doubt — no
   `catch_unwind` can catch under abort — so the finding is the missing
   sentence (S-13). Probe if wanted: `CARGO_PROFILE_TEST_PANIC=abort cargo test
   -p sdax-tokio --test substrate r06` and observe the process abort instead of
   a `Panic` fault.
2. **A body whose poll never returns** (a `std::thread::sleep` inside an async
   body): the abort can never land, `join_aborted` never resolves, the budget
   abandons the node and `End` follows; `tracked()` stays ≥ 2 for ever. By
   reading this is INV-15-honest ("listed as abandoned"). Not run because the
   result is a permanently stuck worker thread in the test process; the probe is
   B1's shape with `std::thread::sleep(Duration::MAX)` in an async `step` and
   `rt.shutdown(100 ms)` expected `Err(2)`.
3. **Rule 2 with a *service restart* racing its own `ServeEnded`** on two
   workers: C1 restarts a service ~440 times in 600 cases with no rejection,
   but I could not force the specific interleaving "`StopService` signalled
   while the restart's start body is mid-poll" deterministically. The probe:
   a start body written like `SlowOk` (holds the worker inside its second
   poll for 40 ms) under `Restart::on_error`, with `shutdown()` timed into
   that window; expect `Interrupted{held:false}` and no `ServeEnded` for a
   node not serving (the OD-SERVE-ARM guard).
4. **`R-05` under load.** All repetition here was on an otherwise idle
   machine; a loaded CI box with ×200 compression could turn the 1–10 ms slop
   into a `within` firing before the body it bounds has been polled once.
   That would be a valid schedule, not a defect, but it might trip a checker
   rule the authors did not expect. Probe: `stress -c 8` alongside 30 runs of
   `r05_the_invariants_hold_on_two_worker_threads`.

## 6. Where the adapter is right and a document is wrong

- `multi_thread.rs:13-14` claims an orphan check that does not exist (S-10).
- `lib.rs:106-108` "reported, never silent" holds for one drop order (S-03).
- `lib.rs:5-6`, `:130`, `:110-112` — counts, a dead name, a wrong reason (S-15).
- OD-REPORT-OBSERVER "every run" (S-14); T7/T7c omit the `Runtime::drop` hang
  (S-08); `Running`'s docs omit the `current_thread` progress condition (S-09).
- The brief's premise that the contract mentions `panic = "abort"`: it does not
  (S-13).
- Stage2Report § 5 `R-05` bullet: "the reason is the clock rather than the
  engine" — confirmed by E1 (bare `sleep` slop with no engine involved). The
  *conclusion* drawn from it (assert nothing) is what S-11 disputes.

## 7. Probe inventory

All in `crates/sdax-tokio/tests/`, untracked, run with
`CARGO_TARGET_DIR=<worktree>/target cargo test -p sdax-tokio --test probe_<x> --offline -- --test-threads=1 --nocapture`.

| file | probes | wall |
|---|---|---|
| `probe_a.rs` | A1 refused `ready()`; A2 observer panic on `Ready`; A3 observer panic in `report`; A4 runtime dropped under the drainer (both orders); A5 `current_thread` drainer freeze; A6 blocking panic; A7/A8/A9 serve, hold-effect and drop panics; A10 double hold + `Err`; A11 unpolled drop | 0.11 s |
| `probe_b.rs` | B1 never-returning blocking body + `Runtime::drop`; B2 blocking cleanup abort; B3 `tracked()` after await ×300 on two workers; B5 hold-during-abort (unbounded / bounded); B6 blocking abandoned at budget; B7 observer panic with a live service | 2.1 s |
| `probe_c.rs` | C1 multi-thread stress, `PROBE_CASES` (150 default, 600 run) with trace-kind coverage; C2 `within` tie ×50 under paused time; C3 budget timing with a stuck thread | 2.3 s (150) |
| `probe_d.rs` | D1 random drops on two workers, `PROBE_CASES` (600 run), orphans via `shutdown()`; D2 pool high-water with `within`/retry/cancel ×30 | 1.8 s (150) |
| `probe_e.rs` | E1 bare timer slop, three flavours | 0.4 s |
| `probe_f.rs` | F1 outcome due in the abort's poll ×20 | 1.3 s |

Loops run from the shell: `r07_…` ×25 (0 failures), `r05_the_invariants_…`
×30 (0 failures), `c3_…` ×3 (Settling 30–31 ms for a 20 ms cancel, budget 7 ms
late, every time).

Disk: 12 GB free before, 11 GB after (six debug test binaries in the
worktree's own `target/`); no target directory was added. The worktree is
otherwise untouched: `git status` shows only this file and the six probes.

---

# Remediation

<!-- Appended 2026-09-06 by the remediation round. The reviewer's text above is
     unchanged; this section is the disposition. The evidence — the RED output
     per finding, the measurements, the walk results — is in
     dev-docs/Stage2-Substrate-Remediation-Log.md. -->

Worked at `7c6257c` (Stage 3 landed; 302 tests green, seven gates green), the
LBT-007 way: the case written as a test first, watched failing with the
predicted symptom, then whatever was actually wrong changed. Result: **310
tests, all seven gates green**. Nothing was committed, tagged or pushed.

| id | sev | disposition | one line |
|---|---|---|---|
| S-01 | major | **fixed** | The refusal latches the readiness state when the handle is built, so `ready()` and `RunHandle::ready()` both answer `Err(Failed)` at once. `OD-REFUSED-READY`. |
| S-02 | major | **fixed** | Every observer callback runs under a panic boundary; a panic is `TraceKind::ObserverPanicked` in the trace and the driver keeps going. The § 10 obligation stands, the run no longer pays for it. `OD-OBSERVER-PANIC`. |
| S-03 | major | **fixed** | `impl Drop for Driver`: the event, a `Cancelled` report with the trace so far, and the readiness latch — in **both** drop orders, because it is the driver's own future that is dropped either way. `OD-DRIVER-DROPPED`. |
| S-04 | minor | **fixed** | `TaskHandle::abort()` is a no-op for a blocking task, so the wrapper stays on the tracker until the thread returns (`OD-BLOCK-ABORT`); `blocking_job` takes an `announce` flag and a cleanup passes `false`. Your fix, both halves. |
| S-05 | minor | **fixed** | With S-02, plus `finish` now latches and sends to the awaiter **before** telling the observer — your first suggestion — so `report()` cannot reach the awaiter at all. |
| S-06 | minor | **fixed** | `DoubleHold` outranks the body's own `Err`; a panic still outranks both, because its payload is carried nowhere else. `OD-DOUBLE-HOLD-ERR`. |
| S-07 | minor | **fixed (test)** | Measured again here: **246 of 300**, max 2. `R-07` now waits for quiescence — `shutdown(5 s) == Ok(())` — rather than dropping the assertion. `finish()` was **not** made async: it cannot help, because the driver task itself is still tracked when it sends `done`. |
| S-08 | note | **document fixed** | Contract `T7d`, next to T7/T7b/T7c, plus the crate docs and `TokioRuntime::shutdown`'s rustdoc, which is where `Err(n)` is read. Stated, not re-executed: your B1 ran it and the probe is preserved. |
| S-09 | note | **document fixed** | `running.rs` module docs, `Running`'s own, and the crate docs: the drainer is a task, so on `current_thread` it makes no progress between `block_on` calls, and `C-14`'s virtual sleep is that fact. |
| S-10 | minor | **fixed (test)** | `R-05` now asserts `rt.shutdown(5 s) == Ok(())` per execution, bounds `running.await` with a 30-second real timeout and derives `stuck` from it. The module doc no longer claims a check it does not have. |
| S-11 | minor | **implemented, at a lower compression** | Your reading was right and your number was conservative: measured here, a bare 20 ms sleep on two workers is 1–11 ms late (median 5.8 ms), so at ×200 the slop is 0.2–2.2 engine seconds and no assertion survives. The checker takes the allowance as a parameter now; a second test runs the two budget-relevant programs at **×20 with 0.5 engine seconds of slack** and checks every invariant, INV-8 included. It has teeth: with the allowance at zero the same run reports 40 ms of overshoot on a 3-second budget, so the slack is ×12 the observed error and a sixth of the smallest budget. **0 failures in 30 runs** (180 executions). `OD-INV8-CLOCK` is amended, and says *absent* for the ×200 pass rather than implying tolerance. |
| S-12 | minor | **fixed** | A rejection is now `TraceKind::Rejected(what)` as well as a record and a stderr line, so it reaches the observer and travels with the report. `OD-REJECT-VISIBLE`. Your point that `RequestDuringCleanup` should not be reused is taken; the variant is new. |
| S-13 | note | **document fixed, not executed** | Contract § 10 and the crate docs now say it. Not executed for the reason you gave: a profile change rebuilds the workspace, and the disk budget for this round did not allow it. |
| S-14 | note | **document fixed** | `OD-REPORT-OBSERVER` now reads "every run **that was launched**", with `C-11` as the reason; same correction in `Observer::report`'s rustdoc and in `Running`'s. |
| S-15 | note | **document fixed** | Three call sites and they are named; `close_and_wait` is gone from the comment; `Drop`'s reason is replaced by the true one — it can run on a thread with no runtime *and* on a caller's own clock, where a fresh reading would not be on the run's scale. |
| S-16 | note | **fixed** | `with_clock` wraps the caller's clock so `seen` is still fed; `Snapshot::live` is renamed `contexts` and documented as "nodes that have ever had a body", with `tracked()` named as the liveness count. |
| S-17 | note | **fixed (test)** | `REPEATS = 3`, `SDAX_R05_REPEATS` to raise it, **in the fast loop** rather than as an `#[ignore]` job: `R-05` costs 0.28 s a pass, three passes plus S-11's new test bring the file to 0.98 s, and a repetition count nobody runs is not repetition. |

## Two predictions that did not reproduce as written

Neither is a defect in the finding; both are recorded because the brief asks for
them and because a future reader comparing the report to the tests would trip.

- **S-02, `ready()` hangs.** True for a panic on `Ready` (your A2). The
  regression test uses B7's shape — a panic on `Settling`, after readiness — and
  there `ready()` had already returned, so that assertion passed even at RED.
  The empty `Cancelled` report is what failed. The bound is kept in the test so
  a regression that moves the panic earlier shows as a timeout, not a stall.
- **S-04, `outcome = Cancelled`.** On this tree the same run ends `Ok` with the
  abandoned release in `incomplete` — a shutdown a `Resident` plan answers is a
  clean end, which is T7b working. The tracker claim, which is the finding, held
  exactly as filed.

## What the re-run walks then found

Re-running `monte_carlo_big` at 50 000 cases on seeds outside the fixed one —
which this remediation round was asked to do, and which the fast loop's 3 000
cases had never reached — surfaced a defect in the **machine**, not the adapter:

> `WHY: S1 is Waiting at t=6s and names no reason` (seed 1, case 48 702)

`abandon` frees a node's grants with `release_grants` and nothing else — it
never reaches `after_settle`, because the node never settles — and its caller's
`after_cleanup` only swept cleanups. So a service abandoned at its `stop_within`
inside a component left a step *outside* the component waiting for ever on a
lock nobody held, with an empty `on`. It reproduces with this round's work
stashed, so it predates it. Fixed in `cleanup.rs` (`after_cleanup` re-admits),
pinned as seed `9106096978137470251`, and the walks are clean after it:
50 000 cases × 3 seeds pure, 20 000 × 3 and 2 000 × 2 on the adapter, and the
pinned-seed replays.

## Not taken up

- **`finish()` made async** (your S-07 fix). It would not make
  "`Running` resolved" imply "every driver-owned task is gone", because the
  driver task itself is still tracked when it sends `done`. `shutdown` is the
  check that can carry the claim, and it is what `R-07` asserts now.
- **Your § 5 probes 2, 3 and 4** — a body whose poll never returns, a service
  restart racing its own `ServeEnded`, `R-05` under `stress -c 8`. Deferred to
  the owner of the next substrate round: 2 leaves a permanently stuck worker in
  the test process, 3 needs an interleaving neither of us could force
  deterministically, and 4 needs a loaded box this round did not have.
