# Stage 3 report — templates and dynamic instances

Written 2026-09-06, on top of `cacf849` (Stages 0–2 and the Stage 1 semantics
review). Normative document: `dev-docs/SdaxContract-v1.md`. Working rules:
`AGENTS.md`. Step-by-step record: `dev-docs/Stage3-TDD-Log.md`.

---

## 1. What Stage 3 delivers

`Machine::new` refused any plan that declared a template and named the stage.
It no longer does: templates and their instances run, on the pure machine and
on the tokio adapter, from the same sources.

- **The machine.** A template node is a node like any other until its imports
  are ready, and then it is **`Live`**: it has no body and never becomes
  `Ready` (contract § 1), and its obligation is stopping the instances it
  admitted. `Table::instantiate` appends one instance's scope and nodes to the
  flat table at run time, with keys minted for that instance so every node of a
  run stays uniquely addressable. An instance is admitted, settles, cleans up
  and ends as one unit; `Effect::SpawnInstance` and the `InstanceSpawned` /
  `InstanceEnded` observations are live.
- **The seam and the run.** `cx.spawn(&template, input)` works from any body,
  and `Child::{ready, stop, id}` drive one instance. The **refusal is the
  machine's**, published after every step as a `SpawnTable` snapshot, so a body
  in its own task answers exactly what `Machine::spawn_check` would.
- **Both drivers.** The stepping simulator performs scripted `SpawnSpec`
  directives through the same `spawn_check`; the tokio run driver performs
  `SpawnInstance`, attaches the run's `Scope` to every body context, and drops
  an instance's slot tables when the trace says it ended. Suite (c) is one
  source compiled twice, as Stage 2 established.
- **The checker.** Every trace rule now groups by the *copy* of a declaration —
  path plus instance chain — instead of by path, so an instance is checked as
  thoroughly as the static graph and never merged into it. Three rules are new:
  `INSTANCE`, `T5-INSTANCE` and `INSTANCE-RELEASE`.
- **The walk.** The Monte Carlo generator declares templates, imports parent
  keys into them, nests them, and scripts spawns at random ticks — awaited,
  closed, refused, or arriving too late. Fourteen new coverage floors.

---

## 2. The gates, verbatim

```
$ cargo fmt --check
(no output)

$ cargo test --workspace --locked --offline
test result: ok. 84 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 74 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 3 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 78 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
  → 302 passed, 0 failed, 2 ignored

$ cargo clippy --workspace --all-targets --locked --offline -- -D warnings
(no output)

$ cargo package -p sdax --locked --offline --allow-dirty
   Packaging sdax v0.1.0
    Packaged 55 files, 503.7KiB (128.7KiB compressed)
   Verifying sdax v0.1.0
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.53s

$ ./scripts/check-architecture.sh
  sdax           role: pure/contract
  sdax-testkit   role: harness
  sdax-tokio     role: implementation
ARCHITECTURE GATE PASSED

$ ./scripts/compile-fail.sh
== 15 witnesses          (all PASS, each with the error code it claims)

$ RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --offline
(no output)
```

The two ignored tests are `monte_carlo_big` and the Stage 0 `#[ignore]`d
witness, both run explicitly below.

---

## 3. Measured

**Test count.** 280 → **302** (2 ignored). Suite (c) is **74 rows** on the
scripted driver and **78** on the adapter — the same 74 rows plus the four
adapter-only tests in `differential.rs` and `multi_thread.rs`.

**The fast loop**, Apple M3 Pro, rustc 1.96.0, `cargo test --workspace
--locked --offline`:

| | at `cacf849` | now |
|---|---|---|
| sum of the runners' own "finished in" | ~3.0 s | **~3.8 s** |
| warm wall clock, nothing to rebuild | 3.9 s | **3.9 s** |
| wall clock after touching `host/engine/machine.rs` | — | **17.4 s** |

The 0.8 s of execution growth is the Monte Carlo fast loop (0.69 s → 1.17 s
for its 3 000 cases, which now build templates) and the doctests. **This does
not match the budget `AGENTS.md` records** (~0.55 s of execution, ~3.8 s with a
rebuild); the discrepancy is not Stage 3's — the same command measured ~3.0 s
of execution at `cacf849` on this machine at the start of the work. Left as an
open question rather than silently rewriting the budget.

**Monte Carlo.**

| walk | cases | result |
|---|---|---|
| `monte_carlo` (fixed seed, CI) | 3 000 | clean, every floor cleared |
| `monte_carlo_big` | 50 000 × 4 runs, distinct seeds | clean |
| `monte_carlo_big` | 200 000 | clean, 23.3 s (release) |
| `monte_carlo_on_the_adapter` | 20 000 | clean, 2.9 s (release) |

**Lines.** 22 617 → **26 061** lines of Rust (+3 444). Eleven new files (2 998
lines): `host/engine/instances.rs` (444), `sim/instances.rs` (185),
`sim/effects.rs` (248, split out of `simulator.rs`), `tests/instances.rs`
(339), `invariants/{occ,report,containment}.rs` (111 / 379 / 267),
`mc/templates.rs` (160), `conformance/instances.rs` (280),
`sdax-tokio/src/scope.rs` (201), and `tests/monte_carlo/corners.rs` (384,
split out of `monte_carlo.rs`, which went 682 → 419). No file this stage wrote
is over 500 lines; `mc/gen.rs` (556) and `host/engine/faults.rs` (580) were
already over before it.

---

## 4. The rows

| row | what it asserts | where |
|---|---|---|
| **`C-30`** | per-connection links: two instances, one closed at t=3 while the run continues, the other stopped by the shutdown; the imported `Endpoint` releases only after **both** have finished | `conformance/instances.rs::c30_*`, both drivers |
| **`C-51`** | a foreign template handed to `cx.spawn` → `SpawnError::ForeignTemplate` at the first run, and nothing is created | `c51_*`, both drivers |
| **`C-64`** | an instance whose `terminal` service finishes ends **that instance** `Ok`, before the run's own shutdown and with no fault | `c64_*`, both drivers |
| **`C-65`** | `cx.spawn` while the scope is settling → `SpawnError::ScopeStopping` | `c65_*`, both drivers |
| **`C-14`** (INV-16 half) | a `cancel()` with a live instance drains it — `InstanceEnded(#1, Cancelled)` — before the key it imports is released | `c14_*`, both drivers; and `tests/instances.rs::a_live_instance_holds_the_key_it_imports_until_it_ends` on the machine directly |
| **`I-34`** | a node that depends on **every** instance being ready: the acceptor's start body spawns two links and awaits each `Child::ready()` before returning `Serving` (INV-17), so `Router`, which `needs` the acceptor, starts only after both instances are ready | `i34_*`, both drivers |

`I-34` is the coverage failure the design comparison held out. It is
expressible because `spawns` is a declaration the validator checks
(`V-SPAWN-KIND`, `V-SPAWN-SELF-IMPORT`) and `Child::ready()` is a real wait:
the test measures the instances' `Ready` against the dependent's `Start`, not
the machine's opinion of them.

Also new on the machine: `a_spawn_opens_an_instance_and_admits_its_nodes`, the
three `spawn_check` refusals, `instance_records_carry_the_instance_in_their_order`
(F4: instances sort by id under the template's own step),
`a_body_event_for_a_template_is_refused` (row 13's lemma extended: a template
has no body either), and `a_spawn_that_lands_after_the_settle_ends_at_once`.

---

## 5. Bugs found, with seeds

Five defects, all found by the walk and all pinned.

| # | seed | symptom | cause and fix |
|---|---|---|---|
| 1 | `10566988825915931949` (pure, case 127) | `INV-16: instance #1 of Tpl was still live at End` | `owes` read a template's obligation off `St::Live` alone. An isolated fault had already `Skipped` the template, so it stopped owing and stopped blocking, and the root ended with the instance running. A template now owes while it is `Live` **and** whenever it ever spawned, whatever state the node reached; `keep_template_live` keeps one that admitted an instance out of `Skipped` in the first place |
| 2 | `1732908402085695841` (pure, case 385) | `INV-16: template C3/C3/Tpl spawned instances and its obligation never ended` | the same rule from the other side: a template whose instances had already ended never closed its own record. It now always ends `StopRequested` → `Stopped`, so a reader can tell "done with its instances" from "still holding them" |
| 3 | `12339652235566683353` (`monte_carlo_big`) | a template with **both** an `Abandoned` and a later `Stopped` | `owes` did not exclude `St::Abandoned`, so `advance_cleanup` opened the obligation a second time after the budget had abandoned it |
| 4 | `1520897640266743839` (`monte_carlo_big`) | `INV-16: instance still live at End` with `INV-15: its release never ended` | `blocks_release` stopped counting a template's live instances once the template itself was abandoned, so the root ended while an instance was mid-cleanup. It now blocks while any instance is live; the instance scope's own zero timer (T7b) is what still terminates the run |
| 5 | `SDAX_MC_SEED=6746427589533237250`, index 281, **adapter only** | `stuck: the run stopped short of End` | the race the design anticipated and mishandled: `cx.spawn` passed the gate and the scope settled before the event reached the machine. The instance's scope was still `Planned`, which `settle` leaves alone, so it stayed `Planned` for ever and held the run open. `on_instance_spawned` now *skips* and ends it at once, so the `Child` answers. The pure driver cannot reach this — its check and its event are one step — which is exactly why the adapter walk exists. Pinned deterministically by `a_spawn_that_lands_after_the_settle_ends_at_once` |

Two more findings were in the **checker**, not the engine, and are worth the
same weight because a wrong rule is a rule that cannot catch anything:

- `MUTEX` rebuilt a `NodePath` by splitting the rendered string on `/`. A
  generated node's *name* contains one, so two instances' own copies of a lock
  looked like one shared lock and the rule fired on a correctly arbitrated run
  (seed `12339652235566683353`). `path_of` now resolves against the view.
- The INV-5 dependent set counted the **template node**, which has no lifetime
  of its own, and so reported a retried effect's *between-attempts* release
  (INV-12) as a breach — the attempt it undoes never reached `Ready`, so no
  dependent could have consumed it. The template is out of that set; the real
  clause is `INSTANCE-RELEASE`, asked of the release that discharges a key's
  obligation and waived after the budget (T7b).

**A non-bug, recorded rather than papered over.** Seed `6312987428304776340`
hung. A service with **no `stop_within` inside any scope** of an unbounded run
has nothing to end its stop: T7a bounds an inner scope by the root's deadline
until its release graph opens, and the root's is `None`. `V-SERVICE-UNBOUNDED`
refuses this for the root's own services and does not reach a child's. That is
a pre-existing gap between the validator and the run-time semantics — a
component can express it too — that the walk reached only because templates
multiplied inner scopes. The engine is behaving as INV-8 allows. It is open
question 1 below; the generator now steers around it (`Shape::bounded_root`)
rather than the rule being weakened.

---

## 6. Decisions

Recorded in the contract's § 13 table, dated 2026-09-06.

| id | decision |
|---|---|
| **OD-INSTANCE-EVENTS** | the machine owns an instance's lifecycle, so "the instance ended" is its own observation and never an input. `Event::InstanceEnded` is replaced by `Event::StopInstance(id)` — what `Child::stop()` sends — and `Event::InstanceSpawned` carries the **spawner**, because a `Template` handle is a declaration key and a template inside a template's plan has one node per instance |
| **OD-SPAWN-INPUT** | `Cx::spawn<I>` requires `I: Send + Sync`. The input lives in the instance's slot table like any other value; `Plan::template::<In>` already required it, so no handle that can be built is excluded |
| **OD-SPAWN-EARLY** | a body may instantiate a template whose own imports are not ready: the instance's nodes wait under T1. `V-SPAWN-SELF-IMPORT` already rules out the one case that could never resolve |
| **OD-INSTANCE-FAULT** | a fault inside an instance does not fail the parent. INV-16 forbids a parent node to name an instance node, so there is no edge for it to travel; the fault is in the run's report in F4 order and the instance ends `Failed`. "Unless the declaration says so" has no spelling today |

Two more choices are worth stating even though they only confirm the contract:

- **A template's node is `Live`, not `Ready`.** § 1 says "becomes Ready: n/a",
  and INV-2 says readiness is a *return*. A template has no body, so it gets a
  public `NodeState::Live` of its own rather than borrowing `Ready` and forcing
  the checker to make an exception.
- **An instance's `Mode` is a declaration only.** Contract § 2 already says a
  child plan's mode is read by `V-MODE` and does not settle its scope at run
  time; an instance follows components exactly, and is ended by `Child::stop()`,
  a `terminal` service inside it, its own `FailFast`, or its template's
  obligation.

---

## 7. What is not claimed

- **`S-02`** (exhaustive schedule enumeration) still did not run. Unchanged
  since Stage 1.
- The **checker's honest limits** from Stage 1 stand: INV-5 and INV-6 in
  `check_plan` are regression guards with positive coverage, not independently
  falsified checks. The new `INSTANCE`, `T5-INSTANCE` and `INSTANCE-RELEASE`
  rules **do** have negative fixtures — each of the five defects above failed
  one of them before it was fixed.
- **MSRV** verification is still partial: the crates declare 1.75 and the
  oldest toolchain here is 1.85.
- Nothing about **nested instances beyond depth 2** is measured. The machine
  has no depth limit — an instance's plan may declare templates and
  `Table::map_of` resolves them against that instance's own map — and the walk
  reaches depth 2 (9 790 cases in 200 000). Deeper is untested.
- The **`Outcome` of an abandoned instance** is `Ok`, matching the root's
  convention (a budget expiry leaves `incomplete` non-empty, and `is_clean()`
  is what says so). It is not evidence the instance finished its work.

---

## 8. Open questions

1. **`V-SERVICE-UNBOUNDED` does not reach a child plan.** Under
   `Shutdown::unbounded()`, any service anywhere in the tree with no
   `stop_within` can hang the settle (T7a: an inner scope is bounded by the
   root's deadline until its release graph opens). The rule is per plan and a
   child plan declares its own bounded budget, so it passes. Options: make the
   rule a whole-tree check at `build` (a component's or template's plan is in
   hand there), or arm a nested scope's own budget at its settle rather than at
   `open_component`. The second changes T7a and needs an owner decision.
2. **Should `SpawnError` distinguish "the template's imports are not ready"?**
   `OD-SPAWN-EARLY` says no and lets the instance wait. If an author ever wants
   the eager answer, it is a fifth variant, not a change of semantics.
3. **Should a template be able to declare a fault propagation policy?**
   `OD-INSTANCE-FAULT` records that an instance's fault stays local; the
   contract's INV-16 wording ("does not by itself fail the parent unless the
   declaration says so") anticipates a `spawns(..).propagate()` that does not
   exist.
4. **The fast-loop budget in `AGENTS.md`** (~0.55 s) does not match what this
   machine measures, at Stage 3 *or* at `cacf849`. Either the budget was
   measured differently or the machine has changed; it should be re-measured
   and rewritten by whoever owns it, not adjusted from here.
5. **`Child::ready()` after the instance is gone** answers `Ok(())`, because the
   latch has been dropped. A body that spawns, drops the `Child`, and asks a
   second handle later cannot distinguish "ready" from "long gone". No test
   depends on it; the alternative is keeping every ended instance's latch for
   the life of the run.
