# Stage 1 review — remediation log

The findings of `dev-docs/Review-Stage1-Semantics.md` (a read-only adversarial
review of `c292c86`; no tests were run), worked the LBT-007 way: the reviewer's
predicted-failure case written as a test **first**, watched failing with the
predicted symptom, then the machine, the driver, the checker or the contract
changed — whichever was actually wrong.

Baseline: the Stage 2 working tree, 253 tests green.

Where a prediction did **not** reproduce, the row says so and quotes what
happened instead; no code was changed to match a wrong prediction.

`dev-docs/Review-Stage1-Semantics.md` carries the per-finding disposition in an
appended **Remediation** section; this file carries the evidence.

---

## Tier 1 — blockers

### F-01 — one `blocked_pool` flag orders unrelated pools

**Test first.** `review_r1_a_free_pool_is_not_queued_behind_a_full_one`
(`crates/sdax-testkit/tests/conformance/review.rs`), the reviewer's R-1 verbatim:
pools `cpu(1)` and `io(1)`, `A1 ok@+10` and `A2 ok@+0` on `cpu`, `B ok@+0` on
`io`, no needs anywhere.

**RED**, exactly as predicted (`Start(Run) B` at 10, not 0):

```
  0  t=0      A1     #1 Start(Run)
  1  t=10     A1     #1 Ready
  2  t=10     A2     #1 Start(Run)
  3  t=10     B      #1 Start(Run)
assertion `left == right` failed: INV-1: io is free and B has no needs
  left: Some(10.0)   right: Some(0.0)
```

**Fix.** `crates/sdax/src/host/engine/admit.rs`: `blocked_locks: Vec<usize>` and
`blocked_pool: bool` become one shape — `Vec<(grant, waiter)>` per lock and per
pool. A waiter is refused only when an earlier waiter wanted **the same** lock
or **the same** pool, which is the only ordering INV-1 allows.

**Pinned by** `review_r1_…` (start of `B` at 0, of `A2` at 10) and, from Tier 3,
the new `POOL`/`MUTEX` checker rules over the whole walk.

### F-06 — `Waiting{on}` cannot name a queue-position wait

**Test first.** `review_r1b_a_queued_waiter_names_the_waiter_ahead_of_it`.

The reviewer's shape (`B` with no needs) turned out to be **vacuous**: `B`
becomes need-ready before `A` does, so T1 starts it at t=0 and it is never
`Waiting` at all. The honest FIFO witness needs `B` need-ready in the *same
instant* as `A` and declared after it, so the test gives both a `needs(Db)`:
`Db`, then a resident service `Holder` that keeps `db` exclusively, then
`A .needs(db).exclusive(db).limit(cpu)` and `B .needs(db).limit(cpu)`.

**RED** on that corrected shape (verified by disabling only the new clause in
`waits_on`): `F-06: a FIFO-blocked waiter names the waiter ahead of it, got []`
— i.e. `B` sits in `Waiting { on: [] }` while `cpu` is free.

**Fix.** New `Reason::QueuedBehind` (`crates/sdax/src/view/model.rs`);
`Slot::blocked_by` records the earlier waiter each grant pass
(`admit.rs`); `Machine::waits_on` emits it (`machine.rs`).

**Pinned by** `review_r1b_…` and the `WHY` checker rule (Tier 3), which fails a
case the moment any `Waiting` node reports an empty `on`.

### F-05 — a blocking `within` freed the pool while the thread ran on

**Test first.** `r05_a_blocking_within_does_not_oversubscribe_its_pool`
(`crates/sdax-tokio/tests/substrate.rs`), the reviewer's R-5 as an `R-04`-shaped
high-water mark: pool `cpu(1)`, `B.within(50ms).retry(2)`, attempt 1's thread
sleeps 300 ms past its own deadline. The bound is measured by the bodies, not
read off a trace.

**RED**, on the real tokio adapter:

```
assertion `left == right` failed: cpu(1): the timed-out thread keeps its grant
until it returns
  left: 2   right: 1
```

Two threads inside a pool of one — INV-12 ("a retried node's attempts never
overlap") and INV-15 ("every task the engine spawned has been joined or is
listed as abandoned") both false, and the declared pool exceeded.

**Decision.** The reviewer offered (a) state the over-subscription in the
contract or (b) keep the grant and the attempt open. (a) would have to weaken
INV-12 **and** INV-15, which the brief forbids and which would make `.limit(p)`
mean "usually p". (b) is implemented: a blocking attempt whose `within` expires
records its `Timeout` fault at the deadline and then **stays `Running`, holding
its grants, until the thread reports**; only then are the grants released and
the retry decision taken. A thread cannot be taken back, so the honest reading
of `within` on a blocking step is "fault now, retry when the thread is back".
Recorded in the contract (T7 and `OD-BLOCK-WITHIN`).

**Fix.** `faults.rs`: `on_within_timer`'s blocking branch records the fault and
sets `Slot::timed_out`; `fail_attempt`'s tail becomes `after_failed_attempt`,
which `on_ok`/`on_err` call when the timed-out thread returns.

**Pinned by** `r05_…` (high-water 1, `rt.tracked() == 0`) and the rewritten
`mc_a_timed_out_blocking_attempt_does_not_end_the_next_one`.

**Suite change.** That regression *asserted the overlap* as correct
(`start_attempt("B", 2) == Some(2.0)` while attempt 1's outcome was at 4). Its
schedule now reads: `Fail` at 2, attempt 2 at 4, `Ready` at 6. What the row pins
— attempt 1's ending must not end attempt 2 — is unchanged; the overlap it used
to encode was the defect.

---

## Tier 2 — majors

### F-02 — a nested scope's budget started at its settle

**Test first.** `review_r2_a_nested_budget_does_not_start_before_the_inner_release_can`
and `review_r2b_a_nested_budget_survives_a_slow_parent_release` (R-2 and its
`FailFast` variant).

**RED**, as predicted — the inner release emitted and abandoned in one instant
with 30 s of parent budget in hand:

```
 19  t=20     C        #1 ReleaseStart
 20  t=20     C/Conn   #1 ReleaseStart
 21  t=20     C/Conn   #1 Abandoned
assertion `left == right` failed: 1 s of release inside 30 s of parent budget
  left: Some(20.0)   right: Some(21.0)
```

R-2b differed from the reviewer's prediction in its numbers only: predicted
`abandoned("C/Conn") == Some(3.0)` with `End` at 3; observed `Some(6.0)` with
`End` at 6, because the parent's own `Sess` release runs 3→6 first. The defect
is the one described (started and abandoned in the same instant, 7 s of budget
unused).

**Fix.** `settle.rs` arms the budget only for the root; a nested scope takes the
parent's deadline for its in-flight bodies and no clock of its own.
`cleanup.rs::arm_scope_budget` arms a nested scope's budget in
`open_component` — the first instant its releases may run — `min`-ed with the
parent's so `V-BUDGET-ORDER` still holds.

**Pinned by** `review_r2_…` (`cleanup_end("C/Conn") == 21.0`, `incomplete`
empty) and `review_r2b_…` (`cleanup_end == 7.0`, nothing abandoned).

### F-03 — inner `Isolate` was silently overridden

**Test first.** `review_r3_an_isolate_child_survives_a_fault_off_the_export_path`
(R-3) and `review_r3b_an_isolate_child_still_fails_a_component_on_the_export_path`.

**RED**, as predicted — `C/A` interrupted at 0 though its body was due at 2,
`U` skipped, the component failed:

```
  5  t=0      C      #1 Fail(Prepare, Error)
  6  t=0      U      #1 Skipped { because: C/B }
  7  t=0      C/A    #1 Interrupted { held: false }
assertion `left == right` failed: A finishes
  left: None   right: Some(2.0)
```

**Decision.** The brief allowed either honouring the child's policy or stating
the override. The override is stated nowhere it could be true: a child plan's
`Policy` is one of three arguments to its `build`, and with `Mode` already
vacuous at run time (F-16) documenting a second one as inert makes the surface
a claim the engine does not keep. So the machine now honours it.

**Fix.** `table.rs` records each scope's `export` node. `faults.rs` gains
`inner_fault_reaches(scope)`: under a child's `FailFast` an inner fault always
faults the component (the inner scope has settled); under `Isolate` it faults
the component only when the export can no longer arrive — the export itself
failed, or was skipped as a dependent of what failed. A child that exports
nothing is `Ready` on its inner steady state; the fault is still in the report.

**Pinned by** `review_r3_…` (`C/A` ready at 2, `C/D` skipped because `C/B`, `U`
runs, one fault) and `review_r3b_…` (a dead export still fails the component).

**Suite change.** `mc_a_failed_component_stops_admitting_inside` pinned "nothing
starts inside a component that has already failed" using a child that exports
nothing, so under the corrected semantics its component never fails and its
`Resident` parent legitimately never ends (`stuck: the run stopped short of
End`). The child now exports the node that fails, which is what makes the
component fail; the assertion it pins is unchanged.

### F-04 — a blocking step was never signalled

**Test first.** `review_r_f04_a_blocking_step_is_signalled_at_settle`.

**RED**: the whole effect list for a `@2 cancel` on a pending blocking body is
`Start(Run)`, `SpawnBlocking`, `Settling`, `Timer`, `Abandoned`, `End` — no
`Signal` anywhere, so `cx.is_stopping()` could never read `true`.

**Fix.** `settle.rs::interrupt`'s `Kind::BlockingStep` arm now sets
`cancelling`/`signalled` and pushes `Effect::Signal`. No grace timer and no
abort: a thread cannot be dropped (T7), and the budget remains the only bound.
Because the signal is unconditional, `.cooperative(g)` on a blocking step adds
nothing and its grace can never be spent, so the new validate rule
**`V-BLOCKING-CANCEL`** refuses it rather than accepting a no-op.

**Pinned by** `review_r_f04_…` (a `Signal`, no `Abort`, `Abandoned` at the
budget), `v_blocking_cancel_rejects_a_cooperative_grace_on_a_blocking_step` and
`v_blocking_cancel_allows_a_blocking_step_with_no_cancel_attribute`.

### F-07 — `OD-PANIC-CANCELLED` held only on the signalled path

**Test first.** `r4_a_panic_joined_after_a_drop_abort_is_not_a_fault`
(`crates/sdax/src/tests/machine.rs`), the reviewer's R-4 on `p17_plan`.

**RED**: `Some(Failed { held: false })` where the decision says `Interrupted`.

**Fix.** `machine.rs::on_joined`: a `Panicked` join on a slot that is `Running`
and `cancelling && !signalled` emits `Fail(phase, Panic)` to the trace and calls
`finish_interrupted`, exactly as the signalled path does. A `NodeErr(_, Panic)`
is still a fault — that is the body's own return, delivered because it was
already due.

**Pinned by** `r4_…` (`A` `Interrupted`, `report.faults == ["B"]`).

### F-08 — the `ambiguous` record survived a successful `Ambiguity::Retry`

**Test first.** `review_r7_a_resolved_ambiguity_leaves_a_clean_run` (R-7).

**RED**: `the retry resolved it: [NodeRecord { node: Registration, … attempt: 1 }]`
— `Outcome::Ok`, `is_clean() == false`, `into_result() == Err`.

**Fix.** The record is parked on the slot (`Slot::ambiguity`) and travels with
the faults: `flush_faults` moves it to the report, `ready` drops it. The
checker's INV-9 rule for `Ambiguous` gains the same "absorbed by a later attempt
that reached `Ready`" escape it already grants a fault, so the trace still
carries the ambiguity and the report does not.

**Pinned by** `review_r7_…` (`report.ambiguous` empty, `is_clean()`).

### F-09 — `OD-PERSIST-AMBIG` belonged in the validator

**Fix.** New rule **`V-PERSIST-AMBIG`**: `persistent` with
`on_ambiguous(Compensate)` is a finding, because the pair cancels out and
`V-IDEMPOTENT-REQUIRED` then obliges the author to assert the idempotency of a
compensation that can never run.

**RED** (verified by removing the rule from the dispatch):
`expected Plan { name: "Persist", nodes: 1 } to be rejected at build`.

**Pinned by** `v_persist_ambig_rejects_a_persistent_effect_that_compensates_an_ambiguity`.
The Monte Carlo generator was taught the rule (it produced the pair at case 18
of the default walk: `V-PERSIST-AMBIG: persistent effect "E4" …`), which is the
generator being wrong, not the rule.

### F-10 — body events were not kind-guarded

**Fix.** `Machine::body_node` refuses `Started`/`Held`/`NodeOk`/`NodeErr`/
`NodeCancelled`/`TaskJoined` for a `Component` or a `Join` ("this kind has no
body"), and `Held` for any kind that carries no obligation. A `debug_assert` in
`fail_attempt` states the empty-fault-vector lemma at the one place that could
break it. The obligation is also written into the contract (§ 10, driver rules).

**Pinned by** `f10_a_body_outcome_for_a_component_or_a_join_is_refused`.

### F-11 — a `NodeCancelled` in a cleanup state was swallowed

**Test first.** `r8_a_cancelled_cleanup_body_is_refused` (R-8, machine level).

**RED**: `[]` — accepted, no `Reject`, no trace event; the node stays
`Releasing` until the budget and for ever under `Shutdown::unbounded()`.

**Fix.** `on_cancelled` returns `Err("a cleanup body was cancelled; INV-7
forbids the engine to cancel one")` for `Stopping | Releasing | Compensating |
RetryRelease`. `Abandoned` still returns `Ok`: there the machine issued the
abort itself.

**Pinned by** `r8_…` (a `Reject`, and the node still `Releasing`).

### F-12 — a skip on shutdown/cancel/finite/terminal left no trace

**Fix.** `TraceKind::Skipped { because: Option<NodePath> }`, emitted
unconditionally by `settle` and `skip_scope`. `Eol::skipped` was added to tell
"never skipped" from "skipped with no cause node" apart.

**Pinned by** the new `SKIPPED` checker rule (Tier 3), which now has an event to
check, and by every existing `skipped_because` assertion.

---

## Tier 3 — the checker and the generator

### The five missing checker rules

`crates/sdax-testkit/src/invariants/arbitration.rs` (new; `trace.rs` was already
over the house line). All five are computed from the `PlanView` and the `Trace`
alone, and run on every case of suite (c) and suite (d).

| rule | what it recomputes | why it was needed |
|---|---|---|
| `MUTEX` | two bodies never hold one resource at once unless both named it `shared`; per attempt, over the `[Start, body-end]` interval, and for a service past its `Ready` until its stop | INV-1's lock clause had **no rule at all** |
| `POOL` | at every trace index, the bodies inside a pool are ≤ its limit — every scope's pools, not only the root's | INV-1's pool clause had no rule; a child plan's pools were invisible |
| `WHY` | a `Waiting` node's `on` is non-empty (contract § 2) | the only thing that can see a silent delay |
| `SKIPPED` | a `Skipped{because}` names a node that really ended badly, and no attempt was in flight at the skip | `Skipped` had no rule; any `because` passed |
| `TERMINAL` | a `terminal` service that finished with no stop request ends its scope: `Settling` for the root, and T5 (nothing of that scope starts afterwards) for an inner one | `OD-BACKSTOP`'s shutdown ended the run instead and the case passed |
| `T5-INNER` | nothing starts inside a component after that component's own attempt ended | `trace.rs` checks T5 against the **root**'s `Settling` only |

Two of these needed the view to carry more: `PoolView` gained a `scope` and is
now collected for every scope (it listed only the root plan's pools, with
`NodePath::root(name)` users that were wrong inside a component), and a node's
`exclusive`/`shared` attributes are rendered as **resolved paths**, all of them,
so a lock on an imported resource names the parent's node and two locks on one
node are both visible. `MUTEX` is global as a result — a child node and a parent
node contending for one lock are one pair.

**The reviewer's `WHY` prediction, tested.** § 6 R-6 predicted `WHY` "fails on
the first case with two pools and a full one". It does fail the default
3 000-case walk — at **case 2127**, and for a different reason (F-24 below).
With F-24 fixed and F-01 restored to its `c292c86` form (one `blocked_pool`
flag) and the `QueuedBehind` clause removed, the 3 000-case walk is **clean**:
the walk really could not see F-01, so "reachable in six nodes" is not
"reached by the default walk".

### The four generator gaps

`crates/sdax-testkit/src/mc/{gen,keys}.rs`, each with a coverage floor so the
corner cannot silently stop being reached. Counts are from the 3 000-case fast
loop (`SDAX_MC_SEED=0x5DA0202609060001`).

| corner | how | count | floor |
|---|---|---|---|
| nested component (depth 2) | `child_plan` may allow one component inside itself (35 %), one level only | 17 | 5 |
| two locks on one node | `pick_attrs` may take a second, distinct resource (35 %) in either mode — `V-DUP-ATTR` forbids the same one twice | 25 | 10 |
| a lock on an imported resource | a child's imports now carry the parent key's *resource-ness*, so a child node can lock one; `Table::flatten` resolves it to the parent's node | 54 | 10 |
| a service holds a pool | a resident holder gets a fresh pool with room to spare and never shares one with another service, which is the shape `V-POOL-STARVE` accepts | 370 | 50 |
| queued behind an earlier waiter | already reachable; now *counted*, so `Reason::QueuedBehind` cannot stop being produced | 4 | 2 |

The generator was also taught `V-PERSIST-AMBIG` (it produced the refused pair at
case 18 of the default walk).

`exclusive contention` and `pool wait` are counted from the machine's own `why`
and are therefore **reachability** counters; `MUTEX`, `POOL` and `WHY` are what
check the arbitration. Said so in `monte_carlo.rs`.

### What the big walk found

`SDAX_MC_SEED=20260906 SDAX_MC_CASES=50000` on the release profile.

**F-25 (new) — a lock a parent drops does not re-admit the child waiting for it.**
Case 6200: `WHY: C5/C5/E0 is Waiting at t=5s and names no reason`. A child node
had locked an imported resource `shared`; a parent step held it `exclusive` and
released it at t=5; `after_settle` re-admitted the *releasing node's own scope*
only, so the child waited for a lock nobody held — for ever, and with an empty
`why`, because the holder it would have named was gone.

Reproduced by hand as
`review_f25_a_released_lock_re_admits_every_scope_waiting_for_it`
(RED: `WHY: C/E is Waiting at t=5s and names no reason` **and**
`stuck: the run stopped short of End`). Fixed with `Machine::admit_all`, a
fixpoint over every `Admitting | Steady` scope, called from `after_settle`;
`Machine::starts` counts starts so a pass knows whether it made progress.

**F-24 (new) — a backoff holds the grants its attempt already released.**
Found by `WHY` at case 2127 of the default walk. `backoff` re-admitted the scope
only on the zero-wait path, so a freed lock or pool slot sat idle until an
unrelated backoff timer fired. Reproduced as
`review_f24_a_backoff_does_not_hold_the_grants_its_attempt_released`
(RED: `WHY: B is Waiting at t=0ns and names no reason`, `Start(Run) B` at t=3
for a `cpu(1)` free since t=0). Fixed: `backoff` re-admits on both paths, and
the `RetryRelease` branch of a failed attempt does too.

**After both fixes**, clean:

| walk | result |
|---|---|
| `SDAX_MC_SEED=20260906 SDAX_MC_CASES=50000` (release) | ok, every floor cleared |
| `SDAX_MC_SEED=20260906 SDAX_MC_CASES=200000` (release) | ok, every floor cleared |
| `SDAX_MC_SEED=8675309 SDAX_MC_CASES=100000` (release) | ok |
| `SDAX_MC_SEED=99 SDAX_MC_CASES=20000` (debug, every `debug_assert` live) | ok |

---

## Tier 4 — minors and notes

`F-08` … `F-12` are in Tier 2 above (the manager named `F-09`, `F-10` and `F-11`
as the ones to close). The rest:

| id | disposition |
|---|---|
| F-13 | **contract**, § 1 `.exclusive/.shared`: the grant is held from the start of the prepare/run body to its `Ready`, fault or interrupt; a service keeps it while it serves; release and compensate bodies, including a retried resource's between-attempts release, run unlocked. Holding it through `RetryRelease` was the alternative and would have made a lock outlive the attempt that took it, which no rule asks for. |
| F-14 | **contract**, § 6 `within`: the deadline is a hard abort whatever the cancel mode. A `cooperative` node's grace is for the *request*, not for its own deadline; running the cancel mode at the deadline would make `within` a soft bound, which is the opposite of what it is for. |
| F-15 | **contract**, § 2 run states: a `terminal` service *finishing* is its serve future returning `Ok`. A serve that returned `Err` is not a finish; it is a fault under the scope's policy like any other. |
| F-16 | **contract**, § 2: a child plan's `Mode` is a declaration only — `V-MODE` reads it and nothing at run time does. Said next to the sentence that says a nested scope has its own, and next to the note that a child's `Policy` *is* honoured (`OD-INNER-POLICY`). |
| F-17 | **contract**, § 8: `Outcome::Ok` means only that every node that started settled without a fault; a trace can end `End(Ok)` while the report is unclean. Read `is_clean()`/`into_result()`. |
| F-18 | **contract**, INV-9: the invariant now carries its own exception — a panic in a body the engine had already cancelled is in the trace, is not a fault, and is therefore recorded nowhere when no observer is attached. A `Report.interrupted_panics` list was the alternative; it would add a report field for a case the decision says is not a failure. |
| F-19 | **fixed.** `check_end`'s `Ready → Finished`/`Stopped` arm for a component was unreachable — every route into an inner `Cleanup` goes through `open_component`, which sets `Releasing` first. Replaced with `debug_assert_eq!(st, Releasing)` and an unconditional `Released`, and the checker no longer accepts `Stopped` as a component's discharge. The `Started` for an `Abandoned` node (a late blocking-thread hello) is documented in `Stage1Report.md` § 10's event table with the `TaskJoined` row. |
| F-20 | **contract**, § 1 service row: the obligation exists iff a serve future exists **and** the start body returned before the stop request; a `Serving` returned after the signal is the answer to the cancel and is dropped unpolled. |
| F-21 | **contract**, § 6 `within`: a mid-run retry release has no deadline before `shutdown()`, so a hung one holds a `Finite` run at `Admitting` until a request arrives. Honest under INV-7 (a cleanup body is never dropped by a request), and now said. |
| F-22 | **contract**, § 1 `try_step` row: a panic or a `within` timeout is a fault, not a value; only a returned `Err` is the value. |
| F-23 | **contract**, T7b: after the budget the remaining gated releases are started **in order to be abandoned**, so a driver sees `Release(k)` immediately followed by `Abort(k)`. |

### Not fixed, with the reason

| id | why not |
|---|---|
| § 4.4, rule 2 (`attempt: u32` on body events) | a host-surface change to `Event` plus a matching change in the tokio driver's task table; it turns a driver convention into a machine-checked rule and belongs with the Stage 3 event work (`InstanceSpawned`/`InstanceEnded`), not in a remediation pass. The machine's kind guards (F-10) close the part of it that could corrupt a component. Owner: Stage 3. |
| § 5.1, INV-3/4 (`Interrupted{held}` never compared with the presence of `Held`) | not one of the five rules the brief named, and the shape it would catch — a machine claiming `held: false` after a `Held` — is already caught in the other direction by INV-3 (a held node's obligation must run). Owner: next checker pass. |
| § 5.1, INV-7 (nothing checks that abandonment happens at `Settling.at + budget`) | the same: not named, and INV-8 already bounds the end. A machine that abandons *early* is the gap; it needs the budget arithmetic in the checker, which is a rule about the plan's `Shutdown` rather than about the trace's shape. Owner: next checker pass. |
| `crates/sdax/src/tests/planner_validate.rs` is over the ~500-line house limit (586) | it was already over (541) before this pass; splitting the budget/policy rows into their own module is a tidy-up, not a remediation, and moving 300 lines of tests at the end of a semantics pass buys nothing. Owner: next housekeeping pass. |

### The `held` lemma

§ 4.2 recommended asserting it. `end_component_attempt` now carries
`debug_assert!(self.slots[c].started, "St::Running implies started")`, and
`fail_attempt` carries `debug_assert!(kind != Kind::Component)` for the
empty-fault-vector lemma of § 4.3. A 20 000-case debug walk
(`SDAX_MC_SEED=99`) runs with both live and is clean.
