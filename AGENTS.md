# Working rules for `sdax-rs`

Read this before changing anything here. `dev-docs/SdaxContract-v2.md` is the
normative contract; this file is how the work is done.

`docs/` is how to use the crate. `dev-docs/` is plans, the contract, stage
reports, TDD logs and reviews. Do not put status, INV-/T-/C- ids, host
engine types, or how the crate was verified into `docs/`. A `rust,guide:<name>`
fence in `docs/` is the entire file `crates/sdax-tokio/tests/guide/<name>.rs`
— do not trim it.

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

- What is not executed is not described as executed. Say so where it matters.
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

## No unsolicited cleanup

The owner prefers retaining workspaces, build outputs, temporary files and recovery copies over reclaiming disk space. Do not delete them unless the owner explicitly asks for cleanup. Report disk-space concerns instead. Use a new isolated directory for another run; leave previous directories available for review and recovery.

## Windows remote execution

Use the installed Git/MinGW Bash for remote scripts on `dabeest`. Send script text through SSH standard input to `C:/Progra~1/Git/bin/bash.exe --noprofile --norc -s`; do not embed scripts in nested PowerShell command strings. The user's instruction is to avoid PowerShell translation. Pass native Windows tool arguments as a Python `subprocess` argument list, so Bash/MSYS does not rewrite their paths or options. Fail on errors and validate literal destinations before mutations. Do not clear a workspace with an interpolated recursive-delete pipeline; use a fresh isolated directory instead.

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
to the same line. Both libraries were verified on Rust 1.75 on Debian arm64 and Windows x64 on
2026-09-08; see `dev-docs/ReleaseReadiness-2026-09-08.md`. The Mac still has no
1.75 toolchain. Reverify changed library code with 1.75 locally, on the Pi,
or in CI; do not substitute a newer compiler result.

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

Python 3.11+ and Cargo 1.96.0 are used for the verification tools. The libraries
retain their Rust 1.75 minimum. Local checks, CI and release preparation share
the inventory in `scripts/check_all.py`:

```sh
python3 -B scripts/check_all.py --allow-dirty
```

This runs the eight baseline gates below, lockfile validation, the Python
release/package regression tests, the external consumer checks (local paths
and pinned local Git), and fresh package-content checks. The core archive is
also built; the adapter archive uses Cargo's temporary staging registry for
content inspection. A real adapter registry package/build remains mandatory
in the publication workflow after the core is available.

The baseline commands remain available individually:

```sh
cargo fmt --check
cargo test --workspace --locked --offline
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
cargo package -p sdax --locked --offline --allow-dirty
./scripts/check-architecture.sh
./scripts/compile-fail.sh
./scripts/check-guide-quotes.sh
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --offline
```

Every public item is documented, so a broken intra-doc link is a real defect.
CI and publication run the complete shared inventory and separate Rust 1.75
library builds (`python3 -B scripts/check_all.py --msrv-only`). An unavailable
local 1.75 toolchain is reported as pending, never replaced by a newer result.

`check-guide-quotes.sh` is the one that keeps `docs/` honest: every
`rust,guide:<name>` fence must be the whole file
`crates/sdax-tokio/tests/guide/<name>.rs`, which
`cargo test -p sdax-tokio --test guide` runs. Edit the test, then copy it into
the fence — never the other way round.

`cargo fmt` runs with rustfmt's defaults; there is no `rustfmt.toml`.

## Stage gates

| stage | delivers | gate before the next |
|---|---|---|
| 0 *(done)* | surface, validator, inspection, seam, host contracts, testkit clock/recorder/static checker, `TokioRuntime` | suite (a) W-*, suite (b) P-* |
| 1 *(done)* | `engine::Machine`, `Plan::simulate`, the scripted driver, trace-level invariants | suite (c) C-* green on the scripted driver |
| 2 *(done)* | `PlanStart::start`, `Running`, the tokio run driver, drop guard, drainer | suite (c) re-run on the adapter with paused time; suite (d) R-* |
| 3 *(done)* | dynamic instances end to end: `cx.spawn`, `Child::{ready, stop, id}`, per-instance scopes and slot tables, INV-16 containment | suite (c) on both drivers, `C-30`/`C-51`/`C-64`/`C-65`/`C-14`/`I-34`; the instance-aware checker; a Monte Carlo walk over templates |

Do not start a stage's work before the previous stage's gate is green.

## House style

- Keep a file under ~500 lines. Split by responsibility, not by line count.
- Document every public item. A validate rule's rustdoc states its decision
  procedure; an invariant's docs name it by id.
- Findings and reports are **values** that name the rule, the node and a fix —
  never strings assembled at the call site.
