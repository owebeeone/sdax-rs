# SDAX fixer — P3–P6 implementation and final verification

Date: 2026-09-07. **Local remediation complete; external release evidence pending.**
Baseline member HEAD remains `8147239ebed2c5a82dc622bdebc39eddfbe9a6de`.
Workspace HEAD remains `840faaf506c7bf5c85a1c3c5701928c48ca10ade`.
All product changes remain uncommitted. No project tag, push, release,
publication or visibility change was performed. Git commits, tags and pushes
mentioned below took place only in disposable local test repositories.

This continuation was implemented directly. Earlier F1/F2 runtime fixes and
F4 onboarding/licence changes were preserved. No runtime dependency, crate
manifest, lockfile, public API or Rust implementation changed in this continuation.
The Python helpers use the standard library and require Python 3.11+.

## P3 — release source and publication

`scripts/release_selection.py` resolves an exact tag ref once and records both
its peeled commit and tag object. Versions are read from that commit, not the
dispatch checkout. Full checks, minimum-version builds and publication all
receive the same SHA; local and remote identity are rechecked before writes.

`scripts/registry_release.py` treats only HTTP 404 as absence. Existing releases
require matching archive checksums, clean VCS provenance, identical packaged
source and the correct adapter core version. VCS metadata alone is not trusted.
`scripts/publish_release.py` preflights both registry names, publishes core
first, waits boundedly for its verified checksum in the registry index, and
requires a real adapter registry package/build before adapter publication.

RED: the selection and publication test modules initially failed to import the
missing implementations. Subsequent behavioral regression tests exposed the
original release helper pushing a commit object instead of an annotated tag
object, attempting a push after local tag movement, and permitting `gh release
create` to create a missing tag. All three failed before their corrections.

GREEN: local Git fixtures cover lightweight/annotated tags, malformed and
missing tags (including a branch masquerading as a tag), mismatched manifests
and dependency versions, dirty/wrong checkouts, tag replacement/deletion and
changed remote annotations. A newer branch with the same package version is
refused; a source-sensitive subprocess check and recorded publications both
use the selected release checkout. Registry fixtures cover 404 versus
401/403/429/5xx/transport errors, bad checksums/provenance/source/lockfiles,
yanked versions, retries, indexing timeout, package failure and tag movement
after core publication. Actual local bare-repository pushes preserve annotated
tag objects and are idempotent. No registry writes are used by these tests.

## P4 — installation source evidence

The existing consumer check now also builds the exact guide from a pinned
`file://` Git snapshot of the current source. It copies current files, including
uncommitted fixes, into a disposable repository, vendors dependencies offline,
and uses an isolated Cargo home. No GitHub or registry network access is needed.
Both SDAX packages must resolve to the same exact commit in the consumer lockfile.

RED: source-substitution tests failed because `build_git_manifest` was absent.
GREEN: source substitutions preserve every other documented manifest option;
omitted Tokio is not repaired. All five executable consumer controls pass:
local-path guide, missing direct Tokio, missing `test-util`, pinned local-Git
guide, and a missing Git revision. Actual anonymous GitHub access remains a
separate prerequisite after an authorized visibility change.

## P5 — archives and gate parity

`scripts/package_archives.py` uses fresh temporary target directories and never
falls back to an old output archive. It verifies exact licence and README
bytes, portable README links, original manifests, package metadata, source
presence and adapter dependency/lockfile versions. Core packaging includes
Cargo build verification. Cargo itself generates the staged adapter archive
by packaging both crates together with `--no-verify` in a separate target.
Neither real manifests nor a real package output directory is edited for staging.

RED: archive and shared-check tests initially failed because their helpers did
not exist. The workflow guard failed on the missing complete shared CI call.
The release-helper tests failed on missing dirty-flag forwarding, post-bump
gate handling and GWZ commit routing.

GREEN: missing/altered licences and README, incorrect package versions,
relative README links and stale archives after Cargo failure are rejected.
Every shared gate's failure stops subsequent execution. `--no-test` removes
only the workspace Cargo test invocation; package, consumer and other checks
remain. Workflow mutation controls reject a missing MSRV dependency, SHA
binding, shared-check call or identity check. Existing-gate negative fixtures
reject an unexpected normal dependency, drifted quotation, incorrect expected
compiler error code and broken rustdoc link. YAML also parses with Ruby Psych.
Workflow assertions remain static regression guards, not executed Actions.

An actual GWZ fixture found that a member-only commit left generated lock and
integrity files staged at the workspace root. The command test was tightened
and failed before the fix. Selecting the member and root in `gwz commit`
now preserves matching member pins and a clean workspace, verified by actual
GWZ commands in a disposable workspace. The real project was not committed.

### Adapter evidence boundary

The initial experiment attempted full multi-package Cargo verification. Cargo
1.96.0 generated both archives but failed the adapter build in its temporary
registry with `no hash listed for sdax v0.1.0`, an internal Cargo error. That
failed operation is not accepted as a passing package check. The final gate
explicitly requests content-only staging with `--no-verify`; ordinary
registry-backed adapter package/build verification is enforced later by the
publication executor after the core is available. It remains unexecuted here.

This follows Cargo's documented separation of archive assembly and build
verification. Cargo also documents that VCS metadata is not proof of source
provenance, which is why retries compare packaged source bytes as well.
References: [cargo package](https://doc.rust-lang.org/cargo/commands/cargo-package.html),
[registry index](https://doc.rust-lang.org/cargo/reference/registry-index.html).

## P6 — final results

`python3 -B scripts/check_all.py --allow-dirty --log-dir <temporary log directory>`
passed all eleven shared checks on the integrated worktree:

| Check | Result |
|---|---|
| Locked offline Cargo metadata | PASS |
| rustfmt | PASS |
| Workspace/all-targets Clippy, warnings denied | PASS |
| Workspace tests and doctests | **369 passed, 0 failed, 2 ignored** |
| Architecture rules | PASS |
| Compile-fail error codes | PASS, 15 witnesses |
| Guide quotations | PASS, 8 scenarios/fences |
| Locked offline rustdoc, warnings denied | PASS |
| Python release/package/gate regression tests | **38 passed** |
| External consumer checks | PASS, all 5 controls |
| Fresh core and staged adapter archive checks | PASS, with the distinction above |
| Whitespace diff check | PASS |
| Workflow YAML parsing | PASS |
| Actual GWZ helper fixture | PASS, clean root/member with matching pin |

The final GWZ selection correction only changed the release helper and its
test. All 38 Python tests were rerun successfully afterward; Rust, consumer
and archive inputs were unaffected. The follow-up is `script-tests-final.log`.

An isolated copy was bumped from 0.1.0 to 0.2.0 using the real version/lockfile
functions. All eleven checks passed there too: 369 Rust executions and the
then-current 35 Python tests. This verifies the changed-version packaging and
consumer path, rather than relying only on mocked post-bump commands. The
real manifests and lockfile stayed at 0.1.0.

Raw current-tree evidence:
`/var/folders/02/bn9c9g2x5qj8bb42zb857p7c0000gn/T/sdax-fixit-lehfko7p/final/`.
Post-bump evidence:
`/var/folders/02/bn9c9g2x5qj8bb42zb857p7c0000gn/T/sdax-version-gate-rbboczew/checks/`.
GWZ fixture evidence: `sdax-fixit-lehfko7p/gwz-helper.json` under that same
temporary parent. These command/result records remain the durable summary if
temporary logs are removed.

`source-snapshot.json` records SHA-256s for 163 files: crates, scripts,
workflows, consumer docs, manifests/lockfile, README, RELEASE, AGENTS and
gitignore. It excludes dev-docs to avoid a self-referential report digest.
The sorted `path + NUL + file SHA-256 + newline` inventory has SHA-256
`7ec50c86e5864bc14ab2cdfd1a4e7e4d9cbaa21de04a22c66a44109c364c7363`.

## Disposition and external prerequisites

F1 and F2 remain verified; see `SdaxFixer-Values-Log.md` and
`SdaxFixer-Input-Log.md`. F3, F4 local consumer evidence, F5 and P6's local
integration checks are complete. The original plan's disposition is updated.
Earlier interrupted patches are historical and were not applied wholesale.

Still external: changed-source Rust 1.75 and hosted workflow evidence,
anonymous GitHub installation after public access is enabled, and real
registry-backed adapter package/build verification during first publication.
`rustup run 1.75 rustc --version` confirmed 1.75 is not installed locally;
no new toolchain was installed and no newer result is substituted for it.
These limitations do not claim a release has run or publication is complete.
