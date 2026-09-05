# Working rules for `sdax-rs`

Read this before changing anything here. `dev-docs/SdaxContract-v1.md` is the
normative contract; this file is how the work is done.

## Commits

- **Never add a `Co-Authored-By: Claude …` trailer, or any other AI attribution
  trailer, to a commit message.** This holds for every commit, including ones
  made by a delegated agent or workflow.
- **Do not commit, tag or push unless you were explicitly asked to.** Map the
  verb to that one action and stop: "commit" is not permission to tag, push,
  branch, merge, rebase or reset. Release tags are immutable; never move one.
- A release is cut by `scripts/release.py` (see `RELEASE.md`), by a person.

## Test first (LBT-007)

For every unit of behaviour: write the failing test, **see it fail**, then write
the smallest thing that passes. Record the step in `dev-docs/Stage0-TDD-Log.md`
(or the stage's own log): what was written, the RED evidence, what made it
green. For a typed surface a test that does not compile is a failing test, and
usually the only failure available before the type exists.

A compile-fail witness is the mirror image: it lives as a ```compile_fail
doctest in `crates/sdax/src/compile_fail.rs` with the error code it claims, and
`scripts/compile-fail.sh` asserts that code with `rustc`, which a doctest
cannot.

## Honest labels

- What is not executed is not described as executed. There is no engine yet;
  say so where it matters.
- **Never stub a missing capability with `todo!()`, a panic, or a function that
  returns a plausible-looking constant.** A stub is a claim the crate cannot
  make. Leave the item out, and say in the docs which stage it belongs to.
- A check with no negative fixture is a regression guard, not a proof. Say which
  it is.

## Crate roles (LBT-001)

| crate | role | rule |
|---|---|---|
| `sdax` | pure / contract | std only; **zero normal dependencies**; no tokio types |
| `sdax-tokio` | implementation | normal deps: `sdax`, `tokio`, `tokio-util`, and nothing else |
| `sdax-testkit` | harness | `publish = false`; never on a normal dependency path |

`scripts/check-architecture.sh` enforces all of it over `cargo metadata`.
Changing a role, an allowlist, or an exception is a policy change that needs
review — not a way to make a failing check green.

Each crate manifest is self-contained: concrete literals, no
`*.workspace = true` inheritance, so an external workspace or `cargo publish`
never sees a dangling field.

## No raw spawns

A body that spawns its own task creates work the engine does not own. The owned
form is `cx.spawn(&template, input)`. `clippy.toml` refuses
`tokio::{task::spawn, task::spawn_blocking, spawn}`,
`tokio::runtime::Handle::{spawn, spawn_blocking}` and `std::thread::spawn`
everywhere. `sdax-tokio` is the one exception; its two call sites carry a
scoped `#[allow(clippy::disallowed_methods)]` and a comment saying why. Do not
widen that allow, and do not add a crate-level one.

## Offline, and pinned

Work offline: `cargo build --offline`, `cargo test --offline`. Dependencies come
from the local cargo cache and are pinned exactly (`=1.53.1`, `=0.7.19`) with
minimal features — no `macros`, so there is no `#[tokio::test]` and no
proc-macro chain in a consumer's cold build. Build runtimes by hand in tests.

Do not add a dependency to `sdax`. Ever. Do not add one anywhere without a
reason recorded in the stage report.

## MSRV

The crates declare `rust-version = "1.75"`. Library code must not use anything
newer: no `LazyLock` (1.80), no `Waker::noop` (1.85), no `#[expect]` (1.81), no
`async fn` in a trait meant for `dyn` — box the future instead. Test code holds
to the same line. Verification is partial and must be reported as such: the
oldest toolchain on this machine is 1.85.

## The fast loop

```sh
cargo test --workspace --locked --offline
```

Measured on an Apple M3 Pro, rustc 1.96.0: **~0.55 s** of execution when nothing
needs rebuilding (about 0.3 s of it rustdoc compiling the doctests), **~3.8 s**
including the incremental rebuild after editing the core. No test sleeps and no test touches the network; every clock in the suite
is injected (LBT-008). If that budget starts drifting, find out why rather than
raising it.

## All gates, before you finish

```sh
cargo fmt --check
cargo test --workspace --locked --offline
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
cargo package -p sdax --locked --offline --allow-dirty
./scripts/check-architecture.sh
./scripts/compile-fail.sh
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --offline
```

The last one is not in CI yet; run it anyway. Every public item is documented,
so a broken intra-doc link is a real defect.

`cargo fmt` runs with rustfmt's defaults; there is no `rustfmt.toml`.

## Stage gates

| stage | delivers | gate before the next |
|---|---|---|
| 0 *(done)* | surface, validator, inspection, seam, host contracts, testkit clock/recorder/static checker, `TokioRuntime` | suite (a) W-*, suite (b) P-* |
| 1 | `engine::Machine`, `Plan::simulate`, the scripted driver, trace-level invariants | suite (c) C-* green on the scripted driver |
| 2 | `Plan::start`, `Running`, the tokio run driver, drop guard, drainer | suite (c) re-run on the adapter with paused time; suite (d) R-* |
| 3 | dynamic instances end to end | C-30, P-07 |

Do not start a stage's work before the previous stage's gate is green.

## House style

- Keep a file under ~500 lines. Split by responsibility, not by line count.
- Document every public item. A validate rule's rustdoc states its decision
  procedure; an invariant's docs name it by id.
- Findings and reports are **values** that name the rule, the node and a fix —
  never strings assembled at the call site.
