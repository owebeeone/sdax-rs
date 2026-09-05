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
| `src/lib.rs` | 169 | crate docs (with a runnable example), module wiring, the **author** re-export list, `prelude` |
| `src/host.rs` + `src/host/engine.rs` | 46 + 137 | the **host** surface: `Runtime`, `TaskHandle`, `Joined`, `Clock`, `Time`, `Observer`, `NoObserver`, `BoxFuture`, `Scope`, `ChildControl`, `InstanceId`, `StopSignal`, `CxInner` and its hand-offs, `RawKey`, `SEMANTICS`, and `host::engine::{Event, Effect, TimerId, JoinedLabel}` — the vocabulary, with no machine |
| `src/key.rs` | 177 | `RawKey`, `Key<T>` (`Copy`, unbranded), `Deps` for `()`/`Key<A>`/tuples to 8, `Slots` |
| `src/policy.rs` | 211 | `Policy`, `Shutdown`, `Mode`, `Ambiguity`, `Retry`, `Backoff`, `Restart`, `CancelMode` |
| `src/plan.rs` | 407 | `Plan<Out, In>`, `PlanIr`, `NodeDecl`, `Attrs`, `Kind`, `ReleaseStyle`, `Pool`, `Template`, `SEMANTICS` |
| `src/builder.rs` | 446 | `PlanBuilder`, `Node<'b, D, K>`, the kind markers and typestates, generic attributes, `release::by_drop` |
| `src/terminals.rs` | 297 | one terminal per kind, `NeedsRelease`, `NeedsCompensate` (`.compensate` \| `.persistent`) |
| `src/shorthand.rs` | 143 | the seven positional constructors (adoption A1) |
| `src/validate.rs` + `src/validate/{rules,budgets}.rs` | 207 + 340 + 279 | `Rule` (15), `Finding`, `Invalid`, and one function per rule with its decision procedure — `rules` for a declaration's shape, `budgets` for policy, pools and budgets |
| `src/view.rs` + `view/{model,render,diff}.rs` | 507 + 155 + 121 + 171 | `PlanView`, `NodePath`, `Edge`, `Why`, `Effects`, `ReleaseOrder`, `Display`, `PlanDiff` |
| `src/cx.rs` + `src/cx/instances.rs` | 437 + 186 | the seam: `Cx<Phase>`, `CxInner`, `Hold`/`Held`, `Serving`, `StopSignal`/`Stop`, `Child`, `Scope`, `SpawnError`, `Timeout` |
| `src/contracts.rs` | 139 | `Time`, `Clock`, `Runtime`, `TaskHandle`, `Joined`, `Observer`, `NoObserver` |
| `src/report.rs` | 348 | `Report`, `Outcome`, `Fault`, `FaultKind`, `FaultLabel`, `Phase`, `RecordOrder`, `Trace`, `TraceEvent`, `TraceKind` |
| `src/compile_fail.rs` | 248 | suite (a): 15 `compile_fail` doctests, each naming its error code |
| `src/tests/**` | 2202 | suite (b): the corpus programs and the `P-*` tests |
| `tests/witness.rs` | 301 | suite (a) compile-pass: the crate as an author sees it |
| `tests/surface.rs` | 277 | suite (a) compile-pass: every author item at the root and in `prelude`, and every host item under `sdax::host` |

### `crates/sdax-testkit` — the harness (`publish = false`)

`FakeClock` (advance by hand, never by itself), `TraceRecorder` (observation
order, never sorted), and `invariants::{check_plan, check_clock, poll_once}` —
a static checker that recomputes the edge set and the ordering closure itself
rather than asking the core to confirm its own answer. 358 lines of source,
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
| `sdax` | 5 171 | 2 202 in `src/tests/**` (suite (b)) + 301 in `tests/witness.rs` + 277 in `tests/surface.rs` |
| `sdax-testkit` | 358 | 155 |
| `sdax-tokio` | 177 | 201 |
| scripts | — | 199 (`check-architecture.sh` 113, `compile-fail.sh` 86) |
| **workspace `.rs` total** | **8 843** | |

So the non-test library surface is 5 171 lines in `sdax` (of which 248 are the
compile-fail witness suite, which lives in `src` because it is documentation),
358 in the testkit and 177 in the adapter — against B's projection of ~1 800 for
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
   documented as such in `builder.rs`. Because it takes any key rather than a
   `Node<_, _, Service>`, `build` decides what it named: `V-SPAWN-KIND` for a
   node that is not a service, `V-FOREIGN-KEY` for a key of another plan (see
   the post-review section below).
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
   — **Decided 2026-09-05: confirmed.** A scope owns its own mode; the
   alternative would make `Mode` derived again, which F3 exists to remove.
   Contract § 2 states it and § 13 records it as `OD-MODE`.
3. **`RecordOrder` is node-major within an instance** (an instance's records are
   grouped, then ordered by inner declaration index). F4 admits another reading
   (all instances of a node together). The chosen order is in the contract § 8.
4. **`try_step`'s value type** is B's `Result<T, Error>` (OD-5). Unchanged, and
   still open. — **Decided 2026-09-05: kept.** Contract § 13, `OD-5`.
5. **The error type** is the fixed `Box<dyn Error + Send + Sync>` (OD-2).
   Unchanged, and still open. — **Decided 2026-09-05: kept.** `?` infers it in
   a body with no annotation (witnessed in W-14) and typed errors survive as
   `downcast_ref` targets on `FaultKind::Error`. Contract § 13, `OD-2`.
6. **`Plan::unresolved_imports` returns `Vec<RawKey>`.** It should probably
   return names once Stage 1 can resolve a child's path; the current shape is
   the minimum that makes `L-IMPORTS` decidable. — **Closed 2026-09-05:** it
   returns `Vec<NodePath>`, naming this plan's import nodes. It still cannot
   name the ancestor key behind each one — that key belongs to a plan whose
   declaration is not in scope — and the rustdoc says so. Contract § 13,
   `OD-IMPORTS`.
7. **Should `sdax-tokio`'s `close_and_wait` stay?** It is not in the proposal;
   it exists so tokio-util earns its place in Stage 0 and so INV-15 has a
   mechanism to build on. Stage 2 may fold it into `TokioRuntime::shutdown`.

## 9. What Stage 1 needs from this

- `host::engine::{Event, Effect}` are the machine's input and output vocabulary
  and are already public. `Machine::new(&Plan)` and `Machine::step` are the only
  additions the contract expects in `sdax::host::engine`.
- `PlanIr` is `pub(crate)` today, reachable through `Plan::inspect()`. The
  machine lives in this crate, so it needs no new visibility.
- The bodies are recorded and erased in `plan::Bodies` (`prepare`, `release`,
  `blocking`), keyed by declaration index and marked `#[allow(dead_code)]` until
  the driver reads them. `host::CxInner::{take_held, put_output, take_serve,
  hold_count}` are the engine-side hand-offs, and `hold_count > 1` is
  `FaultKind::DoubleHold`. They are under `host` rather than `pub(crate)`
  because the Stage 2 run driver lives in `sdax-tokio`.
- `host::Scope` and `host::ChildControl` are the two traits the run must
  implement for `cx.spawn` and `Child::ready` to work; `SpawnError::NotRunning`
  is what the seam answers until one is attached.
- `sdax_testkit::invariants` is where the trace-level INV checks belong, beside
  the static ones.

---

## Post-review fixes (2026-09-05)

An external reviewer read Stage 0 at commit `6a95537` and asked for three
things; the owner accepted them plus two adjacent holes the manager found while
verifying. `dev-docs/Review-2026-09-05-External.md` carries the reviewer's text
and the owner's decisions. Nothing here was committed, tagged or pushed, and no
network was used.

The numbers in § 1 and § 4 above were refreshed to describe the tree after these
fixes; § 2's gate block is left as the record of the pre-review run, and the
block at the end of this section is the run after them.

### 1. Two surfaces: author at the root, host under `sdax::host`

Before 0.1.0 is published, and a break afterwards. The crate root and
`prelude` re-exported engine and host internals — `CxInner` (with public
`take_held`, `put_output`, `take_serve`, `hold_count`), `Slots`, `RawKey`,
`StopSignal`, `Scope`, `ChildControl`, `InstanceId`, `SEMANTICS`,
`pub mod engine`, `pub mod compile_fail`, and `Runtime`/`Observer`/`Clock`/`Time`
in the prelude.

- **The crate root and `prelude` are now the author API** and nothing else: the
  builder and its kind markers, `Plan`, `Key`, `Deps`, `Cx` and its phases,
  `Held`, `Hold`, `Serving`, `Child`, `Template`, `Pool`, the policy types, the
  validate values, the view values, the report values, `Error`, `Timeout`,
  `SpawnError`, and `Duration` in the prelude. The prelude is now the root's
  author half rather than a subset of it, so `use sdax::prelude::*` is enough to
  write, validate, inspect and read back a plan; it no longer exports `Runtime`,
  `Observer`, `Clock` or `Time`, which an author never implements or names.
- **`sdax::host` (`src/host.rs`) is the adapter/driver/engine API**: `Runtime`,
  `TaskHandle`, `Joined`, `Clock`, `Time`, `Observer`, `NoObserver`,
  `BoxFuture`, `Scope`, `ChildControl`, `InstanceId`, `StopSignal`, `CxInner`,
  `RawKey`, `SEMANTICS`, a `#[doc(hidden)]` `Slots`, and
  `host::engine::{Event, Effect, TimerId, JoinedLabel}` (`src/engine.rs` moved to
  `src/host/engine.rs`, so `host::engine` is its real path and not a re-export).
  Its module doc says plainly that it is not author API and may change in a
  minor version before 1.0.
- `Cx::new` and `Cx::inner` stay methods on the public `Cx` — a method cannot
  move into a module — and their rustdoc says they are host API; they are only
  callable with a `host::CxInner` in hand.
- `compile_fail` is `#[doc(hidden)] pub mod`. The doctests still run: the count
  went 15 → **16** (15 witnesses + the crate-level example), not 15 → 2.
- **`RawKey` could not become `pub(crate)`**, and is `pub` under `host` instead.
  Four public signatures name it and none of them can stop: `Deps::raw_keys`
  (a public trait an author's `needs` tuples implement), `Key::raw`,
  `Finding.keys`, and the `host::engine::{Event, Effect}` variants. A
  `pub(crate)` `RawKey` is a private-type-in-public-interface error, not a
  smaller surface. `Slots` is the same shape for `Deps::fetch`, and is
  `#[doc(hidden)]` under `host` as the brief directed.
- **`Plan::unresolved_imports()` returns `Vec<NodePath>`** (report question 6,
  now closed): this plan's import nodes, named the way the report, the trace and
  `inspect()` name every node. It cannot name the ancestor key behind each one —
  that key belongs to a plan whose declaration is not in scope — and the rustdoc
  says so rather than implying otherwise.
- `sdax-testkit` and `sdax-tokio` now import from `sdax::host`. Nothing else in
  either crate changed, which is the point: the split is a rename of where the
  host contracts live, not a change to them.
- **Pinned by two witnesses.** `crates/sdax/tests/surface.rs` (277 lines) names
  every intended root, `prelude` and `host` item and builds a plan through the
  prelude alone, so a move or a rename stops it compiling. Its mirror is the new
  compile-fail witness **S-01** — `use sdax::CxInner;` must fail with `E0432` —
  because only a program that fails to compile can witness an absence.
  `scripts/compile-fail.sh` asserts that code with the other fourteen; W-03's
  witness now reaches for `sdax::host::RawKey`, so it still fails with `E0451`
  (private fields) rather than for the wrong reason.

### 2. The late-`spawns` hole

`PlanBuilder::spawns(key, &template)` looked the key up and pushed the template
with no kind check, and **silently did nothing** when the key was not found.

- The declaration is now recorded whatever it names: onto the node when the key
  is this plan's, and onto `PlanIr::foreign_spawns` when it is not.
- **`V-SPAWN-KIND`** (new, 15th rule): `spawns` on a node that is not a service
  is a finding naming the node, its kind and the template, with the fix "only a
  service may spawn".
- **`V-FOREIGN-KEY`** now covers `foreign_spawns`, naming the key and both
  plans exactly as it does for `needs`, after the per-node findings.
- `inspect()` shows `spawns` only on a service.
- The late form itself stays: it is still the only way to write the
  `V-SPAWN-SELF-IMPORT` deadlock, and therefore the only way to test that rule.

### 3. Repeated or conflicting lock attributes

`.exclusive(k)` and `.shared(k)` record into their own lists rather than into
`attrs.declared`, so `.exclusive(db).exclusive(db)` and
`.exclusive(db).shared(db)` slipped past `V-DUP-ATTR`. Both are now findings.
The check is **per key**, not per name: `.exclusive(a).exclusive(b)` locks two
different resources and is still accepted, which a name-based check would have
rejected. `attrs.declared` was deliberately left alone — it means "the author
set this, so the shown value is not an engine default", and a lock has no engine
default. One consequence is left unfixed and recorded here: `PlanView::diff`
marks a *changed lock target* as `default_changed`, because neither side lists
`exclusive` in `declared`. It is pre-existing, out of the scope the owner
approved, and worth a line of its own later.

### 4. Decisions recorded

`OD-2` (the boxed seam error), `OD-5` (`try_step` → `Key<Result<T, Error>>`),
the finite-parent/resident-component reading and the `unresolved_imports` shape
are now decided, with dates and reasons, in `SdaxContract-v1.md` § 13. § 8 above
annotates each question rather than deleting it.

### Refactor after green

`validate/rules.rs` reached 606 lines, over the ~500-line house rule, so it was
split by responsibility: `validate/rules.rs` (340) keeps the rules about a
declaration's shape — keys, names, attributes, spawns, locks — and
`validate/budgets.rs` (279) takes the rules about policy, pools and budgets.
Behaviour unchanged; the suite was re-run.

### Gate results, verbatim

Run on 2026-09-05 after the fixes above. Commands and exit codes exactly as
issued; the 66-line unit-test name list is elided with `...` as in § 2.

```text
$ cargo fmt --check
(exit 0)
```

```text
$ cargo test --workspace --locked --offline
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.01s
     Running unittests src/lib.rs (target/debug/deps/sdax-85a8d00691f282c8)

running 66 tests
...

test result: ok. 65 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running tests/surface.rs (target/debug/deps/surface-91c80d6b90f0abbc)

running 4 tests
test the_driver_hand_offs_are_reachable_from_another_crate ... ok
test the_host_surface_carries_the_semantics_tag_and_the_engine_vocabulary ... ok
test unresolved_imports_names_the_import_nodes_rather_than_raw_keys ... ok
test the_author_api_builds_and_inspects_a_plan_through_the_prelude_alone ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running tests/witness.rs (target/debug/deps/witness-ff34aaec09cf6f7a)

running 6 tests
test a_release_body_can_wait_then_kill_against_the_injected_clock ... ok
test a_service_hands_over_a_serve_future_that_observes_stop ... ok
test hold_registers_in_the_completing_poll ... ok
test stage_0_has_no_execution_and_says_so ... ok
test a_plan_records_typed_declarations_and_is_send_sync ... ok
test validation_findings_are_values ... ok

test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running unittests src/lib.rs (target/debug/deps/sdax_testkit-db57eeb6eb6e85c4)

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running tests/testkit.rs (target/debug/deps/testkit-76c8e6cb59529af5)

running 7 tests
test the_fake_clock_satisfies_the_clock_contract ... ok
test the_fake_clock_never_advances_by_itself ... ok
test the_trace_recorder_keeps_observation_order ... ok
test the_static_checker_accepts_a_well_formed_plan ... ok
test the_static_checker_catches_an_edge_the_author_did_not_declare ... ok
test the_static_checker_agrees_with_the_views_unordered_pairs ... ok
test the_static_checker_holds_for_nested_and_dynamic_shapes ... ok

test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running unittests src/lib.rs (target/debug/deps/sdax_tokio-922c729bfe7c3271)

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running tests/adapter.rs (target/debug/deps/adapter-6ae0ef91eac7d2a9)

running 9 tests
test a_shutdown_that_runs_out_of_budget_says_what_is_still_running ... ok
test an_aborted_task_joins_as_cancelled_and_the_abort_is_deferred ... ok
test a_spawned_task_runs_and_joins_as_done ... ok
test an_observer_can_be_installed_and_defaults_to_recording_nothing ... ok
test tokio_runtime_implements_the_runtime_contract ... ok
test every_spawned_task_is_tracked_so_a_shutdown_can_wait_for_it ... ok
test a_panicking_task_joins_as_panicked_and_the_panic_does_not_escape ... ok
test the_tokio_clock_only_moves_when_tokio_time_moves ... ok
test a_blocking_body_runs_off_the_async_workers ... ok

test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests sdax

running 16 tests
test crates/sdax/src/compile_fail.rs - compile_fail (line 191) - compile fail ... ok
test crates/sdax/src/compile_fail.rs - compile_fail (line 228) - compile fail ... ok
test crates/sdax/src/compile_fail.rs - compile_fail (line 134) - compile fail ... ok
test crates/sdax/src/compile_fail.rs - compile_fail (line 17) - compile fail ... ok
test crates/sdax/src/compile_fail.rs - compile_fail (line 49) - compile fail ... ok
test crates/sdax/src/compile_fail.rs - compile_fail (line 212) - compile fail ... ok
test crates/sdax/src/compile_fail.rs - compile_fail (line 31) - compile fail ... ok
test crates/sdax/src/compile_fail.rs - compile_fail (line 161) - compile fail ... ok
test crates/sdax/src/compile_fail.rs - compile_fail (line 244) - compile fail ... ok
test crates/sdax/src/compile_fail.rs - compile_fail (line 173) - compile fail ... ok
test crates/sdax/src/compile_fail.rs - compile_fail (line 149) - compile fail ... ok
test crates/sdax/src/compile_fail.rs - compile_fail (line 109) - compile fail ... ok
test crates/sdax/src/compile_fail.rs - compile_fail (line 76) - compile fail ... ok
test crates/sdax/src/compile_fail.rs - compile_fail (line 64) - compile fail ... ok
test crates/sdax/src/compile_fail.rs - compile_fail (line 95) - compile fail ... ok
test crates/sdax/src/lib.rs - (line 32) ... ok

test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.45s

   Doc-tests sdax_testkit

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests sdax_tokio

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

(exit 0)
```

The one ignored test is still `tests::planner_view::show_renderings`. The unit
suite grew from 61 to 65 passing: `v_spawn_kind_rejects_a_late_spawns_on_a_node_that_is_not_a_service`,
`v_foreign_key_catches_a_late_spawns_whose_key_belongs_to_another_plan`,
`a_service_may_spawn_in_the_chain_form_and_in_the_late_form`,
`p12_repeated_or_conflicting_lock_attributes_are_rejected` and
`spawns_is_shown_only_on_a_service` are new, and `tests/surface.rs` adds four
more. Doctests went 15 → 16 with witness S-01.

```text
$ cargo clippy --workspace --all-targets --locked --offline -- -D warnings
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.04s
(exit 0)
```

```text
$ cargo package -p sdax --locked --offline --allow-dirty
   Packaging sdax v0.1.0 (/Users/owebeeone/limbo/sdax-wz/sdax-rs/crates/sdax)
    Packaged 38 files, 267.1KiB (70.0KiB compressed)
   Verifying sdax v0.1.0 (/Users/owebeeone/limbo/sdax-wz/sdax-rs/crates/sdax)
   Compiling sdax v0.1.0 (/Users/owebeeone/limbo/sdax-wz/sdax-rs/target/package/sdax-0.1.0)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.35s
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
15
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
PASS  14_S_01.rs: rejected with E0432
== 15 witnesses
(exit 0)
```

```text
$ RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --offline
    Checking sdax v0.1.0 (/Users/owebeeone/limbo/sdax-wz/sdax-rs/crates/sdax)
 Documenting sdax v0.1.0 (/Users/owebeeone/limbo/sdax-wz/sdax-rs/crates/sdax)
 Documenting sdax-tokio v0.1.0 (/Users/owebeeone/limbo/sdax-wz/sdax-rs/crates/sdax-tokio)
 Documenting sdax-testkit v0.1.0 (/Users/owebeeone/limbo/sdax-wz/sdax-rs/crates/sdax-testkit)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.43s
   Generated /Users/owebeeone/limbo/sdax-wz/sdax-rs/target/doc/sdax/index.html and 2 other files
(exit 0)
```
