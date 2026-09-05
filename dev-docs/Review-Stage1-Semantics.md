# Review — Stage 1 machine, semantics axis (adversarial, read-only)

**Commit reviewed:** `c292c86` (`sdax-rs: extract engine exit helpers`). Every
`path:line` below is as of that commit, read with `git show c292c86:<path>`.

**No tests were run.** No `cargo` command of any kind was executed; the working
tree was not read (a Stage 2 edit is in flight there); nothing outside this file
was written. Where a claim needs a run to settle, § 6 says so and gives the exact
test.

Sources read: `dev-docs/SdaxContract-v1.md`, `dev-docs/Stage1Report.md`,
`dev-docs/Stage1-TDD-Log.md`, `dev-docs/Review-2026-09-06-External.md`, all of
`crates/sdax/src/host/engine/`, `crates/sdax/src/sim/`, `cx.rs`, `plan.rs`,
`policy.rs`, `report.rs`, `builder.rs`, `view.rs`, `view/model.rs`,
`validate/budgets.rs`, `crates/sdax-testkit/src/{driver,invariants,invariants/trace,mc/*}.rs`,
`crates/sdax-testkit/tests/monte_carlo.rs`, the conformance modules named
inline, and — for what the design owed — `sdax-v1/B/Proposal.md` § A.2 and
LG-4…7, `sdax-v1/B/CanonicalTests.md`, `sdax-v1/corpus/Intents.md`,
`sdax-v1/manager/AdversarialProbes.md`. `sdax-v1/heldout/` was not opened.

---

## 0. Verdict

The machine is a careful piece of work and most of what the contract promises
is enforced by construction or by a check I can name; the row-13 collapse of
the three ramp rules is complete, and both lemmas it rests on hold at this
commit. It is nevertheless **not yet the machine the contract describes**, on
two counts that matter and several that will bite at 3 a.m. First, T1's grant
loop conflates every pool into one "blocked" flag (`admit.rs:56,78,85`), so a
waiter for a *free* pool is queued behind a waiter for a *full* one — an
ordering INV-1 forbids, with no signal: the node sits in `Waiting{on: []}`.
Second, a nested scope's shutdown budget starts when the *inner* scope settles
(`settle.rs:31-40`), which under a component that faults or finishes early while
a parent resource still depends on it can be hours before its release graph is
allowed to open; when the parent finally reaches it, every inner release is
started and abandoned in the same instant (`cleanup.rs:144-146,335-342`). Around
those two sit a blocking-step story the contract does not tell (never
signalled; `within` frees the pool grant while the thread runs on, so attempts
overlap and INV-15 is false for that thread), an inner `Isolate` that is
silently overridden by `component_faulted`, and an `OD-PANIC-CANCELLED` that
the machine implements for the signalled path only. The checker is genuinely
independent for what it checks, but it does not check lock or pool exclusion,
`why`, `Skipped`, `terminal`, or anything about inner-scope settling, and the
Monte Carlo generator cannot reach nested components, two locks on one node, a
service on a pool, or a lock on an imported resource — so the 200 000-case walk
says nothing about the first finding, which is reachable in six nodes.

## 1. Findings

| id | severity | one line | where (c292c86) |
|---|---|---|---|
| F-01 | **blocker** | One `blocked_pool` flag for every pool: a waiter for a free pool is queued behind a waiter blocked on a different pool (or on a lock). INV-1 false; silent (`Waiting{on: []}`). | `admit.rs:55-86`, `machine.rs:233-238` |
| F-02 | **major** | A nested scope's budget starts at its own settle, not when its release graph may open; a component that faults or finishes early behind a live parent dependent has every inner release started-and-abandoned when the parent's cleanup reaches it. Signalled via `incomplete`, but INV-3's intent is defeated with budget to spare. | `settle.rs:27-40`, `cleanup.rs:298-317,144-146,335-342`, `cleanup.rs:188-209` |
| F-03 | **major** | Inner `Policy::Isolate` is overridden: the first inner fault fails the component and settles the inner scope; the contract says nothing, and the code cites a contract section that does not exist. | `faults.rs:280-309`, `exits.rs:71-80`, contract `:25,38` |
| F-04 | **major** | A blocking step is never signalled — not at settle, not at the budget — so `cx.is_stopping()` (§ 5 "all phases") is dead in a blocking body and `.cooperative(g)` on one is accepted and ignored. | `settle.rs:117`, `cleanup.rs:220-223`, `builder.rs:174,379-383`, contract `:156` |
| F-05 | **blocker** (realised in Stage 2) | `within` on a blocking step fails the attempt and frees its pool grant while the thread runs on; a retry then runs concurrently with it. INV-12 ("attempts never overlap") and INV-15 (the thread is neither joined nor listed) are false for blocking steps; the pool is over-subscribed. | `faults.rs:314-323,187-192`, `admit.rs:134-151`, `regressions.rs:547-576` |
| F-06 | **major** | `Waiting{on}` / `why` omit the queue-position reason, so a FIFO-blocked waiter reports no reason at all. Contract § 2 "`on` lists each reason" is false. | `machine.rs:204-246`, contract `:75` |
| F-07 | **major** | `OD-PANIC-CANCELLED` is implemented only when `signalled`; a `TaskJoined{Panicked}` (or `NodeErr(Panic)`) after a drop-mode `Abort` is a fault and `Failed`, contradicting the decision as written. | `machine.rs:121-129`, `faults.rs:121-135`, contract `:405` |
| F-08 | minor | The `ambiguous` record is pushed eagerly, so an `Ambiguity::Retry` effect whose retry succeeds still ends unclean; `Retry` can never yield a clean run after one timeout. | `faults.rs:338-358`, `faults.rs:105-117` |
| F-09 | minor | `OD-PERSIST-AMBIG` should be `V-PERSIST-AMBIG`: the accepted pair also forces a meaningless `.idempotent()` via `V-IDEMPOTENT-REQUIRED`. | `state.rs:382-387`, `validate/budgets.rs:16-48`, contract `:406` |
| F-10 | minor | Body events are not kind-guarded: `NodeErr`/`NodeCancelled`/`NodeOk`/`Held` for a component or join are accepted, and the empty-fault-vector lemma then rests on driver discipline alone. | `machine.rs:67-112`, `faults.rs:63-70,121-135,402-412` |
| F-11 | minor | `NodeCancelled` while `Releasing`/`Compensating`/`Stopping` is accepted silently; the node stays in that state until the budget, forever under `unbounded()`. | `faults.rs:409` |
| F-12 | minor | A skip on shutdown/cancel/finite/terminal leaves no trace event (`Skipped` needs a `because`); the trace cannot say the node never ran. | `settle.rs:48-60,94-102`, `report.rs:169-173` |
| F-13 | minor | Lock extent is unstated and short: grants are released at `Ready`/fail, so release and compensate bodies run unlocked, and a `RetryRelease` release runs concurrently with the next exclusive holder. | `faults.rs:105-108,187-192,202-211` |
| F-14 | note | `within` ignores the cancel mode: a cooperative node or a service start body is hard-aborted at its deadline with no grace. | `faults.rs:314-328` |
| F-15 | note | A `terminal` service whose serve returns `Err` does not end the scope under `Isolate`. | `faults.rs:428-436` |
| F-16 | note | A child plan's `Mode` has no run-time effect; only the root's is read. | `admit.rs:221-226` |
| F-17 | note | `Outcome::Ok` is reported with cleanup failures, abandonments or ambiguities present; `End(Ok)` in the trace while `is_clean()` is false. | `state.rs:389-396` |
| F-18 | note | `OD-PANIC-CANCELLED`'s "the panic is in the trace" is nothing when tracing is off (`Report.trace: Option`). INV-9 as written is then violated. | `report.rs:269-270`, `faults.rs:126-129` |
| F-19 | note | Dead branch: `check_end` `Ready → Finished`/`Stopped` for a component is unreachable (`open_component` always sets `Releasing` first); the checker accepts the dead shape. | `cleanup.rs:384-390,188-202`, `trace.rs:140-145,521-526` |
| F-20 | note | A `Serving` returned after a `Signal` is discarded unpolled (`Interrupted{held:false}`, nothing owed). | `faults.rs:77,333-379` |
| F-21 | note | No deadline bounds a mid-run retry release (`RetryRelease`) before shutdown; a hung one stalls a `Finite` run forever. | `faults.rs:202-211,220-233`, `settle.rs:72` |
| F-22 | note | A `try_step`'s timeout or panic is a fault (only `Err` is a value); unstated in the § 1 table. | `simulator.rs:262-270`, `faults.rs:132-135` |
| F-23 | note | After the budget, gated releases are started and abandoned in the same instant (C-56 asks for it; the contract does not say it). | `cleanup.rs:128-147,319-342` |

## 2. Findings in detail

### F-01 — blocker — one `blocked_pool` flag orders unrelated pools (INV-1 false, silent)

**Evidence.** `admit.rs:55-86` is the grant loop. It keeps `blocked_locks:
Vec<usize>` per resource but a single `blocked_pool: bool` for the whole scope:

- `admit.rs:56` `let mut blocked_pool = false;`
- `admit.rs:76-78` `let wants_pool = self.t.nodes[n].pool.is_some(); let blocked = … || (wants_pool && blocked_pool);`
- `admit.rs:84-85` on any block: `blocked_locks.extend(wants); blocked_pool |= wants_pool;`

So once *any* earlier waiter that names *any* pool is blocked — on a full pool,
or on a lock it also wants — every later waiter that names *any* pool is
refused, whatever pool it names and whether that pool has capacity. INV-1
(`SdaxContract-v1.md:114`) promises "no other start ordering except lock and
pool arbitration among nodes that declare the **same** lock or pool". T1
(`:99`) says "FIFO among waiters" — of the same grant. This is neither.

The `why` side (`machine.rs:233-238`) lists a pool only when
`pools[p] >= decl.limit`, so the refused waiter's `Waiting{on}` is **empty**;
nothing in the trace marks the delay. Silent.

**Failing case** (six lines, `Plan::simulate`, default `Schedule::Fifo`):

```text
plan "HOL": Policy::FailFast, Shutdown::within(10s), Mode::Finite
  pool cpu(1); pool io(1)
  A1: step .limit(cpu)            body ok@+10
  A2: step .limit(cpu)            body ok@+0
  B : step .limit(io)             body ok@+0     (no needs)
```

Contract: B has no needs and `io` is free, so B starts at t=0. Machine:
`begin` → `admit(0)` queues A1, A2, B in declaration order (`admit.rs:34-45`);
A1 takes `cpu` and starts; A2 finds `cpu` full → `blocked_pool = true`; B →
`wants_pool && blocked_pool` → not started. Nothing re-admits until A1's
`NodeOk` at t=10 (`faults.rs:105-116` → `after_settle` → `admit`), when A2 takes
`cpu` and, `blocked_pool` now false, B finally takes `io`. Expected trace:
`Start(Run) B` at 10, not 0. `state_of("B")` throughout is `Waiting { on: [] }`.
`V-POOL-STARVE` (`budgets.rs:71-130`) does not fire: no resident holder.

A permanent variant needs only a resident lock holder: a service `S` that
`needs db` and `.exclusive(db)` keeps the lock while serving
(`faults.rs:105-108` releases grants at `Ready` for every kind *but* services);
a step `A` `.exclusive(db).limit(cpu)` waits on `db` forever and, through the
one flag, every later `.limit(io)` waiter waits forever with it, with
`why = []`. `build` accepts all of it — there is no lock analogue of
`V-POOL-STARVE`.

**Smallest fix.** Make `blocked_pool` a `Vec<usize>` keyed like `blocked_locks`
(the pool index within the scope): `admit.rs:56` `let mut blocked_pools:
Vec<usize> = Vec::new();`, `:78` `pool.is_some_and(|p| blocked_pools.contains(&p))`,
`:85` `if let Some(p) = pool { blocked_pools.push(p) }`. Then add the checker
rule V-01 (§ 5) so the walk can see this class at all. F-06 is the companion
fix on the `why` side.

### F-02 — major — a nested scope's budget starts at its settle, not at its cleanup

**Evidence.** `settle()` (`settle.rs:15-82`) arms the scope's budget timer the
moment it enters `Settling`, for the root and for an inner scope alike:
`settle.rs:28-40` computes `deadline = min(own, parent's)` and sets
`Purpose::Budget(scope)`. An inner scope enters `Settling` by itself in three
ways that do not involve the parent's cleanup: a `terminal` service finishing
(`faults.rs:432-434` → `settle(scope, Cause::Terminal)`), an inner fault under
inner `FailFast` (`faults.rs:266-278` → `settle(scope, Cause::Fault)`), and
`component_faulted` (`faults.rs:299` → `settle_or_skip_inner` → `settle(inner,
Cause::Parent)`). Its *release graph*, however, cannot open until the parent's
gate allows: `try_cleanup(inner)` at `Settling` (`cleanup.rs:103-116`) calls
`open_component(c)` only if `slots[c].st == Releasing || gate_open(c)`, and
`gate_open` (`cleanup.rs:78-83`) is closed while any parent dependent
`blocks_release` — which a `Ready`, held parent resource does for the parent's
whole life (`cleanup.rs:64-76`, `owes` at `:47-48`).

When the inner budget fires meanwhile, `on_budget_timer(inner)`
(`cleanup.rs:298-317`) abandons whatever is running (nothing, typically: the
inner resources are `Ready`, not `Releasing`) and sets `spent = true`
(`:304`). Hours later the parent cleans up, `open_component` (`cleanup.rs:203-208`)
moves the inner scope to `Cleanup` and calls `advance_cleanup(inner)`, which
starts every gated release (`:132-143`) and then — because `spent` —
`arm_zero` (`:144-146`); the zero timer (`:335-342`) calls
`abandon_all(inner, false)`, which abandons every `Releasing` node (`:255-257`).
Net effect: every inner release is emitted and aborted in the same instant, with
the parent's budget untouched.

**Failing case** (`Plan::simulate`; a run is needed only to print the trace):

```text
child "Child": Policy::FailFast, Shutdown::within(2s), Mode::Resident
  Conn: resource                         acquire ok@+0; release ok@+1
  Svc : service needs Conn (no restart)  start ok@+0; serve err@+3 "dies"
parent "P": Policy::Isolate, Shutdown::within(30s), Mode::Resident
  C   : component(Child)
  Sess: resource needs C                 ok@+0; release ok@+0
  Api : service needs Sess               stop_within 1s; serve stops@+0
script: @20 shutdown
```

t=0: inner steady, `C` Ready, `Sess` held, `Api` serving. t=3: `Svc` serve
`Err` → `on_serve_ended` (`faults.rs:437-462`) → `node_failed` → inner FailFast
`settle(inner, Fault)` → inner deadline t=5 (`settle.rs:31`; the parent has
none yet). `component_faulted(C, Svc)` → `C` `Failed`, parent `Isolate` →
`skip_dependents(C)` skips nothing (`Sess` is `Ready`). `try_cleanup(inner)`:
`gate_open(C)` is false because `Sess` `owes` → `try_cleanup(parent)` is a
no-op at `Steady`. t=5: inner budget → `spent`. t=20: shutdown; parent cleanup
stops `Api`, releases `Sess`, opens `C`: inner `Cleanup`, `Release(Conn)`
emitted, zero timer, `Conn` **Abandoned**, `incomplete = {Child/Conn}` — with
1 s of release owed and 30 s of parent budget in hand.

The `FailFast`-parent shape is the same defect with a shorter fuse: parent
budget 10 s, child 2 s, a parent resource depending on `C` whose own release
takes 3 s; `V-BUDGET-ORDER` (`budgets.rs:203-255`, contract `:250`) says the
child's budget nests in the parent's, but the machine starts the child's clock
before the child can do anything with it.

**What the contract says.** T7 (`:105`) "the shutdown budget starts at the
transition into `Settling`" is written for the root; for a nested scope it is
silent, and INV-8 cannot even apply (an inner `Settling → End` is gated by
INV-5). So this is a contract gap the machine filled the wrong way.

**Smallest fix.** Arm a *non-root* scope's budget in `open_component`
(`cleanup.rs:203-208`), when its cleanup can actually run, not in `settle`;
keep the in-flight-body abort at the inner settle bounded by the *parent's*
deadline only (it is already `min`-ed in). One sentence in T7: "a nested
scope's budget starts when its release graph opens (T6), and never outlives its
parent's."

### F-03 — major — inner `Isolate` is overridden by `component_faulted`

**Evidence.** `node_failed` (`faults.rs:266-278`) applies the *inner* policy
(`:270-273`: `Isolate → skip_dependents`) and then unconditionally calls
`component_faulted(c, n)` (`:274-276`). `component_faulted` (`:282-309`) marks
the component `Failed` (`:286`), records an `InnerFault` (`:288-293`), and
calls `settle_or_skip_inner(c, Some(inner))` (`:299`), which for an
`Admitting | Steady` inner scope is `settle(inner, Cause::Parent(..))`
(`exits.rs:77`): every inner `Running` node is interrupted and every
`Pending | Waiting` one skipped (`settle.rs:46-81`). The comment at
`faults.rs:280-281` cites "contract § 11: inner fault propagation"; § 11 of
`SdaxContract-v1.md` is the stage table and says nothing of the kind, and the
component row (`:38`) says only "becomes Ready when the inner run reaches
steady state". Contract `:25` says a plan is a scope whose nodes "share one
fail policy" — the child plan's declared `Isolate` is that policy, and it is
not what runs.

**Failing case.**

```text
child "Child": Policy::Isolate, Shutdown::within(4s), Mode::Finite
  A: step               ok@+2
  B: step               fail@+0 "b"
  D: step needs B       ok@+0
  export A
parent "P": Policy::Isolate, Shutdown::within(10s), Mode::Resident
  C: component(Child)
  U: step needs C       ok@+0
script: @6 shutdown
```

By the child's own declaration (`Isolate`, C-59 `faults.rs` conformance
`:171-189` is the root-level shape) `B` fails, `D` is skipped, `A` finishes at
t=2, the inner run is steady with one fault, and `C` — whose export `A` is fine
— becomes Ready so `U` runs. Machine: at t=0 `B` fails → `component_faulted`
→ `C` `Failed`, inner scope settled → `A` **Interrupted** (`settle.rs:71`), `D`
Skipped, `U` Skipped (`faults.rs:303`). `U` never runs; the report carries
`P/C` and `P/C/B` faults; `A`'s work is thrown away. Nothing in the suite has
an inner `Isolate` with a survivable fault: `c58` (`components.rs:59-96`) uses
`i32()`, whose child is `FailFast` (`corpus.rs:347`), and the `mc_*`
regressions with `Isolate` children all *assert* the settle
(`regressions.rs:326-361`).

This is also where TDD row 5's fix landed (`Stage1-TDD-Log.md:24`, "the failed
component's inner scope is now settled"): the T5 violation it cured was the
*parent* having settled under `FailFast` while the inner `Isolate` scope kept
admitting — which `settle.rs:80` now handles for every node — so the
`component_faulted` settle is no longer needed for that case and only survives
to override inner `Isolate`.

**Smallest fix.** Either (a) the contract states it: "an inner fault faults the
component whatever the child's policy; the child's `Isolate` governs only which
inner nodes are skipped before the component's release opens" — and then
`settle_or_skip_inner` at `faults.rs:299` is the documented behaviour; or (b)
the machine honours the child's policy: in `node_failed`, call
`component_faulted` only when the inner scope is `FailFast` (or when the failed
node is on the export's need-closure), and let an `Isolate` child reach
`Steady` with faults, at which point `check_steady` (`admit.rs:227-234`) makes
the component Ready and the faults stay in the report. (b) also needs a rule for
"the export itself failed" (component `Failed`, not Ready). Either way the
citation at `faults.rs:280-281` must point at real text.

### F-04 — major — a blocking step is never signalled

**Evidence.** `interrupt()` (`settle.rs:111-162`) matches
`Kind::BlockingStep => {}` (`:117`): no `Effect::Signal`, no timer, nothing.
`abandon()` (`cleanup.rs:220-235`) skips the `Abort` for a blocking step
(`:221-223`) and emits no `Signal` either. Yet the builder exposes
`.cooperative(g)` on every `Node<'_, D, K>` (`builder.rs:379-383`), including
`Node<'_, (), Blocking<NoPool>>` (`:174-176`), and records
`cancel: cooperative(g)` in `inspect()` (`view.rs:362`). Contract § 5
(`:156`) lists `cx.stop()` / `cx.until_stop` / `cx.is_stopping()` as available
in **all phases**; a blocking body has a `Cx` and can poll
`is_stopping()` (`cx.rs`, `Cx::is_stopping`), and it will never read `true`
before the run ends. T5 (`:103`) says "every `Running` non-service node is
cancelled per its cancel mode" — a blocking step is a non-service node with a
cancel mode, and it is not.

**Failing case.** `i29`-shaped plan (`corpus.rs:312-333`) with
`Verify.cooperative(1s)` and a body `while !cx.is_stopping() { chunk() }`;
script `@2 cancel`. Expected by § 5/T5: `Signal(Verify)` at 2, the body
returns, `Interrupted`. Machine: no `Signal`; `Verify` stays `Running` until
the budget (`on_budget_timer`, `cleanup.rs:298-317`) marks it `Abandoned` at
12, as C-68 (`cancel.rs:138-…`) already asserts for the non-cooperative body.
In Stage 2 the thread runs to completion or forever regardless.

**Smallest fix.** `settle.rs:117`: `Kind::BlockingStep => { self.slots[n].cancelling = true; self.fx.push(Effect::Signal(key)); }`
(no timer: it cannot be aborted, the budget still abandons it), and the same
`Signal` in `abandon` before the state flip. One sentence in T5: "a blocking
body is signalled and never aborted". Or, if the owner prefers the current
truth, refuse `.cooperative` on `Blocking<_>` at the type level and say in § 5
that the stop request never reaches a blocking body.

### F-05 — blocker (Stage 2 realisation) — blocking `within` frees the pool and lets attempts overlap

**Evidence.** `on_within_timer` (`faults.rs:314-328`) for a blocking step calls
`fail_attempt(n, Run, Timeout)` at once (`:319-323`; "the thread's later outcome
is ignored"). `fail_attempt` (`:187-216`) releases the node's grants at `:192`
— the pool count is decremented (`admit.rs:147-150`) — and, if attempts remain
and the scope admits, schedules the next attempt (`:212-215`). The thread of
attempt 1 is still running: T7 (`:105`) says so for the *budget* case and calls
the node `Abandoned`; here it is neither `Abandoned` nor in `incomplete` — it is
a `Fail(Run, Timeout)` and a fresh `SpawnBlocking` (`admit.rs:180-184`). The
regression `regressions.rs:547-576` pins exactly this schedule and asserts
`start_attempt("B", 2) == Some(2.0)` while attempt 1's thread ends at 4 —
i.e. the suite *asserts* the overlap; the fix in TDD row 11 (`Stage1-TDD-Log.md:30`)
was to make the simulator drop the stale outcome, not to stop the overlap.

Consequences, all against written text: INV-12 (`:125`, "a retried node's
attempts never overlap") is false for blocking steps; INV-15 (`:128`, "every
task the engine spawned has been joined or is listed as abandoned") is false at
`End` for the zombie thread (bug 21's trace ends `Ok` with a thread alive);
the pool limit (`inspect()` shows `cpu(1)`) is exceeded by one per timed-out
attempt, which `R-04` (`CanonicalTests.md` § 5, "at most two threads inside the
bodies at any time") will measure as a failure the moment Stage 2 runs it.

**Failing case.** The pinned regression itself, read against the pool: pool
`cpu(1)`, `B.within(2s).retry(3)`, attempt 1's thread ends at 4, attempt 2
starts at 2 — two threads in a pool of one from t=2 to t=4. With a sibling `B2`
on the same pool declared after `B`, `B2` is granted at 2 while attempt 1 still
occupies the only thread.

**Smallest fix.** Two options; the contract must pick one. (a) State it: "a
blocking attempt that exceeds `within` is recorded as a `Timeout` fault; its
thread is not joined and is listed in `incomplete` as abandoned; its pool grant
is held until the thread returns" — then `fail_attempt` must *not* call
`release_grants` for a blocking timeout and must push a `NodeRecord` to
`incomplete`, and the grant is released on the stale `NodeOk/NodeErr` that
`on_ok`/`on_err` currently swallow (`faults.rs:99,168`), which requires the
driver to *forward* the stale outcome (contradicting ordering rule 2,
`Stage1Report.md:521`) or the events to carry `attempt` (`Stage1Report.md:459`,
open question 5). (b) Keep the grant and the attempt open: on a blocking
`within`, record the fault but leave the slot `Running` (a new sub-state
`Zombie`) until the thread's outcome arrives, and only then release the grant
and start attempt k+1 — INV-12 then holds literally and `within` on a blocking
step becomes "fault now, retry when the thread is back". (b) is the smaller
change and needs no event vocabulary change.

### F-06 — major — `Waiting{on}` / `why` cannot name a queue-position wait

**Evidence.** `waits_on` (`machine.rs:204-246`) lists: needs not
`Ready | Finished` (`:210-218`), the exclusive/shared holders of the locks the
node wants (`:220-232`), a pool only when full (`:233-238`), and
`ScopeNotAdmitting` (`:239-244`). It has no clause for "an earlier waiter's
unmet want blocks this grant" — the FIFO rule at `admit.rs:46-47,77-86` — so a
node refused for that reason renders as `Waiting { on: [] }`
(`machine.rs:252-254`). Contract `:75` says `on` "lists each reason".
`Reason` (`view/model.rs:60-73`) has no variant for it.

**Failing case.** Any node blocked by F-01's flag, and — independent of F-01 —
the honest FIFO case: `A` `.exclusive(db).limit(cpu)` queued first and blocked
on `db` (held by a resident service); `B` `.limit(cpu)` queued after; `cpu`
has capacity. `admit.rs:84-85` blocks `B` behind `A` on `cpu`; `why("B")` is
`[]`. The testkit driver records `why` after every step (`driver.rs:73-82`)
and the P-16 tests (`tests/machine.rs:145-236`) only ever assert a *present*
reason, never that a waiting node has *at least one*.

**Smallest fix.** Add `Reason::QueuedBehind` and, in `admit`'s grant loop, record
on the slot which earlier waiter blocked it (`slot.blocked_by: Option<usize>`);
`waits_on` emits `(path_of(blocked_by), QueuedBehind)`. Then a checker rule
"a `Waiting` node has a non-empty `on`" becomes writable (§ 5, V-08).

### F-07 — major — `OD-PANIC-CANCELLED` holds only on the signalled path

**Evidence.** The decision (`SdaxContract-v1.md:405`) says: "a body the engine
had already cancelled panics … the node's terminal state is `Interrupted` …
neither is a fault in the report." In the machine the escape is keyed on
`signalled`, not on `cancelling`: `on_err` `faults.rs:123-131` (`St::Running if
self.slots[n].signalled` → trace `Fail(_, Panic)` + `finish_interrupted`), and
`on_ok` `:77`. For a **drop-mode** cancel — the default for resources, steps
and effects (contract `:197`) — `interrupt` sets `cancelling = true` and emits
`Abort` but leaves `signalled` false (`settle.rs:147-152`). A panic that then
arrives, whether as `NodeErr(_, Panic)` or as `TaskJoined { Panicked }`
(`machine.rs:124-126` maps it to `on_err`), takes the `St::Running =>
fail_attempt` arm (`faults.rs:132-135`): a `Fault` is parked, `can_retry` is
false because the scope no longer admits (`:198-199`), `node_failed` flushes it
to the report and the node is `Failed` (`:266-268`). That is the case the
decision describes and the opposite outcome.

The simulator cannot produce it: after `Abort` it delivers only outcomes
already due at `now` (`simulator.rs:279-287`), and a tokio task that panics in
`Drop` while being aborted reports `JoinError::is_panic()` — Stage 2 will hand
the machine exactly `TaskJoined { Panicked }` for an aborted body. The machine
cannot distinguish "panicked before the abort landed" (the body's own outcome,
which `simulator.rs:4-7` says stands) from "panicked on the way out", and the
decision text covers the second without saying how the driver tells them apart.

**Failing case** (machine-level, no simulator): `p17_plan()`
(`tests/machine.rs:54`), `begin`, `NodeErr(B, Error)` → `Abort(A)`, `Abort(C)`
(row 1 of the TDD log); then `step(TaskJoined { node: A, joined: Panicked })`.
Decision: `A` `Interrupted`, no fault for `A`. Machine: `Fail(Prepare, Panic)`
for `A` in `report.faults`, `A` `Failed`.

**Smallest fix.** In `on_joined` (`machine.rs:121-129`), treat `Panicked` on a
slot with `cancelling && !signalled` the way `on_err` treats the signalled
case: emit `Fail(phase, Panic)` to the trace and call `finish_interrupted`.
Leave `NodeErr(_, Panic)` (the body's own return, delivered because it was
already due) as a fault. Write the split into the decision's reason column and
into `Stage1Report.md` § 10's `TaskJoined` row (`:512`), which today says
"`Panicked` is an `Err`" without qualification.

### F-08 — minor — the `ambiguous` record survives a successful `Ambiguity::Retry`

**Evidence.** In the timeout branch of `finish_interrupted`
(`faults.rs:338-358`) the machine pushes the node onto `self.ambiguous`
(`:343-344`) *before* deciding whether to retry (`:351-358`). A retry that
reaches `Ready` clears the parked faults (`faults.rs:114`) — the `Timeout` is
absorbed, as INV-9's checker escape allows (`trace.rs:545-547`) — but nothing
ever removes the ambiguity record, and `is_clean()` (`report.rs:289-295`)
requires `ambiguous` empty. So an idempotent effect declared
`on_ambiguous(Retry)` whose first attempt times out and whose second succeeds
ends `Outcome::Ok`, `is_clean() == false`, `into_result() == Err`.

**Failing case.** `i15(Mode::Finite, Some(2s), Ambiguity::Retry)` with
`.idempotent()` (`corpus.rs:188-201`), script `Registration` attempts
`[pending, ok@+0]`. Expected by an author who chose `Retry`: a clean run
(the effect happened once, idempotently). Machine: `ambiguous =
{Registration}`, unclean. C-61 (`retry.rs:172-…`) asserts the record stays for
`Compensate`, which is B's text; nothing in B or the contract says it stays
for `Retry`.

**Smallest fix.** Either push the ambiguity record at the same point faults
are flushed (park it on the slot, clear it with the faults at `:114`), or
state in INV-11 that a resolved ambiguity is still reported and `Retry`
therefore never yields a clean run — and then say why `Retry` exists.

### F-09 — minor — `OD-PERSIST-AMBIG` belongs in the validator

**Evidence.** `compensates_ambiguity` (`state.rs:382-387`) and the checker
(`trace.rs:114-117`) both special-case `persistent && Compensate`; the decision
(`:406`) records "persistent wins". But `V-IDEMPOTENT-REQUIRED`
(`budgets.rs:29-34`) still demands `.idempotent()` for `on_ambiguous(Compensate)`,
so the accepted pair obliges the author to assert idempotency of a compensation
that can never run. Two meaningless declarations are accepted where the
crate's own rule set refuses a pool nobody uses (`V-UNUSED-POOL`) and a
`try_step` nobody reads (`V-TRY-UNCONSUMED`).

**Smallest fix.** `V-PERSIST-AMBIG`: `release == Persistent ∧ on_ambiguous ==
Compensate` is a finding ("a persistent effect has nothing to compensate; use
`Report` or `Retry`"). Ten lines in `validate/budgets.rs`, one row in § 7, and
the decision's last sentence becomes true.

### F-10 — minor — body events are not kind-guarded

**Evidence.** `step` (`machine.rs:67-102`) resolves a key with `node()`
(`:104-112`), which checks only run state and existence. `on_held` (`faults.rs:63-70`)
accepts any `St::Running` node — a step, a join (never `Running`, fine), a
service, a component — and sets `held`; `on_err` (`:121-135`) and `on_ok`
(`:72-103`) accept any `St::Running` kind; `on_cancelled` (`:402-412`)
likewise. Only `on_serve_ended` checks the kind (`:421-423`). A component is
`St::Running` while its inner graph comes up (`admit.rs:164,173-179`), so
`NodeErr(component)` reaches `fail_attempt` → pushes a fault onto the
component's slot → `node_failed` → `Failed` **without** settling the inner
scope (`node_failed` never calls `settle_or_skip_inner`); `NodeOk(component)`
makes it `Ready` with the inner scope still `Admitting`; `NodeCancelled`
makes it `Interrupted` with the inner scope live.

The row-13 empty-fault-vector lemma (`Stage1-TDD-Log.md:32`,
`Review-2026-09-06-External.md` last paragraph) is therefore true *of this
crate's callers* — `start` never emits `Spawn` for a component — and false of
the machine's input domain. D1 totality (`machine.rs:64-66`) promises a
`Reject` for "a body that is not in flight"; a component has no body, ever.

**Smallest fix.** In `node()` or per handler: `Kind::Component | Kind::Join`
→ `Err("this kind has no body")` for `Started/Held/NodeOk/NodeErr/NodeCancelled/TaskJoined`;
`Held` additionally rejected unless `kind.can_hold()` (`plan.rs:70-72`). Then
the lemma is by construction and Stage 2's driver cannot break it.

### F-11 — minor — `NodeCancelled` in a cleanup state is swallowed

**Evidence.** `on_cancelled` `faults.rs:409`: `St::Abandoned | St::Stopping |
St::Releasing | St::Compensating => Ok(())`. For `Abandoned` that is right (the
machine issued the `Abort`). For the other three the machine issued no abort —
INV-7 (`:120`) forbids it — so the event can only mean the driver aborted a
cleanup body on its own. It is accepted without a `Reject` and without a trace
event; the node stays `Releasing` until the budget (`abandon_all`,
`cleanup.rs:255-257`), and under `Shutdown::unbounded()` forever. A driver bug
that violates the strongest shielding claim in the contract is thus the one
input the machine declines to flag.

**Smallest fix.** Return `Err("a cleanup body was cancelled; INV-7")` for
`Stopping | Releasing | Compensating`, keeping `Abandoned => Ok(())`.

### F-12 — minor — a skip without a fault leaves no trace

**Evidence.** `TraceKind::Skipped { because: NodePath }` (`report.rs:169-173`)
requires a cause node. `settle` emits it only `if let Some(b) = because`
(`settle.rs:52-55`), and `because` is `None` for `Cancel`, `Shutdown`,
`Finite` and `Terminal` (`:41-45`); `skip_scope` is the same (`:98-101`). A
node skipped because the run was shut down or cancelled therefore has **no
event at all**; the report's `NodeState::Skipped { because: None }`
(`machine.rs:271-273`) is readable only through `Machine::state`, which a
report consumer never sees. The checker's `is_body_end`/INV-15 rules are
indifferent (no `Start`, no orphan), so nothing notices; an observer replaying
a trace cannot tell "never eligible" from "eligible and skipped by the
request".

**Smallest fix.** `Skipped { because: Option<NodePath> }` (or a second
`SkippedBy(Cause)` kind) and emit it unconditionally in both arms.

### F-13 — minor — the extent of a lock is unstated and shorter than an author assumes

**Evidence.** Grants are taken at start (`admit.rs:121-132`) and released at
`Ready` for every kind but a service (`faults.rs:105-108`), at every failed
attempt before the retry decision (`faults.rs:192`), at interrupt
(`faults.rs:349,367`) and at abandon (`cleanup.rs:226`). So `.exclusive(db)` on
a *resource* protects its acquire body only: its release body
(`start_obligation`, `cleanup.rs:152-156`) runs with no lock, and a held
attempt that fails enters `RetryRelease` (`faults.rs:202-211`) *after* its
lock was released at `:192`, so the next exclusive user of `db` can be granted
and started while the release body of attempt k is still running. Contract
`:46` ("arbitration on a resource the node already needs") and T1 say when a
lock is taken; nothing says when it is dropped. I-27's migrators are steps, so
the corpus never asks.

**Case.** `Conn: resource needs db .exclusive(db) .retry(2)`, `Mig: step
needs db .exclusive(db)`; `Conn` attempt 1 `held@+0 fail@+1`, release `ok@+3`;
`Mig` starts at 1 while `Conn`'s release runs 1..4 on the same `db`.

**Smallest fix.** A sentence in § 1/T1: "a lock is held from the start of a
body to its `Ready`, fault or interrupt; release and compensate bodies run
unlocked." If the release should be covered, hold the grant through
`RetryRelease` (move `release_grants` in `fail_attempt` to after the
`RetryRelease` decision, and release it in `resume_retry`).

### F-14 — note — `within` ignores the cancel mode

`on_within_timer` (`faults.rs:314-328`) emits `Abort` for every async kind,
including a `cooperative(g)` node and a service's start body (whose declared
cancel is signal-then-deadline, contract `:197`). A cooperative body therefore
gets its grace on `cancel()`/`shutdown()` (`settle.rs:153-158`) but none at its
own deadline. Nothing in § 6 or T4 says the deadline is a hard abort; an author
who chose `cooperative` so that an in-flight `hold` can land will be surprised
when `within` produces `Ambiguous` instead. Fix: say it in T4, or run the cancel
mode at the deadline (signal, grace timer, then abort — the `Grace` machinery
already exists).

### F-15 — note — a `terminal` service that fails does not end the scope under `Isolate`

`on_serve_ended` (`faults.rs:428-436`) settles with `Cause::Terminal` only for
`fault: None`. With `Some(_)` and no restart, `node_failed` (`:459-461`) applies
the policy: `Isolate` skips dependents and the scope keeps running until a
request. Contract `:87` says "a `terminal` service finishing"; whether a crash
is a finish is not said. Fix: one clause either way.

### F-16 — note — a child plan's `Mode` has no run-time effect

`check_steady` reads `mode` only in the `component: None` arm
(`admit.rs:221-226`); a component's inner scope stays `Steady` whatever its
mode until the parent's cleanup opens it (`OD-MODE`, `:403`, says so for
`Resident`). Contract `:90` "a nested scope has its own [mode]" is true of the
declaration and vacuous at run time; `V-MODE` is the only consumer. Say so.

### F-17 — note — `Outcome::Ok` with an unclean report

`outcome()` (`state.rs:389-396`) is `Cancelled` on `Cause::Cancel`, else `Ok`
iff `faults` is empty. Cleanup failures, abandonments and ambiguities do not
move it, so the trace ends `End(Ok)` while `is_clean()` is false. `report.rs:133-134`
documents `Ok` as "every node that started settled without a fault", which is
exact but not what "ok" reads as in a log line. C-18 and C-53 assert this
shape, so it is B's; the contract's § 8 should say it in one line.

### F-18 — note — the trace is optional, so the panic in `OD-PANIC-CANCELLED` can vanish

The decision's reason (`:405`) is that INV-9 is "satisfied by the trace". The
trace is `Report.trace: Option<Trace>` (`report.rs:269-270`), "if tracing was
on", and the machine's own emit for the signalled panic is trace-only
(`faults.rs:126-129`). With no observer the panic is observed by the engine and
recorded nowhere, which is INV-9's exact prohibition (`:122`). Fix: amend
INV-9 ("… is in the `Report` or, for a panic in a body the engine had already
cancelled, in the trace") or add a `Report.interrupted_panics` list.

### F-19 — note — a dead component branch, and `Started` for `Abandoned`

`check_end` (`cleanup.rs:384-390`) turns a `Ready` component into
`Finished`/`Stopped`, but every path into an inner `Cleanup` goes through
`open_component`, which sets `Releasing` first (`cleanup.rs:190-202`; the
`on_budget_timer` and `try_cleanup` transitions are root-only, `:313`, `:103-108`).
The branch is unreachable, and the checker's component clause accepts
`Stopped` (`trace.rs:140-145,521-526`) for a shape the machine never emits.
Separately `on_started` accepts `St::Abandoned` (`machine.rs:116`) — a `Started`
after abandonment is a late blocking-thread hello and is right to swallow, but
it is undocumented in § 10's event table (`Stage1Report.md:503`).

### F-20 — note — a `Serving` returned after `Signal` is discarded unpolled

A service start body signalled at settle (`settle.rs:127-146`) that still
returns `Ok(Serving)` is `Interrupted{held:false}` (`faults.rs:77` →
`finish_interrupted`), and `owes` (`cleanup.rs:53-56`) owes nothing for an
`Interrupted` service, so the serve future is never spawned and never stopped.
Consistent with C-60's "the return is the answer to the cancel", and with § 5's
rule that a service's acquisitions belong in resources — but the contract's
service row (`:35`) says the obligation exists iff "a serve future exists",
and here one does. Say "…and the start body returned before the stop request".

### F-21 — note — nothing bounds a retry release before shutdown

A held attempt's release between attempts (`RetryRelease`,
`faults.rs:202-211`) has no `within` (release bodies have none, § 6 `:191`) and
no budget until the scope settles (`settle.rs:72` leaves it alone, as INV-7
requires). A hung one holds a `Finite` run at `Admitting` for ever with no
request in sight; only `shutdown()` and then the budget end it. Honest under
INV-7; unstated in § 6, which says the shutdown budget "still bounds cleanup"
without noting that a mid-run cleanup has no budget at all.

### F-22 — note — a `try_step`'s timeout or panic is a fault

The simulator turns a scripted `Fail` into `NodeOk` for a `TryStep`
(`simulator.rs:262-264`, OD-5) but a `Panic` into `NodeErr` (`:268-270`), and
`on_within_timer` → `finish_interrupted` → `fail_attempt(Run, Timeout)` applies
to a try-step like any other (`faults.rs:359-362`). B's X10 row says "try_step
panics are faults, not values"; the contract's § 1 row (`:33`) says only "the
body returns (`Ok` or `Err`)". Add "a panic or a `within` timeout is a fault".

### F-23 — note — post-budget releases are started and abandoned in one instant

After the budget the root goes to `Cleanup` (`cleanup.rs:313-315`),
`advance_cleanup` starts every release whose gate is open (`:132-143`), and the
zero timer (`:319-342`) abandons whatever did not complete at `now`. C-56
(`cleanup.rs` conformance `:148-174`) asks for exactly this, so it is B's
design, not a defect; but T7 (`:105`) says "everything still running is
aborted", and these were not running — they are *started in order to be
aborted*, which a driver will see as `Release(k)` immediately followed by
`Abort(k)`. Worth one sentence in T7 so the Stage 2 driver does not treat it
as a machine bug.

## 3. The thirteen probe families against the implementation

Legend: **(a)** enforced by construction, **(b)** enforced by a check I can
name, **(c)** merely asserted (a driver convention or a comment), **(d)** false.

| probe | verdict | where the decision is made, and what I found |
|---|---|---|
| P1 partial-acquisition leak | **(a)** for the seam, **(b)** for the machine | `Hold::poll` registers in the completing poll (`cx.rs`, `Hold` impl); `on_held` flips `held` (`faults.rs:63-70`); `owes` releases a held `Failed`/`Interrupted` resource (`cleanup.rs:53-56`); `fail_attempt` releases a held attempt before retrying (`faults.rs:202-211`). Checked by INV-3/INV-4 in `trace.rs:472-502,291-313`. Residue is the stated one (`hold_value` after an await); the machine cannot see it. |
| P2 cleanup prerequisite released early | A: **(a)** — a handle is reachable only through `needs`, and `table.rs:123-130` derives `dependents` from `needs` with imports resolved (`:175-178,184-188`), so INV-5's gate (`cleanup.rs:64-83`) covers cross-scope edges. B (stop-by-input-close): **(c)** — nothing in the surface expresses "release X before stopping Y"; a `Writer needs Outbound` that only stops when the sender drops waits until its `stop_within`/budget and is `Abandoned` (`cleanup.rs:212-218`); honest, recorded, and not written anywhere in the contract. C (abandoned holder): **(b)** — `abandon` flips the node and `blocks_release` no longer counts it (`cleanup.rs:64-76`), so `Db`'s release proceeds while the abandoned task may hold the `Arc`; B's C-18 says so and the contract's T7 does not. D: Stage 3. |
| P3 cancellation interrupting cleanup | **(b)** at the machine: `on_cancel` in `Settling`/`Cleanup` only emits `RequestDuringCleanup` (`settle.rs:207-210`); `settle` leaves `RetryRelease` alone (`:72`); checked by INV-7 (`trace.rs:315-329`) — but only for the *trace shape* (an `Interrupted` after a cleanup start). The drop-of-`Running` half is Stage 2 (**c**). And F-11: a driver that does abort a cleanup is not refused. |
| P4 ambiguous retry | **(b)**: `attempts_allowed` (`faults.rs:175-183`) and the `retry` gate at `:351-353` retry after an ambiguous timeout only under `Ambiguity::Retry`; `V-IDEMPOTENT-REQUIRED` (`budgets.rs:16-48`) refuses `retry` without `idempotent`; checked by the INV-11 rules at `trace.rs:350-373`. Note F-08: the ambiguity record then never clears. |
| P5 scope/lifetime escape | **(c)**: `clippy.toml` refuses raw spawn; the machine cannot see a raw task. INV-15 as checked (`trace.rs:463-471,503-508`) is about the machine's own spawns. F-05: the machine's *own* zombie thread on a blocking timeout is the escape the machine makes itself, unlisted. |
| P6 error loss | **(b)**: every terminal ramp flushes parked faults through `flush_faults` (`exits.rs:27-30`; call sites in § 4.1); two same-tick faults are two faults (`fail_attempt` parks, `node_failed` flushes each); cleanup failures are a separate list (`faults.rs:136-149`); `report.sort()` orders them (`cleanup.rs:410`). Checked by INV-9/INV-20 (`trace.rs:539-617,656-699`). Residue: F-18 (a signalled panic exists only in an optional trace), F-12 (a request-skip has no record at all). |
| P7 conflicting providers | **(a)**: providers are keys; two nodes of one type are two keys; nothing to arbitrate. `V-FOREIGN-KEY` for the smuggled key. Nothing to attack. |
| P8 budget deadlock / starvation | Pools: **(b)** `V-POOL-STARVE` (`budgets.rs:71-130`) for resident holders; grants atomic (`admit.rs:79-81`). Locks: **(d)** for starvation — a resident service holding `.exclusive(k)` starves every later exclusive user for the run's life and `build` accepts it; no lock analogue of `V-POOL-STARVE` exists. FIFO cross-grant: **(d)** — F-01 orders unrelated pools; F-06 hides it. The run does not hang (a request ends it) but INV-1 is violated silently. |
| P9 readiness by spawn | **(a)**/**(b)**: `Ready` only from `on_ok` → `ready` (`faults.rs:78,105-116`); `Serving` has private fields; INV-2 checked (`trace.rs:272-290`). The `NeverReady` fault is the driver's (`Stage1Report.md:508-512`), **(c)** until Stage 2. |
| P10 panic policy | **(b)** for the body's own panic (`on_err` → fault; `Releasing` → cleanup failure, `faults.rs:136-149`; never re-raised); **(d)** for a panic after a drop-mode abort (F-07): the contract's decision says `Interrupted`, the machine says `Failed`. |
| P11 executor stop / deadline overrun | Deadline: **(b)** — `on_budget_timer` → `abandon_all` → `incomplete` (`cleanup.rs:298-317,220-235`); checked by INV-8 (`trace.rs:435-446`) and the `Abandoned ⇒ incomplete` rule (`:583-590`). Nested budgets: **(d)** (F-02). Executor stop / `RuntimeDroppedWithLiveRuns`: Stage 2, **(c)**. |
| P12 dynamic-declaration validity | Stage 3; the machine refuses templates at `Table::build` (`table.rs:94-98`) and instance events at `machine.rs:90-92`. **(a)** for "no stub". |
| P13 drop of the caller | Stage 2 (`CancelRequested` from a drop guard). The machine half is P3's: **(b)**. The drainer that keeps driving after the drop is asserted only (`Stage1Report.md:469-…`), **(c)**. |

Two probe-adjacent items the families do not name:

- **Executor stop for blocking bodies.** A blocking body that never returns is
  `Abandoned` and the run ends (`cleanup.rs:221-223`); the thread leaks, as
  LG-6 says. Under `Shutdown::unbounded()` the run instead waits for it for
  ever (`in_flight`, `cleanup.rs:20-37`, counts a `Running` blocking step; the
  generator avoids the case, `mc/script.rs:38-43`). Stated in the proposal,
  not in the contract.
- **Stop-by-input-close and the one-flag FIFO.** P2-B's natural declaration
  (`Writer needs Outbound`, both on a pool) is exactly the shape F-01 delays.

## 4. The five specific risks

### 4.1 Terminal ramps — is the row-13 collapse complete?

Every assignment of a terminal or release state in `host/engine` at `c292c86`
(`git grep -n 'st = St::' c292c86 -- crates/sdax/src/host/engine`), and
whether it reaches the three helpers:

| ramp | site | `flush_faults` | `end_component_attempt` | `settle_or_skip_inner` |
|---|---|---|---|---|
| Isolate skip | `admit.rs:246` (`skip_dependents`) | `:253` ✓ | n/a (never `Running`) | `:254` ✓ |
| settle: Pending/Waiting → Skipped | `settle.rs:49` | `:59` ✓ | n/a | `:80` ✓ (tail, every node) |
| settle: Backoff → Interrupted | `settle.rs:66` | `:69` ✓ | n/a | `:80` ✓ |
| settle: Running → interrupt | `settle.rs:71` → `interrupt` | via the result event | component arm `:125` ✓ | `:119` ✓ and `:80` |
| skip_scope → Skipped | `settle.rs:95` | **no** — sound: the scope is `Planned` (`:89`), no node ever started, so `faults` is empty | n/a | recursive `:103-105` ✓ |
| retry release ends, scope not admitting → Interrupted | `faults.rs:227` | `:230` ✓ | n/a | not called — sound: a resource/effect has no inner |
| attempts exhausted → Failed | `faults.rs:267` | `:268` ✓ | n/a | **not called** — a component never reaches `node_failed` unless F-10's driver misuse occurs |
| inner fault → component Failed | `faults.rs:286` | **no** — sound by the lemma in 4.3 | n/a (component is `Running` or `Ready`, and the fault *is* the terminal observation) | `:299` ✓ |
| timeout, effect not held → Ambiguous | `faults.rs:345` | via `ambiguous_failed` `:390` ✓, or kept for the retry (`:355`) ✓ | n/a | `ambiguous_failed` does not call it — sound (effects have no inner) |
| interrupted → Ambiguous / Interrupted | `faults.rs:369,374` | `:377` ✓ | n/a | not called — sound (F-10 aside) |
| budget → Abandoned | `cleanup.rs:228` | `:232` ✓ | components are never `abandon`ed (`:250-251,262-264`) | `abandon_inner` `:282` ✓ |
| open a component's release | `cleanup.rs:200` | via `end_component_attempt` `:199` ✓ | `:199` ✓ | `:203` ✓ |
| settling parent, inner quiesced | `cleanup.rs:96` | via helper ✓ | ✓ | n/a (inner is `Settling`) |
| inner scope ends → component Released/Finished | `cleanup.rs:385,388` | n/a (already terminal) | n/a | n/a |
| release/compensate/stop results | `faults.rs:80,85,137,144,471,475` | n/a (a node in a cleanup state already flushed or absorbed) | n/a | n/a |

**Verdict: complete for the three rules**, with two sites that are correct by
argument rather than by the helper (`settle.rs:95` and `faults.rs:286`) and
one that is correct only while F-10's guard is absent (`faults.rs:267` for a
component). All three would cost one call each to make the helper the proof.

One ramp *class* the collapse does not cover, because it is not one of the
three rules: **a state change that opens a release without a `Start`-side
terminal observation for a non-component** does not exist (every non-component
release opens from `Ready | Failed | Interrupted | Ambiguous`, `cleanup.rs:46-59`).
And one rule the collapse *should* have had a fourth helper for and does not:
**`release_grants`** is called on six ramps (`faults.rs:107,192,349,367,426`,
`cleanup.rs:226`) and missed on none I can find — but it is the rule F-05
gets wrong by calling it too *early*, so the risk is direction, not omission.

### 4.2 The `held` lemma

`st == St::Running ⟹ started == true`. At `c292c86`:

- `St::Running` is assigned exactly once: `admit.rs:164`, the line after
  `slot.started = true` (`:163`), on the same `&mut Slot`, for every kind.
- `started` is assigned exactly once: `admit.rs:163`. It is read at
  `cleanup.rs:45,271`, `exits.rs:52`, `machine.rs:268`; never cleared, never
  reset by `Slot::new()` after construction (`state.rs:201-220` is construction).
- No other file constructs a `Slot` or writes `st` to `Running` (the grep in
  § 4.1 lists every `st =` write; none is `Running` outside `admit.rs:164`).

The lemma holds. **Stage 2 risk:** the tokio driver has no access to `Slot`
(`pub(super)`, `state.rs:176`), so it cannot break it. The plausible Stage 2/3
break is inside the engine: an instance table that re-initialises a slot for a
template instance (`Slot::new()` per instance) would keep the lemma if it
also goes through `admit::start`; a "restart the component" feature that
re-enters `Running` without `start` would not. Recommend: assert it in
`end_component_attempt` (`debug_assert!(self.slots[c].started)`) so the next
writer trips on it.

### 4.3 The empty-fault-vector lemma

"A component's slot can never hold a fault." Writers of `slots[n].faults` at
`c292c86`: `faults.rs:191` (`fail_attempt`), `:348` (`finish_interrupted`,
guarded `effect && !held`, so `Kind::Effect` only), `:453,460`
(`on_serve_ended`, guarded `Kind::Service` at `:421-423`). `fail_attempt` is
reached from `on_err` (`:134`, any `St::Running` kind), `on_within_timer`
(`:322`, `BlockingStep` only), and `finish_interrupted` (`:361`, needs
`timing_out`, which only `on_within_timer` sets, and a component has no
`within` timer because `start` does not arm one for it, `admit.rs:173-179`).

So the only route is `on_err(component)` — `Event::NodeErr(c, _)` or
`Event::TaskJoined { c, Panicked }` while the component is `Running`. The
lemma is exactly: **the driver never delivers a body outcome for a key it was
never told to spawn.** Stage 2's driver must therefore (i) key its task table
by the `Spawn`/`SpawnBlocking` effects it executed and forward outcomes only
for those keys, (ii) never synthesise `NodeErr` for a `Kind::Component` (e.g.
from a `NeverReady` or `DoubleHold` check it runs per node — those checks must
be per *spawned task*), and (iii) never map an `InstanceEnded` or an inner
report into a `NodeErr` on the component. F-10's guard would make all three
unnecessary.

### 4.4 The two ordering rules — checkable from a trace?

Rule 1 (an outcome already due when `Abort` arrives is delivered;
`Stage1Report.md:517`): **not checkable from the trace.** A driver that drops
an already-due `NodeOk` produces `Interrupted` where `Ready` was due; a driver
that forwards a not-yet-due one produces `Ready` after `Settling`. Both traces
satisfy every rule in `trace.rs` (T5 only forbids a `Start` after `Settling`,
`:214-221`; INV-2 only needs a live attempt, `:272-290`). The machine cannot
tell either (`on_ok`'s comment at `faults.rs:74-76` is the whole mechanism).
It is a convention, **(c)**, and the only place it can be checked is the
driver's own log (abort-issued time vs. outcome-observed time), i.e. suite (d).

Rule 2 (a superseded attempt's outcome is dropped; `:521`): **partly
checkable, by the machine, by accident.** A stale `NodeOk` credited to attempt
k+1 makes that attempt `Ready` before its own `Started`, which the next
`Started` turns into `Effect::Reject` (`machine.rs:114-118`) — that is how bug
21 surfaced. But a stale `NodeErr` on a node with attempts left is credited to
attempt k+1 as a *fault* with no `Reject` (`faults.rs:132-135`), and the
checker sees a well-formed `Fail`. So a driver that forwards stale errors passes
the walk. The fix is the one `Stage1Report.md:459-466` already names: put
`attempt: u32` on `Started/Held/NodeOk/NodeErr/NodeCancelled`, and have the
machine `Reject` a mismatch. That turns rule 2 into **(b)**; rule 1 stays a
convention by nature.

### 4.5 The decisions

- **OD-PANIC-CANCELLED** — right, for the reason given, and only half
  implemented (F-07); and its reason leans on an optional trace (F-18). Add to
  the reason column: "for a drop-mode cancel the driver reports the join, and
  a `TaskJoined{Panicked}` after an `Abort` is the panic on the way out".
- **OD-PERSIST-AMBIG** — "persistent wins" is the right run-time reading and
  the wrong place to stop: it should be `V-PERSIST-AMBIG` (F-09). The decision
  already says so in its last sentence; the open question in
  `Stage1Report.md:454` should be closed as "yes".
- **OD-BACKSTOP** — right as stated, and its cost to the walk is not stated:
  because every generated script ends with a shutdown at 20–25 s
  (`mc/script.rs:116-121`), a run that *should* have ended by itself and did
  not (a `terminal` service whose finish failed to settle the scope; a `Finite`
  plan stuck at `Admitting` on a hung `RetryRelease`, F-21) is ended by the
  backstop and passes. The liveness guard (`driver.rs:15,57-63`) counts steps,
  not time, so it does not see it either. § 5 V-05 has the check.
- **OD-MODE** — right; and F-16 says the run-time half of "a nested scope has
  its own mode" is vacuous, which the decision should say too.
- **OD-2 / OD-5 / OD-IMPORTS** — nothing to attack on the semantics axis. OD-5
  leaves F-22 (a try-step's panic/timeout) unstated.

## 5. Vacuity: what the checker and the walk cannot fail on

### 5.1 The checker (`crates/sdax-testkit/src/invariants/trace.rs`)

What it really tests, per invariant, on the plans the generator produces:

| inv | checked? | what would pass that should not |
|---|---|---|
| INV-1 needs | yes (`:193-213`) | — |
| INV-1 lock/pool clause | **no rule at all** | two `.exclusive(db)` bodies overlapping; three bodies in a pool of two; F-01's cross-pool delay. The "exclusive contention" and "pool wait" corners (`monte_carlo.rs:239-244`) are counted from `d.waited(..)`, which reads `Machine::state_of` after every step (`driver.rs:73-82`) — the machine's own answer, contradicting `monte_carlo.rs:7-8,123-124` ("never from its state"). Vacuous as a check; it only proves the machine *said* it waited. |
| INV-2 | yes (`:272-290`) | — |
| INV-3 / INV-4 | yes (`:472-502`, `:291-313`) | the `held` flag inside `Interrupted{held}` is never compared with the presence of `Held` — a machine emitting `held:false` after a `Held` passes as long as a release still starts. |
| INV-5 | yes, from the view's `needs` (`:374-387`), imports resolved to parent paths (`view.rs:298-309`) | — |
| INV-6 | not checkable from a trace (a "may") | — |
| INV-7 | shape only (`:315-329`) | an `Abandoned` at any time passes: nothing checks that abandonment happens at `Settling.at + budget` (or at `stop_within`), so a machine that abandons early is INV-7-clean. INV-8 (`:435-446`) only bounds the *end*. |
| INV-8 | root only | an inner scope's budget is never checked (there is no inner `Settling` event: `settle.rs:24-26` emits it for the root alone). F-02 is invisible to it by construction. |
| INV-9 | yes, both directions (`:539-617`, `:685-723`) | — (the `Interrupted|Ambiguous` escape at `:553-555` is the OD-PANIC-CANCELLED reading) |
| INV-10 | first clause only (`:599-613`) | "`Cancelled` only for an external cancel" is unchecked: the trace has no `TraceKind` for a request (`report.rs:153-209` has none), so a machine reporting `Cancelled` on a fault passes. |
| INV-11 | yes (`:330-373`) | — |
| INV-12 | numbering and bracketing (`:231-270`) | "backoff ends immediately on cancel" is unchecked (no rule relates an `Interrupted` in `Backoff` to the settle instant); F-05's physical overlap is invisible because the trace says the attempt ended. |
| INV-13, 16, 17, 19 | n/a in Stage 1 | — |
| INV-14 | one case in sixteen (`monte_carlo.rs:38,413`) | fine as a sample; stated. |
| INV-15 | yes (`:463-471,503-508`), for the machine's own spawns | F-05's thread (not an engine "spawn" the trace knows about after the timeout). |
| INV-18, INV-20 | yes (`:293-298,618-623`, `:656-683`) | — |
| T4 Isolate skip | **no rule**; only *counted* (`monte_carlo.rs:222-229`) | a machine that skips nothing on an Isolate fault: under `Resident` the backstop shutdown settles everything with `because = None`, no `Skipped` event is emitted (F-12), the node never `Start`s, INV-15 is silent, and the case passes. Under `Finite` it would hang at `Admitting` → `LIVENESS`, so the rule is checked there only by accident. |
| T5 | root `Settling` only (`:214-221`) | a start inside a component after the *inner* scope settled (inner `FailFast` fault, inner `terminal`) is unchecked; only the machine's own `admit` guard (`admit.rs:19-21`) prevents it. |
| T6/T7 nested budget, T7 "started to be aborted" (F-23) | no | — |
| `terminal` | **no rule** | a `terminal` finish that fails to settle the scope passes (OD-BACKSTOP ends it later). |
| `Skipped{because}` | **no rule** | any `because` passes, including a node that never faulted. |
| `Waiting{on}` / `why` | **never validated** | F-06's empty `on`; a wrong reason. |
| `Reject` | yes — any rejection fails the case (`driver.rs:104-116`, `monte_carlo.rs:391`) | strong; this is what found bugs 6, 20, 21. |

Static side (`invariants.rs:48-148`): INV-5/INV-6 there are self-referential
regression guards, as the file says at `:14-19`; INV-1 has a negative fixture.

### 5.2 The generator (`crates/sdax-testkit/src/mc/{gen,script,keys}.rs`) — what it cannot generate

| corner | why unreachable | what it leaves untested |
|---|---|---|
| nested components (depth ≥ 2) | `child_plan` calls `fill(.., allow_component = false, ..)` (`gen.rs:380-391`) | `component_faulted`'s grandparent recursion (`faults.rs:305-307`), `skip_scope`'s recursion (`settle.rs:103-105`), `abandon_inner` under `abandon_all` of an inner scope (`cleanup.rs:261-278`), `RecordOrder` at depth 3 |
| two locks on one node | `pick_attrs` sets at most one of `exclusive`/`shared` (`gen.rs:97-104`) | T1's all-or-none atomicity across several locks (`admit.rs:70-81`); `V-DUP-ATTR` per key |
| a lock on an imported resource | locks are chosen from needs with `res == true`, and imports are pushed with `false` (`gen.rs:91-96`, `:366-369`) | cross-scope lock arbitration through `table.rs:198-199` (a child node and a parent node contending for one lock; the global FIFO `seq` across scopes) |
| a service holding a pool (valid plan) | `limit` only on `Step|Resource|Effect` (`gen.rs:105-107`); services get one only under `Mutation::PoolStarve`, which must be refused | a resident pool holder at run time; F-01's permanent variant |
| a `cooperative` service or blocking step | `gen.rs:111-117` | F-04 (accepted, ignored) |
| `retry` on a service or try-step | `gen.rs:75-78` | `attempts_allowed` for a service start body; a retried try-step |
| `within` on a join / component | not applicable (no attrs) | — |
| an unbounded child plan | `child_plan` always builds `Shutdown::within(span)` (`gen.rs:371-376`) | inner `deadline = parent's` only (`settle.rs:32-35`, the `(None, b)` arm) |
| more than 3 inner nodes; a child with > 2 imports; a child importing a try/join key | `count = range(1, 3)`, `subset(.., 2)`, `parent_units` only (`gen.rs:366,379`) | small |
| a body that `held` then pends or panics | `body()` pairs `held` only with `ok` or the "after-hold" `fail` (`script.rs:27-50`) | `Interrupted{held:true}` by abort-after-hold (only reachable through the fail-after-hold race today); `Abandoned` of a held, hung prepare |
| a `Held` for a service/step | n/a in the sim | F-10's `on_held` leniency |
| more than four requests; two requests in one instant with a body outcome | `requests = below(3)` (`script.rs:100-115`) | fine |
| the same plan run twice | one run per case | INV-13 (Stage 2) |
| `by_drop` release outcomes | scripted like any release (`script.rs:96-98`): `Fail`/`IgnoreStop` for a drop that cannot fail | harmless over-approximation; but the machine never treats `ReleaseStyle::Drop` specially either (it emits `Effect::Release`), which the contract's `release: drop` rendering may lead a reader to doubt |
| two pools with a waiter on each | **reachable** (`pool_of`, `gen.rs:223-237`) — and F-01 is not found because INV-1's pool clause has no checker rule | the walk *has* been through F-01 and could not see it |

### 5.3 Coverage floors that pass trivially

- `exclusive contention` (floor 5) and `pool wait` (floor 5): counted from
  the machine's `why`, not from anything the checker verifies (§ 5.1). They
  measure reachability of a *state*, not correctness of the arbitration.
- `isolate skip propagation` (floor 5): counts a `Skipped` event under an
  `Isolate` plan (`monte_carlo.rs:222-229`); no rule says the right nodes
  were skipped.
- `cooperative grace completes/expires`: counted from `Signal`/`Abort`
  effects (`:187-202`); a service with a `cooperative` attribute (never
  generated) would be counted under the wrong mechanism.
- `component fault`: counts any `Fail` on a path containing `/`
  (`:204-208`); says nothing about what the component did with it (F-03).

## 6. What I could not determine without running something

Every finding above was reached by reading; none was executed. The following
runs would confirm or refute the ones that turn on a trace. All use the
existing testkit API (`ScriptedDriver::run`, `Driven::eol`, `Machine::step`);
none needs new infrastructure except R-6.

**R-1 — F-01 / F-06 (blocker/major), `crates/sdax-testkit/tests/conformance/regressions.rs`:**

```rust
#[test]
fn review_a_free_pool_is_not_queued_behind_a_full_one() {
    let mut p = Plan::builder("HOL");
    let cpu = p.pool("cpu", 1);
    let io = p.pool("io", 1);
    p.step("A1").limit(cpu).run(|_cx, ()| async move { Ok(()) });
    p.step("A2").limit(cpu).run(|_cx, ()| async move { Ok(()) });
    p.step("B").limit(io).run(|_cx, ()| async move { Ok(()) });
    let plan = p.build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Finite).expect("valid");
    let script = Script::new()
        .prepare("A1", Body::ok(At::plus(10.0)))
        .prepare("A2", Body::ok(At::plus(0.0)))
        .prepare("B", Body::ok(At::plus(0.0)));
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    let t = d.eol();
    assert_eq!(t.start("B"), Some(0.0), "INV-1: io is free, B has no needs");   // predicted: Some(10.0)
    assert!(d.why_at("B", 0.0).is_empty() == false, "F-06: a waiting node names a reason"); // predicted: empty
}
```

Predicted result: both assertions fail — `Start(Run) B` at t=10, and every
recorded `why` for `B` before 10 is `[]`.

**R-2 — F-02 (major), same file:**

```rust
#[test]
fn review_a_nested_budget_does_not_start_before_the_inner_release_can() {
    let mut inner = Plan::builder("Child");
    let conn = res(&mut inner, "Conn");                       // corpus.rs:34 helper
    inner.service("Svc").needs(conn).stop_within(secs(1))
        .start(|_cx, _c: Arc<Unit>| async move { Ok(serving()) });   // corpus.rs:106
    let child = inner.export(conn)
        .build(Policy::FailFast, Shutdown::within(secs(2)), Mode::Resident).expect("valid");
    let mut p = Plan::builder("P");
    let c = p.component("C", &child);
    let sess = res_needs(&mut p, "Sess", c);                  // corpus.rs:40 helper
    p.service("Api").needs(sess).stop_within(secs(1))
        .start(|_cx, _s: Arc<Unit>| async move { Ok(serving()) });
    let plan = p.build(Policy::Isolate, Shutdown::within(secs(30)), Mode::Resident).expect("valid");
    let script = Script::new()
        .serve("C/Svc", [Serve::Err(At::plus(3.0), "dies".into())])
        .cleanup("C/Conn", Cleanup::Ok(secs(1)))
        .at(20.0, Request::Shutdown);
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();                                                // predicted: passes — the checker cannot see it
    let t = d.eol();
    assert_eq!(t.cleanup_start("C/Conn"), Some(20.0));
    assert_eq!(t.cleanup_end("C/Conn"), Some(21.0), "1 s of release inside 30 s of budget");
    assert!(d.report.incomplete.is_empty(), "{}", d.report);  // predicted: incomplete = {C/Conn}, abandoned at 20
}
```

Also run the `FailFast`-parent variant (parent 10 s, child 2 s, `Sess`'s
release `Cleanup::Ok(secs(3))`, no `Api`, no request): predicted
`abandoned("C/Conn") == Some(3.0)` with `End` at 3 and 7 s of budget unused.

**R-3 — F-03 (major):** build the plan in F-03's case, script `C/B`
`fail@+0`, `C/A` `ok@+2`, shutdown at 6; print `d.eol().render()`. Predicted:
`C/A Interrupted`, `C/D Skipped`, `U Skipped`, faults `[C, C/B]`, `U` never
starts. The contract-side question ("what should an `Isolate` child do?") is
the owner's; the run only shows what it does today.

**R-4 — F-07 (major), `crates/sdax/src/tests/machine.rs`, next to `p17_…`:**

```rust
#[test]
fn review_a_panic_after_a_drop_abort_is_not_a_fault() {
    let plan = p17_plan();
    let mut m = Machine::new(&plan).expect("static");
    m.begin();
    for n in ["Base", "A", "B", "C"] { let _ = ok(&mut m, n); }  // whatever p17's set-up is; get A, C Running
    let _ = err(&mut m, "B");                                 // → Abort(A), Abort(C)
    let a = key(&m, "A");
    let fx = m.step(Event::TaskJoined { node: a, joined: JoinedLabel::Panicked });
    // OD-PANIC-CANCELLED: A is Interrupted, the panic is in the trace, no fault.
    assert!(matches!(m.state_of("A"), Some(NodeState::Interrupted { .. })), "{:?}", m.state_of("A"));
    // predicted: Failed { .. }, and after End: report.faults contains A (Prepare, Panic)
}
```

**R-5 — F-05 (blocker, Stage 2):** nothing in Stage 1 can show a thread.
The proof is `R-04` from `CanonicalTests.md` § 5 on the tokio adapter with the
`regressions.rs:547-576` shape plus a sibling `B2` on `cpu(1)`: an atomic
high-water mark inside the bodies will read 2. Until then, the machine-level
witness is `t.start_attempt("B", 2) == Some(2.0)` while attempt 1's scripted
outcome is at 4 — which the existing regression already asserts as *correct*.

**R-6 — the checker rules that would have found F-01/F-06 (and would guard
INV-1's pool clause), then the walk:**

1. In `driver.rs:75-82`, after recording a `Waiting { on }`, push
   `Violation { rule: "WHY", detail }` if `on.is_empty()`. Run
   `cargo test -p sdax-testkit --test monte_carlo` (default 3 000). Predicted:
   fails on the first case with two pools and a full one, which the generator
   reaches (`gen.rs:223-237`).
2. In `trace.rs`, add "MUTEX": for every pair of nodes that both name
   `attr("exclusive") == Some(r)` (or one exclusive, one shared), their
   `[Start, body-end]` intervals (per attempt) do not overlap; and "POOL": at
   every trace index, the number of nodes with `attr("pool"|"limit") == p`
   whose latest attempt has a `Start` and no body end is `≤ limit`. Run the
   200 000-case walk (`SDAX_MC_SEED=20260906 SDAX_MC_CASES=200000 … monte_carlo_big -- --ignored`).
   Predicted: clean (F-01 is a delay, not an overlap) — but now the corner
   floors mean something.
3. Add "TERMINAL": a `Stopped` on a `terminal` service with no prior
   `StopRequested` is followed, at the same `at`, by root `Settling`. Predicted
   clean; it closes the OD-BACKSTOP blind spot.

**R-7 — F-08 (minor):** `i15(Mode::Finite, Some(secs(2)), Ambiguity::Retry)`
with `.idempotent()` added to the corpus helper; script
`.body("Registration", vec![Body::pending(), Body::ok(At::plus(0.0))])`.
Predicted: `report.ambiguous == [Registration]`, `outcome Ok`,
`is_clean() == false`.

**R-8 — F-11 (minor), machine-level:** `i16`, shutdown, then
`step(NodeCancelled { node: PeerStore, held: true })` while `PeerStore` is
`Releasing`. Predicted: `Ok(())`, no `Reject`, node still `Releasing`.

## 7. The unstated — what an author has to know that the contract does not say

Each of these is derivable from the code and from nothing an author reads.

1. A lock covers the prepare/run body only; release and compensate bodies run
   unlocked, and a retried resource's between-attempt release runs while the
   next exclusive user may already hold the lock (F-13).
2. A waiter that names a pool is queued behind every earlier waiter that
   names *any* pool (F-01); and `why` will not tell you (F-06).
3. A blocking body is never told to stop; `cx.is_stopping()` there is always
   `false`; `.cooperative(g)` on it does nothing (F-04). Its `within` is "fault
   now, thread keeps running, retry may start beside it, pool slot freed" (F-05).
4. A component fails at the first inner fault whatever the child's policy;
   `Isolate` on a child plan governs only what is skipped on the way down
   (F-03). A child's `Mode` governs nothing at run time (F-16).
5. A child plan's shutdown budget starts when the child settles, which can be
   long before its release graph is allowed to run; declare it as long as the
   parent's if the parent has resources depending on the component (F-02).
6. `Ambiguity::Retry` never yields a clean run once a timeout has happened
   (F-08); `Ambiguity::Compensate` on a `persistent()` effect is accepted and
   compensates nothing, and still requires `.idempotent()` (F-09).
7. `within` is a hard abort regardless of cancel mode (F-14). A `try_step`'s
   timeout or panic is a fault (F-22). A `terminal` service that dies under
   `Isolate` ends nothing (F-15).
8. `Outcome::Ok` does not mean clean: read `is_clean()`/`into_result()`
   (F-17). A node skipped by a shutdown has no trace event (F-12). A panic in
   a body the engine had already signalled is in the trace only, and there is
   no trace unless an observer was attached (F-18); after a drop-mode abort it
   is a fault (F-07).
9. After the budget, gated releases are emitted and aborted in the same
   instant; a driver will see `Release(k)` then `Abort(k)` (F-23). A hung
   mid-run retry release has no deadline before `shutdown()` (F-21).
10. A `Serving` returned after the stop request is dropped unpolled; put the
    listener in a resource (F-20 — § 5 says so for acquisitions, not for this).
11. For the Stage 2 driver specifically: never deliver a body event for a
    component or a join (§ 4.3); `Started` must precede every other event of
    its attempt or the machine `Reject`s; a stale attempt's outcome must be
    dropped by *you* (§ 4.4); a `NodeCancelled` for a release will be swallowed,
    not refused (F-11).

*End of review. No files other than this one were written; no commands other
than `git show`, `git ls-tree`, `git grep`, `grep`, `sed`, `wc`, `ls` and
`find` were run.*
