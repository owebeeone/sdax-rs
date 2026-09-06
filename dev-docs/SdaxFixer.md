# SDAX public-readiness remediation plan

Date: 2026-09-06. Updated: 2026-09-07.
Status: **local remediation implemented and verified; external release evidence pending**.

P0–P6's local work is complete. The integrated tree passes 369 Rust
tests/doctests (two ignored), 38 Python regression tests, all original gates,
the external consumer checks and fresh archive checks. See
[the final TDD and verification log](SdaxFixer-TDD-Log.md) for evidence and the
remaining hosted/MSRV, anonymous Git access and first-publication adapter
verification prerequisites. Changes remain uncommitted and unpublished.

Baseline: `8147239ebed2c5a82dc622bdebc39eddfbe9a6de`. The recent root-input
change is `b72e20eff22a6c5facde629e68ae09342a1907c1`; the baseline also includes
the eighth guide scenario. This plan addresses all five findings from the
public-readiness review. It does not authorize a commit, tag, push, GitHub
Release, crates.io publication, or repository visibility change.

## 1. Objective and evidence

Make the advertised author surface work in ordinary consumer programs, make
the installation instructions reproducible outside this workspace, and ensure
the release workflow tests and publishes the requested immutable commit.

The baseline passed the eight gates in [AGENTS.md](../AGENTS.md): 332 test and
doctest executions passed, two tests were ignored, and the separate
compile-fail gate checked 15 error-code witnesses. GitHub CI, including the
Rust 1.75 library builds, passed at this commit. These results do not close
the findings: additional consumer probes reproduced the failures below.

| Finding | Observed failure | Work package | Completion evidence |
|---|---|---|---|
| F1 | A step needing a join or component fails with `DriverError("the body source has no body for this node")`; directly exporting a component can return `Ok` with no output | P1: structural-node values | Real bodies consume join and component values; root reports contain the expected exports |
| F2 | A child importing root input has a phantom `input` edge in inspection; a component declaring unit input is accepted without a supplier and fails during execution | P2a: input inspection; P2b: admission | Inspection and execution agree; invalid component inputs refuse before effects |
| F3 | Manual publishing accepts a tag but checks out the dispatch ref, with only a version-string comparison | P3: immutable release selection | Both test and publish jobs use the same resolved tag commit; wrong or missing tags refuse |
| F4 | Registry installation is advertised before publication; the guide also needs an undeclared direct Tokio dependency and `test-util` | P4: consumer onboarding | A fresh external project can install and run the exact guide with documented dependencies |
| F5 | The generated core archive omits both license files; CI omits four required local gates | P5a: archive contents; P5b: gate parity | Both distributable packages carry licenses; CI and release checks enforce the full bar |

The eight original consumer probes yielded two passes and six failures. Two
failures exercised the same component-export defect, so these are not six
independent defects. Reading root input inside a component **did work**;
exporting its result through the component did not. Do not misdiagnose that
failure as broken input delivery.

Neither Rust package existed on crates.io when checked during the review.
The Python reference files have recorded provenance in
[reference/SOURCE.md](reference/SOURCE.md); the already-public Python project
is not a dependency or a publication blocker for this work.

## 2. Execution rules and order

Follow [AGENTS.md](../AGENTS.md) and
[SdaxContract-v1.md](SdaxContract-v1.md). Preserve the std-only core, dependency
allowlists, exact dependency pins, Rust 1.75 compatibility, and the distinction
between the author and host surfaces. No new runtime dependency is planned.

Implement in this order: **P0 → P1 → P2a → P2b → P3 → P4 → P5a → P5b → P6**.
Each package ends with its focused checks. P5b integrates the checks introduced
by earlier packages; P6 runs the complete suite on the resulting tree.

For each behavior change, first write and run the failing test, then make the
smallest implementation change that passes it. Record the actual RED and
GREEN evidence in a new `dev-docs/SdaxFixer-TDD-Log.md`. Keep failure fixtures
and positive controls; do not weaken a checker or substitute a scripted
constant for actual typed dataflow. Tests use injected or paused clocks and
remain offline. Record changed files and any host-interface change per package.

The fixes restore the existing contract rather than redefine joins, components,
or root input. No semantics-tag change is expected. If implementation reveals
a necessary change of meaning, record that separately before changing the
normative contract; do not silently turn a defect into supported behavior.

## 3. P0 — preserve the reproductions

Port the standalone review probes into repository-owned regression tests.
The temporary review directory is not a durable dependency of this plan.
The following recipes are sufficient to reconstruct them:

| Regression | Minimal setup | Required result |
|---|---|---|
| Join consumer | `A` returns `()`; `Barrier = join(A)`; `After.needs(Barrier)` returns `7u32` | `After` executes, clean report, output `Some(7)` |
| Component export | Child step returns and exports `99u32`; parent exports the child's component key | Clean report, output `Some(99)` |
| Component consumer | Same child; parent step needs component key and adds one | Clean report, output `Some(100)` |
| Imported root input | Root declares `u32`; a child imports it, reads it and exports it; parent exports component | Starting with `42` yields `Some(42)` |
| Imported input inspection | Inspect that root and child | No dependency targets a nonexistent `input` node; input does not become a runnable node |
| Unsupplied component input | Child uses `Plan::with_input::<()>`, and a step needs `child.input()`; register it as a component | Typed refusal before any run body or resource acquisition |
| Direct input export, positive control | Root declares `u32`, has one runnable step, and exports its input key | Starting with `123` yields `Some(123)` |
| Actual input read, positive control | Child imports root input and records the received value independently of component export | Reads exactly the value supplied to that run |

Put runtime cases in focused integration modules under
`crates/sdax-tokio/tests/`; keep declaration and inspection tests in the core.
The unsupplied-input probe must assert refusal, replacing the exploratory
probe's looser question of whether an accepted plan eventually runs.

## 4. P1 — publish values for joins and components

Primary surfaces: `crates/sdax/src/builder.rs`, `host/bodies.rs`,
`host/engine/admit.rs`, and `crates/sdax-tokio/src/driver.rs`. Include the
scripted driver and every `BodySource` implementation if the host seam changes.

The contract gives a join `Arc<()>` and a component `Arc<Out>`. Neither has a
body. Currently their lifecycle readiness does not fill the slot that
`Deps::fetch` or the root export reader expects.

1. Record the typed component-output bridge while `O` is still known in the
   builder, following the existing typed-erasure approach used for imports.
   Copy the child's actual exported `Arc<O>` into the component's parent slot;
   do not move the child's value or manufacture an arbitrary default output.
2. Supply the join's unit value and an unexported unit component's value as
   part of their existing readiness/dataflow behavior. An explicitly exported
   unit value still follows the export path; absence of a declared export and
   a missing declared export are different cases.
3. Connect readiness to slot publication through a small internal host seam.
   Before implementing it, record its exact signature and ordering in the TDD
   log. A dependent's body construction, a readiness waiter, or final export
   must not observe readiness before its required value is available. Check
   same-batch dependent admission and multi-threaded execution explicitly.
   Use existing ordered driver processing if it can meet that requirement;
   otherwise add a narrowly scoped ordered engine effect. Observer callbacks
   must not own the operation, and disabling observation must not affect it.
4. Resolve both source and destination by declaration key **and instance**.
   Cover components within components and components within template instances.
   Avoid cross-run caching and holding one scope's mutex while recursively
   resolving another scope in the opposite lock order.
5. Preserve the no-body rule: never synthesize `Started`, `NodeOk`, or
   `NodeErr` for a join/component to make publication happen. Update custom
   sources explicitly if needed; do not add a default success that hides an
   unimplemented value transfer. A missing declared export must not become a
   fabricated clean result or disappear behind a weaker assertion.

Required regressions, beyond P0: chained joins; a consumed unit component with
no declared export; nested component exports; components in two simultaneous
instances receiving different inputs; two runs with different outputs; and a
failed/skipped child export that never starts its parent consumer. Assert
actual received values, not only `Ready` events. Retain cleanup ordering,
child-policy behavior, and instance containment checks.

**Exit:** P0's four runtime failures involving joins/component outputs pass,
the added boundary cases pass, and both driver conformance suites still pass.
Document any host obligation in contract §10 and its rustdoc, without adding
host details to the user guide.

## 5. P2 — close the input boundaries

### P2a. Preserve supplied-input provenance in inspection

Primary surface: `crates/sdax/src/view.rs` and core planner-view tests.

The root's direct input need is omitted correctly, but a child's import is
resolved back to the string/path `input`, which has no corresponding node.
Carry the fact that a key denotes already-supplied root input through import
resolution, including nested imports. Omit the corresponding wait edge without
discarding ordinary resource dependencies or hiding unresolved foreign keys.
Do not solve this by filtering every edge whose target happens to be absent.

Test a direct child import, nested components, and a template importing root
input. Verify `inspect`, `why`, rendered edges and release ordering together;
compare with execution and the independent invariant checker. Preserve the
existing representation of a template's **own** input and its instantiation
boundary. Keep a real imported-resource edge as a positive control and an
invalid import as a refusal control.

**Exit:** supplied root input creates no phantom waits through any of these
import paths, while real lifecycle dependencies remain visible and enforced.

### P2b. Refuse a component's unsupplied declared input

Primary surfaces: `builder.rs`, `host/engine/table.rs`, load-error tests, and
contract §7. Use the existing `EngineError::TemplateAsScope` load refusal.

A component call supplies no input. `In = ()` does not imply the declaration
has no input: `Plan::with_input::<()>` and `Plan::builder` are different shapes.
Do not silently seed unit input or extend the component API as part of this fix.

Extend admission to inspect component subtrees, including components inside a
template declaration, before any root effects. Reject any component scope that
declares input, even when the input is unused. Template-instance scopes remain
legal because `cx.spawn` supplies their input. Preserve root `with_input`
admission and the existing refusal for roots offered to no-input entry points.
Report the complete offending scope/input path.

Test immediate and nested component refusal, both input constructors
(`with_input` and `template`), a component within a template, and unused input.
Use a root acquisition counter to prove refusal precedes side effects.
Verify `try_start` returns the typed error, `start` produces a failed refusal
report, and simulation agrees. Retain valid no-input components and supplied
unit roots/instances as controls. Update the acknowledged residue in
`RootInput-Log.md` and current queued-work statements when the fix lands.

**Exit:** every scope with declared input either has a supplier or refuses
before execution; no case reaches the misleading missing-body runtime error.

## 6. P3 — bind publishing to one immutable commit

Primary surfaces: `.github/workflows/publish.yml`, `scripts/release.py`, and
`RELEASE.md`. Preserve the existing person-operated release process.

1. Resolve the requested release tag once. Accept the existing `vX.Y.Z`
   release syntax; validate it and look up **`refs/tags/<tag>`**, not a branch
   or arbitrary revision expression. Peel annotated and lightweight tags to
   the commit. A missing tag or version mismatch is a refusal.
2. Carry that commit SHA as a job output. Test and publish both check out the
   SHA, not the dispatch branch and not a fresh independently resolved tag.
   Check the package versions and internal `sdax` dependency version on that
   checkout. Recheck tag identity before publishing; refuse if it moved after
   resolution. A normal `release.published` event uses the same path.
3. Keep credential-bearing publication downstream of all checks, including
   MSRV. Values from event inputs travel through quoted environment variables
   or structured arguments; do not interpolate them as shell program text.
4. Keep publishing `sdax` before `sdax-tokio`. Distinguish a registry 404 from
   authentication, rate-limit, transport or server errors; only a confirmed
   absent version enters the publish branch. For an existing version, establish
   that it belongs to the expected release rather than calling any same-version
   artifact an idempotent success. Wait boundedly for the core version to become
   resolvable before publishing the adapter.

Verification uses temporary local Git repositories and a command-recording
publish stub, with no registry writes. Cover: dispatch branch newer than tag
but carrying the same version; unknown tag; malformed tag; version mismatch;
annotated/lightweight tags; tag movement between resolution and publication;
registry 404 versus 5xx/transport errors; and a valid retry. The wrong-ref
fixture must prove the tested and published source selection, not merely that
the YAML contains a `ref` key. Any static workflow wiring check is labelled a
regression guard, not an executed GitHub workflow.

**Exit:** no manual path tests or publishes a different tree from the resolved
tag commit; refusals make no publish call. Record hosted-run verification
separately when an authorized push makes it available.

## 7. P4 — make the guide work in a new consumer project

Primary surfaces: `README.md`, `docs/{README,QuickStart,Reference}.md`,
`crates/sdax-tokio/tests/guide/`, and a new external-consumer check script.

While the crates remain unpublished, document Git installation from
`https://github.com/owebeeone/sdax-rs`, selecting both packages from the same
revision. State that repository access is required while it is private.
Show the registry commands as a later, published-version option, or add them
when publication actually happens. Do not claim this plan publishes anything.

Give the quoted Quick Start test a complete dependency recipe: direct `tokio`
at the supported pinned version, with `rt`, `time`, and `test-util`. Explain
that `test-util` supports the example's paused test clock and belongs in the
consumer's dev-dependencies. Keep production runtime requirements distinct;
no Tokio macros feature or extra normal dependency is required by this fix.

Create a check that builds and runs the exact quoted test in a temporary Cargo
project **outside** the workspace. Give it only the documented dependencies;
it must not inherit this repository's dev-dependencies or feature unification.
For offline checks, substitute local paths solely for the two unpublished
packages. Preserve the documented versions/features for all registry
dependencies. Exercise Git-source selection separately using a temporary local
Git checkout or, after an authorized public push, the actual remote URL.
Label those two forms of evidence accurately.

Negative controls: removing direct Tokio must fail with unresolved `tokio`;
removing `test-util` must fail at `start_paused`; the complete recipe must run
and assert both per-run outputs. Retain whole-file guide quotations. Edit the
test first, then copy it into Markdown and run `check-guide-quotes.sh`.
Use repository-absolute HTTPS documentation links where the shared README is
also rendered on crates.io, so copying it into a package does not break links.

**Exit:** a user can follow the documented source and dependency recipe in a
fresh project; compilation and execution do not depend on workspace features.

## 8. P5 — package contents and automated gates

### P5a. Ship the license texts

Primary surfaces: both publishable crate directories/manifests, root license
files, and a new archive-content check.

Retain `MIT OR Apache-2.0`. Put both license texts inside each publishable
package root, using tracked copies checked byte-for-byte against the root
canonical files. Prefer explicit copies over symlinks for archive portability.
Verify actual archive members and contents, not only manifest metadata or
`cargo package` exit status. Cover the shared README's links as part of this
inspection. Keep `sdax-testkit` unpublished.

Package and verify `sdax` offline. Respect the first-release dependency order:
`sdax-tokio` cannot undergo ordinary registry-backed package verification until
that `sdax` version is available. Before publication, use an isolated temporary
packaging fixture with a local registry or staged dependency arrangement to
inspect the adapter artifact. Do not alter the real manifests/lockfile to make
this fixture pass, and do not label it a crates.io resolution check. Record the
remaining real adapter verification as a release prerequisite, then run it
after the core is available and before adapter publication.

Negative fixture: removing either license from a generated test archive fails
the content gate. Mismatched license contents also fail.

**Exit:** both intended distribution archives include the exact two license
texts; the initial adapter-verification sequencing is explicit and enforced.

### P5b. Enforce the complete verification bar

Primary surfaces: `.github/workflows/{ci,publish}.yml`, `scripts/release.py`,
check scripts, `AGENTS.md`, and `RELEASE.md`.

Add the four currently omitted gates: architecture, compile-fail error codes,
guide quotations, and warning-free rustdoc. Include the new consumer,
release-selection, and archive-content checks. Keep fmt, workspace tests,
clippy, core package verification, lockfile validation, and Rust 1.75 library
builds. Preserve offline local operation; CI may prepare dependency/toolchain
caches before offline script execution.

Use a shared check entry point or an explicit tested inventory so local, CI,
and release paths do not drift. Keep the existing `--no-test` option accurately
documented; it does not bypass non-test publication checks, and the publication
workflow always runs the full required bar. Validate release-version edits
before tagging as well as validating the pre-bump tree. Do not introduce a
second release mechanism or bypass the workspace's commit rules.

Add negative fixtures for the new gates where feasible. A gate without one
must be described as a regression guard. Remove stale documentation claiming
rustdoc is absent from CI only after the workflow has actually been updated.

**Exit:** CI and the publication workflow invoke every applicable gate, and
publishing depends on their success, including MSRV at the selected commit.

## 9. P6 — final verification and disposition

Run the repository's eight baseline gates after integration:

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

Also run the new external-consumer, release-selection and archive checks, and
their negative fixtures. The release path uses a clean tree and does not use
`--allow-dirty`. Record the exact resulting commit/tree state and commands;
do not carry forward the baseline's pass counts as evidence for changed code.
Run or obtain the hosted Rust 1.75 build result for the resulting source. If
that toolchain or hosted result is unavailable, report it as pending rather
than substitute a newer compiler's result.

Update this plan's disposition table and the TDD log with a test/result link
for every row. Resolve all introduced TODOs and reconcile `QueuedWork.md`,
the root-input residue, contract wording, and release instructions touched by
the fixes. This is not a mandate to rewrite historical reports or unrelated
deferred work such as exhaustive schedule enumeration.

| Finding | Final local disposition | Evidence / external prerequisite |
|---|---|---|
| F1 | Complete: join values and component exports consumed, with isolation and failure controls | [Value-publication log](SdaxFixer-Values-Log.md); all regressions pass in the final integrated suite |
| F2 | Complete: root-input inspection corrected; unsupplied component input refused before effects | [Input log](SdaxFixer-Input-Log.md); all regressions pass in the final integrated suite |
| F3 | Complete locally: immutable source selection, refusal/retry tests, dependency-ordered publication | [Final log](SdaxFixer-TDD-Log.md); changed-source hosted/MSRV results pending |
| F4 | Complete locally: exact guide succeeds outside the workspace via paths and pinned local Git | [Final log](SdaxFixer-TDD-Log.md); actual anonymous GitHub installation follows authorized public access |
| F5 | Complete locally: fresh licence archives checked, shared gates wired and tested | [Final log](SdaxFixer-TDD-Log.md); real adapter registry package/build remains enforced at publication time |

The remediation is complete when every local finding has its required
evidence, no regression remains, and any external release prerequisite is
clearly identified. Actual public visibility, tags, releases and registry
publication remain separate actions for the owner to request.
