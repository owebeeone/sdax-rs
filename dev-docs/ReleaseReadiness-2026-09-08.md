# Release readiness — 2026-09-08

The release-readiness work began at `709533856b31dba48265ef74c66c241451e5de2a`.
The runtime/Linux fixes landed in `169ab40`; this follow-up adds Windows
consumer-check portability fixes.
Version remains **0.1.0, unreleased**. This is the current status record;
older stage/fixer logs retain their historical checkpoint statements.

## Changes

- Linux architecture checks use a portable temporary-file template.
- CI and publication fetch with Cargo 1.75 before its offline library builds.
- An unbounded root rejects services without `stop_within` anywhere in its
  component/template declaration tree. A bounded child budget alone is not
  sufficient under T7a. Findings name each registration in depth-first order.
  Runtime shutdown timing is unchanged; this is an authoring-time rejection.
- Compile-fail witnesses link the library produced by the current build,
  rather than selecting an arbitrary dependency artifact from another compiler.
- Cleanup documentation and the contract explain the nested stop requirement.

## Test-first evidence

The Pi baseline failed the architecture check with GNU `mktemp`'s
"too few X's" error; its existing unexpected-dependency regression failed
for the same reason. The new workflow guard failed on the unqualified fetch.
The three workflow tests now include missing, wrong-toolchain and late-fetch
negative controls in both CI and publication workflows.

Before the validator change, `unbounded_shutdown` had **two failures and one
passing control**: the root accepted a child with an unbounded service stop,
and findings for a reused child only named the root's own service. Afterward,
all three tests pass. The matrix exercises all 30 component/template trees
through depth four, plus 60 accepted variants protected by either a root budget
or service stop bounds. Reused children are reported at both registrations.

After Rust 1.75 was built in the Pi checkout, the old compile-fail script chose
an incompatible artifact and failed all 15 witnesses with E0786. A new test
with an invalid stale rlib reproduced E0786 locally, then passed with the fix.
It also checks an explicit target directory whose path contains spaces.

## Verification

All eleven checks passed on macOS arm64, Debian 13 arm64 on the Raspberry Pi,
and native Windows 11 Pro x64 (10.0.26200), using Rust 1.96.0. Each ran
**372 Rust tests/doctests with zero failures and two ignored**, and
**42 Python tests**. Architecture,
15 compile-fail witnesses, eight guide quotations, rustdoc, five consumer
controls, and fresh core/staged-adapter archive checks all passed.

Both libraries also built on Rust 1.75 on the Pi and Windows. The 15 compile-fail
witnesses passed again under Rust 1.96 after those builds on both, confirming mixed-toolchain
artifacts no longer confuse the checker. The two ignored tests were not run.
The adapter's registry-backed build remains a publication prerequisite.

Logs are retained in the containing task's `artifacts/` directory:
`sdax-release-local-final-20260908/` and `sdax-release-pi-final-20260908/`.
The Pi retains its logs at `~/git/sdax-release-pi-clean-final-20260908/` and its
checkout at `~/git/sdax-rs-pi-20260908`. An initial transfer introduced macOS
AppleDouble files; these were removed before the successful final inventory.
No source workaround was introduced for that transfer artifact.

[Hosted CI for 169ab40](https://github.com/owebeeone/sdax-rs/actions/runs/34182642754)
passed both the full inventory and Rust 1.75 library builds. The Windows-follow-up
hosted result must be checked separately; native Windows evidence comes from
`dabeest`, not a GitHub Windows runner.

## Windows follow-up

Python 3.13.5 was installed per-user from the signed Python Software Foundation
installer, with pip. uv 0.12.10 created `~/git/sdax-windows-venv-20260908` against
that interpreter. The test clone is `~/git/sdax-rs-windows-20260908`. Checks use
native Windows MSVC Rust toolchains, with Git for Windows providing `sh` for the
shell gates. The task runner prepends the venv and Git shell directories, enables
Python UTF-8 mode, and provides a `python3.exe` alias to the venv launcher. Rust's
existing default toolchain was not changed.

The baseline passed all Rust tests and both MSRV builds. Two Python tests failed
because `/tmp/...` is not a drive-qualified absolute Windows path; they now use
real temporary-directory paths. All five executable consumer controls passed,
but their final `shutil.rmtree` failed on a read-only Git pack index. A new
read-only-file regression reproduced WinError 5 before the fix. Cleanup now uses
`TemporaryDirectory`, which handles those attributes while still reporting
cleanup failures. Another test confirms `--no-clean` retains the directory.

All three platforms passed the full inventory after these script-only changes.
Final follow-up logs are in the task's `artifacts/` directories:
`sdax-windows-final-20260908/`, `sdax-windows-fix-local-20260908/`, and
`sdax-windows-fix-pi-20260908/`. Windows retains its own logs and a rerun wrapper
at `~/git/sdax-windows-final-20260908.ps1`.

## Publication prerequisites

- Commit/push the reviewed patch and obtain green hosted CI on that commit.
- Configure `CARGO_REGISTRY_TOKEN` for the GitHub `crates-io` environment.
  Read-only inspection on 2026-09-08 found no environments and no repository
  secrets configured. No credential values were requested or read.
- Make repository access public if anonymous Git installation is intended,
  then verify installation from the actual GitHub URL. Local path and pinned
  local-Git consumer tests do not establish anonymous access.
- Cut the immutable `v0.1.0` release using `scripts/release.py` and the documented
  workflow. Publish core first; the adapter must pass a real registry-backed
  package/build before publication. Staged archive content checks do not replace it.

The owner accepted the implementation, hosted-CI and release-preparation plan.
This checkpoint is prepared for commit and push to obtain hosted evidence.
No release tag, publication or repository visibility change has been made.

## Remaining coverage limits

`QueuedWork.md` retains exhaustive schedule enumeration, panic-abort execution,
and the three deferred substrate probes. The historical fast-loop timing budget
is not a current performance measurement. These checks were not silently marked
complete by this release-readiness work.
