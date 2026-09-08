# Releasing sdax-rs

Current checkpoint: [2026-09-08 readiness](dev-docs/ReleaseReadiness-2026-09-08.md).

Cut releases from `main` with `scripts/release.py`. Tags are immutable.
Creating the GitHub Release triggers the crates.io workflow. Manual workflow
retries accept an existing tag and use exactly the same verification path.

## Prerequisites

- Clean worktree on `main`; when this repository is a gwz member, the whole
  gwz workspace must be clean.
- Python 3.11+, Git, Cargo 1.96.0 with rustfmt/clippy, and cached dependencies.
- `gwz` for a workspace member; `gh` for `--github-release`.
- crates.io token in the GitHub `crates-io` environment as
  `CARGO_REGISTRY_TOKEN` (publication step only).
- Rust 1.75 library builds must pass in the publication workflow. The local
  check can also run when that toolchain is installed.

The helper uses `gwz add` and a `gwz commit` selecting this member and the root
when inside a GWZ workspace, so generated lock/integrity updates are committed
together with the release version. It uses Git directly for a standalone checkout. Workspace
metadata stays under GWZ's control; do not hand-edit pins or capture files.

## Cut a release

```sh
# Check, bump, check the changed version, commit and tag. Does not push.
python3 -B scripts/release.py vX.Y.Z

# Also push main + tag atomically.
python3 -B scripts/release.py vX.Y.Z --push

# Also create the GitHub Release, triggering crates.io publication.
python3 -B scripts/release.py vX.Y.Z --push --github-release
```

`--github-release` requires `--push`. Existing tags are accepted only when
already pointing at the release commit. The helper never moves them.
The full gate inventory runs before version edits and again on the bumped
manifests and refreshed lockfile before committing or tagging. A failed
post-bump check leaves the version edits available for correction.

## Shared verification

```sh
# Complete local checks on an edited worktree.
python3 -B scripts/check_all.py --allow-dirty

# Same checks on a clean checkout, as used by CI and release preparation.
python3 -B scripts/check_all.py

# Populate the cache with the same Cargo version used by the offline build.
cargo +1.75 fetch --locked

# Library-only minimum-version check; requires installed Rust 1.75.
python3 -B scripts/check_all.py --msrv-only
```

The shared inventory includes lockfile validation, fmt, clippy, workspace
unit/integration/doctests, architecture rules, compile-fail error codes, guide
quotations, warning-free rustdoc, Python release/package tests, fresh external
consumer projects, and package archives. `--log-dir PATH` retains each gate's
output and a JSON result record. CI prepares caches before running offline.

The external consumer check uses the documented dependency recipe verbatim,
substituting only the two unpublished SDAX source coordinates. It tests local
paths and a pinned `file://` Git snapshot of the current source. Git testing
uses an isolated Cargo home and vendored registry dependencies. It checks both
crates resolve to the same commit; omitted Tokio, omitted `test-util`, and a
missing Git revision are negative controls. This does not establish anonymous
access to the actual GitHub repository; verify that after making it public.

## Package evidence and first-publication order

Both crates carry the canonical MIT and Apache-2.0 licences and shared README.
The archive check always uses fresh temporary output directories, checks exact
contents and portable README links, and refuses any Cargo failure. An old
`target/package` file cannot satisfy it.

The core is packaged and built offline. Before the core exists in crates.io,
the adapter is packaged alongside it using Cargo's own temporary staging
registry with `--no-verify`. This verifies the intended adapter archive's
contents; it does not establish registry-backed build success. The installed
Cargo 1.96.0 fails staged adapter build verification with an internal missing
checksum error, so staged build success is not claimed.

At publication time the workflow:

1. Resolves `refs/tags/vX.Y.Z` once, recording both the commit SHA and tag object.
   It validates all crate/workspace versions and the adapter's core dependency
   against the selected commit, including annotated tags.
2. Checks out that SHA in both full-check and Rust 1.75 jobs. The credentialed
   publication job depends on both jobs and checks out the same SHA.
3. Rechecks the remote tag and clean checkout before publication operations.
   A missing or changed tag refuses; a dispatch branch with the same version
   is not accepted as the requested release source.
4. Looks up both registry versions. Only HTTP 404 means absent; authentication,
   rate-limit, transport, malformed-response and server errors stop the run.
   Existing versions must have a matching checksum, clean VCS provenance and
   packaged source identical to freshly packaged selected source. The adapter
   lockfile must select this release's core version. A version string or VCS
   metadata alone is insufficient.
5. Packages/builds and publishes `sdax` if absent. It waits for the matching
   archive checksum in the core registry index, with at most twelve attempts
   and five-second intervals (each network request also has a timeout).
6. Performs a real `cargo package -p sdax-tokio --locked --registry crates-io`
   build and archive check against that core before publishing the adapter.
   This step is mandatory on retries too. The harness is never published.

Publishing is serialized in GitHub Actions. Tests use temporary Git repositories,
synthetic registry responses and command-recording publication substitutes;
they perform no registry writes. Workflow text checks are static regression
guards with mutation controls, not evidence of a hosted Actions run.

## Skip the workspace test run only

```sh
python3 -B scripts/release.py vX.Y.Z --no-test
```

This skips only `cargo test --workspace` in local preparation. Every other
check still runs, including Python regression tests, consumer test execution,
compile-fail checks and archive verification. The publication workflow always
runs the full bar, including workspace tests and Rust 1.75 builds.
