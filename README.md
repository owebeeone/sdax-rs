# sdax-rs

Rust implementation of SDAX: a declarative async lifecycle orchestrator
(typed plan, derived release graph, reverse teardown). Python `sdax` is the
inspiration; this is a Rust-native design, not a port.

Nothing public is frozen yet. The crate layout follows the sdax-v1 comparison
(Proposal B semantics under the `sdax` name): a std-only core, a tokio adapter,
and a dev-only testkit. Stop at machine + testkit before buying an engine.

## Layout

- `crates/sdax` — core library. Authoring surface, inspectable plan, validator,
  the seam, and (from Stage 1) the pure event/effect machine. No tokio types.
  This is the crates.io package.
- `crates/sdax-tokio` — tokio `Runtime` adapter; the run driver and drop-guard
  drainer arrive in Stage 2. The only crate that may mention tokio types, and
  the only one that may spawn a raw task.
- `crates/sdax-testkit` — fake clock, trace recorder, static invariant checks;
  the scripted driver arrives in Stage 1. `publish = false`; never a production
  dependency.
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

# Whole workspace (the fast loop: ~0.25 s of execution, ~3.2 s with a rebuild).
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

**Stage 0 — the contract, the validator and the seam. No engine.** Version
`0.1.0` is unreleased and nothing is frozen.

What exists:

- the complete authoring surface — resources, steps, try-steps, blocking steps,
  services, effects (compensated or persistent), joins, components, templates —
  in a chain form and a positional shorthand that records the same declaration;
- `build(policy, shutdown, mode)` with 14 validate rules, each reporting a
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

What does not exist, and is not stubbed: `engine::Machine`, `Plan::simulate`,
`Plan::start`, `Running`, the scripted driver, the run driver and the drainer.
`dev-docs/Stage0Report.md` states what was measured, what is deferred, and where
verification stops.
