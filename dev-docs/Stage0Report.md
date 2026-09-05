# Stage 0 report

Date: 2026-09-05. Scope: `sdax-rs` only. Nothing is committed, tagged or pushed;
no network was used.

Stage 0 turns the adopted recommendation of `sdax-v1/reviews/Comparison.md` § 1
into a public API surface, a validator, an inspection API, the seam, the host
contracts and a normative contract document. **It contains no execution
engine** — no `Machine`, no `Plan::start`, no `Running`, and no stub standing in
for one. What is not executed is not claimed executed.

---

## 1. What was built

### `crates/sdax` — the core (std-only, zero normal dependencies, MSRV 1.75)

| file | lines | what |
|---|---|---|
| `src/lib.rs` | 140 | crate docs (with a runnable example), module wiring, the public re-export list, `prelude` |
| `src/key.rs` | 177 | `RawKey`, `Key<T>` (`Copy`, unbranded), `Deps` for `()`/`Key<A>`/tuples to 8, `Slots` |
| `src/policy.rs` | 211 | `Policy`, `Shutdown`, `Mode`, `Ambiguity`, `Retry`, `Backoff`, `Restart`, `CancelMode` |
| `src/plan.rs` | 387 | `Plan<Out, In>`, `PlanIr`, `NodeDecl`, `Attrs`, `Kind`, `ReleaseStyle`, `Pool`, `Template`, `SEMANTICS` |
| `src/builder.rs` | 436 | `PlanBuilder`, `Node<'b, D, K>`, the kind markers and typestates, generic attributes, `release::by_drop` |
| `src/terminals.rs` | 297 | one terminal per kind, `NeedsRelease`, `NeedsCompensate` (`.compensate` \| `.persistent`) |
| `src/shorthand.rs` | 143 | the seven positional constructors (adoption A1) |
| `src/validate.rs` + `src/validate/rules.rs` | 200 + 502 | `Rule` (14), `Finding`, `Invalid`, and one function per rule with its decision procedure |
| `src/view.rs` + `view/{model,render,diff}.rs` | 501 + 155 + 121 + 171 | `PlanView`, `NodePath`, `Edge`, `Why`, `Effects`, `ReleaseOrder`, `Display`, `PlanDiff` |
| `src/cx.rs` + `src/cx/instances.rs` | 434 + 186 | the seam: `Cx<Phase>`, `CxInner`, `Hold`/`Held`, `Serving`, `StopSignal`/`Stop`, `Child`, `Scope`, `SpawnError`, `Timeout` |
| `src/contracts.rs` | 139 | `Time`, `Clock`, `Runtime`, `TaskHandle`, `Joined`, `Observer`, `NoObserver` |
| `src/report.rs` | 348 | `Report`, `Outcome`, `Fault`, `FaultKind`, `FaultLabel`, `Phase`, `RecordOrder`, `Trace`, `TraceEvent`, `TraceKind` |
| `src/engine.rs` | 133 | `engine::{Event, Effect}`, `TimerId`, `JoinedLabel` — the vocabulary, with no machine |
| `src/compile_fail.rs` | 231 | suite (a): 14 `compile_fail` doctests, each naming its error code |
| `src/tests/**` | 2018 | suite (b): the corpus programs and the `P-*` tests |
| `tests/witness.rs` | 300 | suite (a) compile-pass: the crate as an author sees it |

### `crates/sdax-testkit` — the harness (`publish = false`)

`FakeClock` (advance by hand, never by itself), `TraceRecorder` (observation
order, never sorted), and `invariants::{check_plan, check_clock, poll_once}` —
a static checker that recomputes the edge set and the ordering closure itself
rather than asking the core to confirm its own answer. 356 lines of source,
155 of tests.

### `crates/sdax-tokio` — the adapter

`TokioRuntime` (spawn, spawn_blocking, clock, observer), `TokioTask`,
`TokioClock`, `FnObserver`, on `tokio =1.53.1` (`rt`, `time`) and
`tokio-util =0.7.19` (`rt`) — no `macros`, so no proc-macro chain enters a
consumer's cold build and there is no `#[tokio::test]` anywhere in the
workspace. Every task is registered in a `TaskTracker`, and
`close_and_wait(budget)` returns `Err(n)` with the number still running rather
than a silent success. 177 lines of source, 201 of tests.

### Tooling and docs

`clippy.toml` (adoption A2), `scripts/check-architecture.sh` (113 lines),
`scripts/compile-fail.sh` (86 lines), `dev-docs/SdaxContract-v1.md`,
`dev-docs/Stage0-TDD-Log.md`, this report, `AGENTS.md`, and the README's
Status section.

---

## 2. Gate results, verbatim

Run on 2026-09-05 from a clean tree. Commands and exit codes exactly as issued.

```text
$ cargo fmt --check
(exit 0)
```

```text
$ cargo test --workspace --locked --offline
running 61 tests
...
test result: ok. 60 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running tests/witness.rs (target/debug/deps/witness-ff34aaec09cf6f7a)
running 6 tests
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running unittests src/lib.rs (target/debug/deps/sdax_testkit-db57eeb6eb6e85c4)
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running tests/testkit.rs (target/debug/deps/testkit-76c8e6cb59529af5)
running 7 tests
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running unittests src/lib.rs (target/debug/deps/sdax_tokio-922c729bfe7c3271)
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running tests/adapter.rs (target/debug/deps/adapter-6ae0ef91eac7d2a9)
running 9 tests
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests sdax
running 15 tests
test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.36s

   Doc-tests sdax_testkit
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests sdax_tokio
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
(exit 0)
```

The one ignored test is `tests::planner_view::show_renderings`, which prints two
`inspect()` renderings for eyeballing and asserts nothing.

```text
$ cargo clippy --workspace --all-targets --locked --offline -- -D warnings
    Checking sdax v0.1.0 (/Users/owebeeone/limbo/sdax-wz/sdax-rs/crates/sdax)
    Checking sdax-testkit v0.1.0 (/Users/owebeeone/limbo/sdax-wz/sdax-rs/crates/sdax-testkit)
    Checking sdax-tokio v0.1.0 (/Users/owebeeone/limbo/sdax-wz/sdax-rs/crates/sdax-tokio)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.36s
(exit 0)
```

```text
$ cargo package -p sdax --locked --offline --allow-dirty
   Packaging sdax v0.1.0 (/Users/owebeeone/limbo/sdax-wz/sdax-rs/crates/sdax)
    Packaged 34 files, 239.7KiB (62.5KiB compressed)
   Verifying sdax v0.1.0 (/Users/owebeeone/limbo/sdax-wz/sdax-rs/crates/sdax)
   Compiling sdax v0.1.0 (/Users/owebeeone/limbo/sdax-wz/sdax-rs/target/package/sdax-0.1.0)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.60s
(exit 0)
```

```text
$ ./scripts/check-architecture.sh
  sdax           role: pure/contract
  sdax-testkit   role: harness
  sdax-tokio     role: implementation

ARCHITECTURE GATE PASSED
(exit 0)
```

```text
$ ./scripts/compile-fail.sh
== toolchain
rustc 1.96.0 (ac68faa20 2026-05-25)
== building the rlib the witnesses link against
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.00s
14
PASS  00_W_01.rs: rejected with E0425
PASS  01_W_02.rs: rejected with E0271
PASS  02_W_03.rs: rejected with E0451
PASS  03_W_04.rs: rejected with E0599
PASS  04_W_05.rs: rejected with E0631
PASS  05_W_06.rs: rejected with E0308
PASS  06_W_07.rs: rejected with E0425
PASS  07_W_09.rs: rejected with E0599
PASS  08_W_10.rs: rejected with E0599
PASS  09_W_11.rs: rejected with E0061
PASS  10_W_12.rs: rejected with E0308
PASS  11_W_13.rs: rejected with E0277
PASS  12_W_16.rs: rejected with E0308
PASS  13_W_17.rs: rejected with E0451
== 14 witnesses
(exit 0)
```

One extra check, not among the five required and not in CI:

```text
$ RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --offline
(exit 0)
```

Every public item is documented (`#![warn(missing_docs)]` plus clippy's
`-D warnings`), so a broken intra-doc link is a real defect; two were found this
way and fixed.

The six error codes B's `typed_keys` experiment reported (`E0425`, `E0271`,
`E0451`, `E0599`, `E0631`, `E0308`) are reproduced against this crate, and the
five rows B left `[uncompiled]` (W-07, W-09, W-10, W-11, W-13) are now compiled,
with the codes B projected.

---

## 3. Measured timings

**Machine**: Apple M3 Pro (Mac15,7), 12 cores, 36 GiB, macOS 26.6.2.
**Toolchain**: rustc 1.96.0 (ac68faa20 2026-05-25), cargo 1.96.0.
Measured with `/usr/bin/time -p`, `real` seconds. These are measurements of this
tree on this machine, not budgets and not a guarantee.

| what | command | measured (five consecutive runs) |
|---|---|---|
| test execution (everything already built) | `cargo test --workspace --locked --offline` | 0.60, 0.55, 0.53, 0.53, 0.53 s |
| warm incremental build + test | `touch crates/sdax/src/lib.rs; cargo test --workspace --locked --offline` | 3.61, 4.41, 3.68, 3.78, 3.79 s |
| cold build, whole workspace | `rm -rf $T; CARGO_TARGET_DIR=$T cargo build --workspace --locked --offline` | 4.14 s |
| cold build + test, whole workspace | `rm -rf $T; CARGO_TARGET_DIR=$T cargo test --workspace --locked --offline` | 6.74 s |
| cold build, the std-only core alone | `rm -rf $T; CARGO_TARGET_DIR=$T cargo build -p sdax --locked --offline` | 0.58 s |

About 0.3 s of the execution figure is rustdoc compiling the crate-level
example and the fourteen compile-fail witnesses; the 66 unit and integration
tests themselves report `0.00s`.

The fast loop is `cargo test --workspace --locked --offline`: about 0.55 s of
execution, about 3.8 s including the incremental rebuild after a core edit. No
test sleeps and no test touches the network; every clock in the suite is
injected.

## 4. Lines of code

Counted with `find <dir> -name '*.rs' | xargs wc -l | tail -1` — physical lines
including doc comments, which are a large share of this crate on purpose.

| crate | library source (excluding the in-`src` test modules) | tests |
|---|---|---|
| `sdax` | 4 912 | 2 018 in `src/tests/**` (suite (b)) + 300 in `tests/witness.rs` |
| `sdax-testkit` | 356 | 155 |
| `sdax-tokio` | 177 | 201 |
| scripts | — | 199 (`check-architecture.sh` 113, `compile-fail.sh` 86) |
| **workspace `.rs` total** | **8 119** | |

So the non-test library surface is 4 912 lines in `sdax` (of which 231 are the
compile-fail witness suite, which lives in `src` because it is documentation),
356 in the testkit and 177 in the adapter — against B's projection of ~1 800 for
stage 0's core. The overshoot is documentation: every public item is documented,
every V-rule's rustdoc states its decision procedure, and every invariant is
named by id where it is implemented. The four items the contract added
(`spawns` + `Child::ready`, `.persistent()`, `Mode`, `RecordOrder`) account for
perhaps 200 of it.

## 5. What is deferred

| deferred to | what |
|---|---|
| Stage 1 | `engine::Machine` and `Machine::step`; `Plan::simulate`; the testkit's `Script`, `Schedule` and `ScriptedDriver`; the trace-level invariant checker (INV-2, INV-3, INV-4, INV-7…INV-16); suite (c) `C-01…C-41`; `Child::ready` and `cx.spawn` behaviour; the run-time enforcement of `SpawnError::{UndeclaredTemplate, ScopeStopping}`; producing a `Report` already in the F4 order |
| Stage 2 | `Plan::start`, `Running`, the tokio run driver, the drop guard and the drainer, blocking pools, `TokioRuntime::shutdown`; suite (d) `R-*`; re-running suite (c) against the adapter |
| Stage 3 | dynamic instances end to end |
| not planned here | `PlanIr::from_json` (A's data front-end), a `LocalRuntime` for `!Send` bodies, backoff jitter |

Canonical-test rows not implemented in Stage 0, with the reason:

- **P-15** as written needs `Plan::start`. Its decision procedure (`L-IMPORTS`)
  is implemented and tested as `Plan::unresolved_imports()`; the refusal at
  `start` is Stage 1.
- **P-16, P-17** are machine step tests. Stage 1.
- **P-14**'s second half ("the machine's effect list ends with `End`") is
  Stage 1; the mode half is tested.
- **P-13**'s "engine default changed" annotation is tested by simulating a later
  crate version over a cloned IR, because one crate version cannot produce two
  different defaults.

## 6. Deviations from the contract, and why

1. **`PlanBuilder::spawns(service_key, &template)` exists** alongside the chain
   form. Without it `V-SPAWN-SELF-IMPORT` is unreachable: `spawns(&t)` requires
   the template registered first, a template's `import` can only name keys that
   already exist, and keys only exist after their node — so in the chain form a
   template can never name the service that spawns it. The late form is the one
   way to write the mistake, and therefore the one way to test the rule. It is
   documented as such in `builder.rs`.
2. **`V-POOL-STARVE`'s "resident holders = ∞" branch is not reachable through
   the surface.** Pools are per plan, so a child plan naming a parent pool is
   caught earlier by `V-FOREIGN-KEY`; B's P-09 variant has no spelling. The
   branch is kept (B's rule text says templates count as unbounded) and is
   covered by a white-box test that builds the IR directly. Both facts are
   stated in the test.
3. **`TokioRuntime::with_observer`**, not `.observer(..)`: the `Runtime`
   contract already has an `observer(&self)` reader and one name cannot be both.
4. **`Phase::ReleaseBody`**, not `Phase::Release`: `Release` is already the
   phase marker type on `Cx<Release>`.
5. **`Serving::new(handle, serve)`** as B proposed, rather than the experiment's
   `cx.serving(..)` method form.
6. **`sdax-testkit`'s INV-5 and INV-6 checks have no negative fixture.** The
   core derives the release order from the edge set, so the two computations
   cannot disagree unless the core regresses — which is what the checks are for.
   They are regression guards with positive coverage over every plan shape, and
   `invariants.rs` says so rather than implying independent falsification.
7. **The `Display` rendering is close to but not byte-identical with
   `Proposal.md` § A.5.** P-03 asserts the header, the node lines, the exact
   declared edge set, the layers, the release graph and the `why` lines; it does
   not assert whitespace.

## 7. Verification limits, stated plainly

- **No lifecycle behaviour is verified.** Nothing in this crate starts, orders,
  cancels, retries, releases or reports a run, because none of that exists yet.
  Every test here is about types, recorded declarations, decision procedures,
  or one seam mechanism (`hold` registering in the completing poll) polled by
  hand.
- **MSRV 1.75 is respected by construction, not by measurement.** No toolchain
  older than 1.85 is installed and the network is off, so the check actually
  run is `rustup run 1.85.0 cargo test --workspace --locked --offline`, which
  passes (all suites green). That rules out APIs newer than 1.85; it does not
  prove 1.75. The library code deliberately avoids `LazyLock` (1.80),
  `Waker::noop` (1.85), `async fn` in traits (1.75, but not `dyn`-safe) and
  `#[expect]` (1.81); `OnceLock` (1.70) and `let … else` (1.65) are used.
  Test code uses nothing newer than library code.
- **The architecture gate reads declared manifests**, not a resolved
  third-party audit. It was checked against two hand-made violations (see the
  TDD log, step 12), which is negative evidence for those two rules only.
- **The clippy raw-spawn gate catches direct calls to the six named paths.** It
  does not catch a spawn reached through a re-export, another runtime, or a
  helper crate. It moves X5 from silent to caught for the common form, which is
  what A claimed for it, and no more.
- **`idempotent` is trusted, not verified**, as in B.

## 8. Open questions for the owner

1. **`Plan::simulate` is declared but unbuilt.** It is the strongest argument
   for the staged plan (a counterfactual answered pre-ship with production
   code). Confirm it stays in Stage 1 rather than sliding to Stage 2.
2. **`Mode::Finite` and components.** `V-MODE` looks only at the plan's own
   nodes: a finite parent may contain a `Resident` component, which becomes
   Ready when its inner run is steady and is then cleaned up with the parent.
   That reading is deliberate but is not in the proposal; confirm it.
3. **`RecordOrder` is node-major within an instance** (an instance's records are
   grouped, then ordered by inner declaration index). F4 admits another reading
   (all instances of a node together). The chosen order is in the contract § 8.
4. **`try_step`'s value type** is B's `Result<T, Error>` (OD-5). Unchanged, and
   still open.
5. **The error type** is the fixed `Box<dyn Error + Send + Sync>` (OD-2).
   Unchanged, and still open.
6. **`Plan::unresolved_imports` returns `Vec<RawKey>`.** It should probably
   return names once Stage 1 can resolve a child's path; the current shape is
   the minimum that makes `L-IMPORTS` decidable.
7. **Should `sdax-tokio`'s `close_and_wait` stay?** It is not in the proposal;
   it exists so tokio-util earns its place in Stage 0 and so INV-15 has a
   mechanism to build on. Stage 2 may fold it into `TokioRuntime::shutdown`.

## 9. What Stage 1 needs from this

- `engine::{Event, Effect}` are the machine's input and output vocabulary and
  are already public and stable. `Machine::new(&Plan)` and `Machine::step` are
  the only additions the contract expects in `sdax::engine`.
- `PlanIr` is `pub(crate)` today, reachable through `Plan::inspect()`. The
  machine lives in this crate, so it needs no new visibility.
- The bodies are recorded and erased in `plan::Bodies` (`prepare`, `release`,
  `blocking`), keyed by declaration index and marked `#[allow(dead_code)]` until
  the driver reads them. `CxInner::{take_held, take_serve, hold_count}` are the
  engine-side hand-offs, and `hold_count > 1` is `FaultKind::DoubleHold`.
- `cx::Scope` and `cx::ChildControl` are the two traits the run must implement
  for `cx.spawn` and `Child::ready` to work; `SpawnError::NotRunning` is what
  the seam answers until one is attached.
- `sdax_testkit::invariants` is where the trace-level INV checks belong, beside
  the static ones.
