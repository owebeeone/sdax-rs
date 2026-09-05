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
- `crates/sdax-tokio` — tokio `Runtime` adapter; the run driver and drop-guard
  drainer arrive in Stage 2. The only crate that may mention tokio types, and
  the only one that may spawn a raw task.
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

# Whole workspace (the fast loop: ~1.65 s of execution, ~8.5 s with a rebuild).
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

**Stage 1 — the engine, simulated. No run driver.** Version `0.1.0` is
unreleased and nothing is frozen.

**Two surfaces.** The crate root and `sdax::prelude` are the **author** API:
what a plan is written, validated, inspected and read back against. `sdax::host`
is the **host** API: what a runtime adapter, a run driver or the engine needs and
an author does not — `Runtime`, `TaskHandle`, `Joined`, `Clock`, `Time`,
`Observer`, `NoObserver`, `BoxFuture`, `Scope`, `ChildControl`, `InstanceId`,
`StopSignal`, `CxInner`, `RawKey`, `SEMANTICS` and
`host::engine::{Event, Effect, TimerId, JoinedLabel}`. **Only the author surface
carries the stability promise**; `sdax::host` may change in a minor version
before 1.0, because the run driver is still being written — Stage 1 already
changed four of its signatures (`Event::NodeErr` and `Event::ServeEnded` carry
their fault, `Effect::CancelTimer` and `Effect::Reject` are new).
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

What does not exist, and is not stubbed: `Plan::start`, `Running`, the tokio run
driver, the drop guard and the drainer (**Stage 2**); `cx.spawn`, `Child::ready`
and live template instances (**Stage 3** — `Effect::SpawnInstance` is in the
vocabulary and the machine never emits it). Five suite-(c) rows are omitted for
those reasons and named in `dev-docs/Stage1Report.md` § 7, which also states
that `S-02` (exhaustive schedule enumeration) did not run.

`dev-docs/Stage0Report.md` and `dev-docs/Stage1Report.md` state what was
measured, what is deferred, and where verification stops.
