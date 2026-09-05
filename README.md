# sdax-rs

Rust implementation of SDAX: a declarative async lifecycle orchestrator
(typed plan, derived release graph, reverse teardown). Python `sdax` is the
inspiration; this is a Rust-native design, not a port.

Nothing public is frozen yet. The crate layout follows the sdax-v1 comparison
(Proposal B semantics under the `sdax` name): a std-only core, a tokio adapter,
and a dev-only testkit. Stop at machine + testkit before buying an engine.

## Layout

- `crates/sdax` — core library. Authoring surface, inspectable plan, validator,
  the seam, the pure event/effect machine and the stepping simulator behind
  `Plan::simulate`. No tokio types.
  This is the crates.io package. Its crate root and `prelude` are the **author**
  API; `sdax::host` is the **host** API (see Status).
- `crates/sdax-tokio` — the tokio `Runtime` adapter **and the run driver**:
  `plan.start(rt)`, `Running` with its drop guard and drainer. The only crate
  that may mention tokio types, and the only one that may spawn a raw task.
- `crates/sdax-testkit` — fake clock, trace recorder, static invariant checks,
  the scripted driver, the trace-level invariant checker and the Monte Carlo
  suite. `publish = false`; never a production dependency.
- `dev-docs/SdaxContract-v1.md` — the normative contract (`sdax/1`).
- `scripts/release.py` — cut an immutable `vX.Y.Z` tag (see `RELEASE.md`).
- `.github/workflows/` — CI on push/PR; crates.io publish on GitHub Release.

Each crate manifest is self-contained (no `*.workspace = true` inheritance) so
external workspaces and `cargo publish` do not see dangling workspace fields.

## Build / test

```sh
# Core library (no tokio).
cargo build -p sdax
cargo test -p sdax

# Tokio adapter.
cargo build -p sdax-tokio

# Dev-only harness.
cargo build -p sdax-testkit

# Whole workspace (the fast loop: ~2.5 s of execution, ~11.7 s with a rebuild).
cargo test --workspace --locked --offline
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
cargo fmt --check

# Gates the CI does not run yet.
./scripts/check-architecture.sh   # crate roles and dependency direction
./scripts/compile-fail.sh         # the error code each compile-fail witness claims
```

## Release

```sh
python3 scripts/release.py v0.1.0
python3 scripts/release.py v0.1.0 --push --github-release
```

Publishing `sdax-tokio` waits until `sdax` of the same version is on crates.io.
Release tags are immutable. Details in [`RELEASE.md`](RELEASE.md).

## Status

**Stage 2 — the engine, running on tokio.** Version `0.1.0` is unreleased and
nothing is frozen. Dynamic template instances are Stage 3 and are not stubbed.

**Two surfaces.** The crate root and `sdax::prelude` are the **author** API:
what a plan is written, validated, inspected and read back against. `sdax::host`
is the **host** API: what a runtime adapter, a run driver or the engine needs and
an author does not — `Runtime`, `TaskHandle`, `Joined`, `Clock`, `Time`,
`Observer`, `NoObserver`, `BoxFuture`, `Scope`, `ChildControl`, `InstanceId`,
`StopSignal`, `CxInner`, `RawKey`, `SEMANTICS`, `Bodies`, `BodySource`, `Task`,
`bodies_of` and `host::engine::{Event, Effect, TimerId, JoinedLabel}`. **Only
the author surface carries the stability promise**; `sdax::host` may change in a
minor version before 1.0 — Stage 1 changed four of its signatures
(`Event::NodeErr` and `Event::ServeEnded` carry their fault, `Effect::CancelTimer`
and `Effect::Reject` are new) and Stage 2 added `Observer::report`,
`Machine::{kind_of, deadline_for, nodes}` and the `Bodies`/`BodySource` group.
`crates/sdax/tests/surface.rs` pins the author half and witness S-01 in
`sdax::compile_fail` pins the absence of the host half from the root.

What Stage 0 built:

- the complete authoring surface — resources, steps, try-steps, blocking steps,
  services, effects (compensated or persistent), joins, components, templates —
  in a chain form and a positional shorthand that records the same declaration;
- `build(policy, shutdown, mode)` with 15 validate rules, each reporting a
  `Finding` that names the rule, the node, the key and a fix;
- `inspect()`: nodes, exactly the declared edges, earliest-start layers, the
  release order as a partial order, `why`, `effects()`, `diff` and a rendering;
- the seam on `Cx<Phase>`: `hold` registering in the poll that observes the
  effect completing, `hold_value`, `Serving`, `stop`/`until_stop`, an injected
  clock, `attempt`, and the `spawn`/`Child` signatures;
- the host contracts (`Runtime`, `TaskHandle`, `Clock`, `Observer`), the report
  and trace types with a defined record order, and `engine::{Event, Effect}`;
- `sdax-tokio`'s `TokioRuntime`; `sdax-testkit`'s `FakeClock`, `TraceRecorder`
  and static plan checker.

Stage 1 adds, on top of that:

- `host::engine::Machine` — a pure, `std`-only event/effect machine
  implementing T1–T8 for static plans and components: need-readiness, atomic
  FIFO lock and pool grants, retries and backoff, `within` deadlines, service
  restarts, both fail policies, cancellation, the derived release graph, the
  shutdown budget and the report in F4 order. No clock, no tasks, no I/O;
  `Machine::step(Event) -> Vec<Effect>` is total and never panics on input;
- `Plan::simulate` and the stepping simulator behind it: a script of body
  outcomes on a virtual clock, so any interleaving is an input of the run and
  is reproducible;
- `sdax-testkit`'s `ScriptedDriver`, `eol::Eol` trace reader, and the
  trace-level invariant checker (INV-1…5, 7…12, 15, 18, 20, `orphans: none`,
  report ⊆ trace ⊆ report);
- suite (c): 41 of B's 46 `C-*` rows, plus a `regressions.rs` module of named
  tests for every bug the Monte Carlo walk found;
- a Monte Carlo suite that generates random plans and scripts, drives them
  through the machine and checks every trace, with a coverage floor per corner.
  It has found 24 defects; a 50 000-case walk and a 200 000-case walk both pass.

Stage 2 adds the part that executes:

- the **run driver** in `sdax-tokio`: a loop around `Machine::step` over an
  `mpsc` inbox, one task per attempt, one handle per node, timers as tasks,
  aborts joined before a node counts as settled (T5), a superseded attempt's
  outcome dropped by epoch, and a body's registration reported in the poll that
  observed it (T2);
- `use sdax_tokio::PlanStart;` then **`plan.start(rt)`** → a `#[must_use]`
  `Running<Out>`: a `Future` for the `Report`, plus `ready()`, `shutdown()`,
  `cancel()`, `snapshot()` and a clonable `handle()`. It is **lazy** — a cancel
  before the first poll ends the run with nothing spawned — and **dropping it**
  cancels the run and leaves one tracked drainer to finish the release graph
  inside the shutdown budget, reporting to the `Observer` (`C-14`);
- `host::BodySource` and `host::bodies_of`, with `Bodies` now carrying a
  component plan's own bodies and an import's value copy;
- suite (c) **re-run against the adapter** with paused time, from the same
  sources rather than a second copy: 72 rows green, and the traces are equal to
  the pure machine's once same-instant events are normalised;
- suite (d): `R-01`…`R-07`, `C-14`, and `S-01` — the research spike, walked
  with random cancellation, drops and panics — plus a Monte Carlo walk of
  generated plans on the adapter.

What does not exist, and is not stubbed: `cx.spawn`, `Child::ready` and live
template instances (**Stage 3** — `Effect::SpawnInstance` is in the vocabulary
and the machine never emits it). Four suite-(c) rows are omitted for that reason
and named in `dev-docs/Stage1Report.md` § 7; `S-02` (exhaustive schedule
enumeration) still did not run.

`dev-docs/Stage0Report.md`, `dev-docs/Stage1Report.md` and
`dev-docs/Stage2Report.md` state what was measured, what is deferred, and where
verification stops.
