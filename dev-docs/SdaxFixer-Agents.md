# SDAX fixer: agent execution and integration

Date: 2026-09-06. Coordinator: current task. Implementation baseline:
`8147239ebed2c5a82dc622bdebc39eddfbe9a6de`.

Current status: **F1 and F2 implemented and locally verified** (369 test/doctest
executions pass). See sections 9–10 and their evidence logs; sections 7–8 preserve
the first Spark checkpoint and its then-current resume plan.

This is the execution companion to [SdaxFixer.md](SdaxFixer.md). That document
remains unchanged. Its five findings and acceptance criteria remain the scope;
this document specifies the workers, ownership, interface decisions, and
evidence needed to integrate their results.

## 1. Execution environment and authorization

The owner requested GPT-5.3-Codex-Spark agents, with more-capable architectural
coordination where needed. The agent launcher accepted the exact model id
`gpt-5.3-codex-spark`; do not silently substitute another model if a later call
fails or quota runs out. Report the limitation and leave a concrete handoff for
unfinished work. Architecture and final review remain coordinator work.

The implementation checkout is
`/Users/owebeeone/limbo/sdax-wz/sdax-rs`. The newly selected desktop directory
`/Users/owebeeone/Documents/ChatGPT/sdax-wz` currently contains only `.git` and
is not an implementation checkout. Do not relocate or materialize a second
workspace as an incidental part of this task.

Agents share the real checkout and use disjoint file ownership. They must not
reset, clean, stash, stage, commit, tag, push, publish, change visibility, or edit
GWZ configuration. Use workspace status through `gwz status`. The owner has
authorized implementation and local verification, not a release. No credentials
are needed for worker tests. Do not consume account reset credits.

## 2. Worker contract

Each worker reads its assignment below, the corresponding section of
`SdaxFixer.md`, and the exact code it will change. It owns only the listed files
and its own log. Request additional ownership from the coordinator before
editing another worker's files. Do not edit `SdaxFixer.md`, this document, the
normative contract, shared manifests, or shared test-module registries.

Use new standalone Cargo integration-test targets when possible so registering
a test does not require editing a shared module list. Existing code changes
must have a failing regression first. Record the real before/after commands and
results in the assignment's log. A mock or scripted body does not prove typed
value transfer: runtime regression tests must consume the actual values.

Run focused tests while other workers are active. Do not run whole-workspace
formatting, rewrite a lockfile, install dependencies, or run release scripts.
Formatting must be limited to owned Rust files. The coordinator runs global
formatting and the final full checks after workers have returned.

Return this handoff:

1. Exact files changed and the implemented behavior.
2. RED command and observed failure; GREEN commands and results.
3. Positive and negative controls covered.
4. Any unresolved acceptance criterion or requested interface change.
5. Whether another worker was active during verification, so the coordinator
   does not mistake a mixed working-tree result for an isolated result.

Do not describe implementation as complete if a required case remains failing.
If an architectural assumption fails, stop the dependent edit, report a minimal
counterexample, and keep working only on independent owned tests.

## 3. Assignments and exclusive ownership

| Assignment | Model | Owned implementation files | Owned evidence |
|---|---|---|---|
| ARCH | Coordinator model, read-only adviser | No files; recommends F1's precise runtime contract | Returns architectural recommendation to coordinator |
| VIEW | GPT-5.3-Codex-Spark | `crates/sdax/src/view.rs`; new `crates/sdax/tests/fixer_input_view.rs` | `dev-docs/SdaxFixer-View-Log.md` |
| INPUT | GPT-5.3-Codex-Spark | `crates/sdax/src/host/engine/table.rs`; new `crates/sdax/tests/fixer_input_admission.rs`; new `crates/sdax-tokio/tests/fixer_input_admission.rs` | `dev-docs/SdaxFixer-Input-Log.md` |
| DOCS | GPT-5.3-Codex-Spark | `README.md`; `docs/README.md`; `docs/QuickStart.md`; `docs/Reference.md`; new `scripts/check-consumer-guide.py` | `dev-docs/SdaxFixer-Docs-Log.md` |
| RELEASE | GPT-5.3-Codex-Spark | `.github/workflows/publish.yml`; `RELEASE.md`; new `scripts/release_selection.py`; new `scripts/test_release_selection.py` | `dev-docs/SdaxFixer-Release-Log.md` |
| PACKAGE | GPT-5.3-Codex-Spark | License copies under `crates/sdax/` and `crates/sdax-tokio/`; new `scripts/check-package-contents.py` | `dev-docs/SdaxFixer-Package-Log.md` |
| VALUES | Stronger model; planned, not launched | Engine protocol and state transitions, both drivers, typed bridge and custom-source implementations listed in section 6 | `dev-docs/SdaxFixer-Values-Log.md` |
| INTEGRATE | Coordinator | Shared manifests, contract/rustdoc consistency, CI, shared check entry point, `scripts/release.py`, and cross-package integration edits after workers return | This document's disposition and final check record |

ARCH is advisory and must not edit VALUES files. RELEASE owns `publish.yml`
and `RELEASE.md` until it returns; only then may INTEGRATE wire package and
consumer gates into them. PACKAGE does not edit crate manifests: tracked
license files at crate roots should be included by Cargo's normal packaging.
Any manifest adjustment is handed to INTEGRATE.

## 4. Dependency order

1. Launch ARCH and let it settle the unresolved F1 publication mechanism.
2. VIEW, INPUT, DOCS, RELEASE and PACKAGE may work concurrently: their file
   ownership is disjoint and their behavior changes do not depend on F1.
3. Record ARCH's selected signatures, order and failure behavior below before
   launching VALUES. VALUES does not get an unresolved choice between a new
   engine effect and existing driver processing.
4. Collect and inspect every handoff. Integrate one returned patch area at a
   time, preserving other agents' edits. Do not repair files while their owner
   is still editing them; send the owner a focused follow-up instead.
5. INTEGRATE wires all new checks into CI and release paths, reconciles docs,
   then runs the combined baseline and new checks. Run an independent review
   of value publication and immutable release selection before disposition.

Parallel worker starts supersede the purely sequential package order in the
remediation plan for implementation scheduling only. They do not relax any
acceptance test, architectural invariant, or release prerequisite.

## 5. Assignment acceptance details

### VIEW — F2 inspection

Represent supplied root input distinctly in the resolver and propagate that
fact through imports. Do not remove arbitrary unresolved edges. Test direct
and nested component imports, a template importing root input, ordinary
resource imports, `why`, release ordering, and existing template-input display.
The tests must show that all displayed lifecycle edges have valid targets and
that root input does not introduce a wait. Preserve valid template-boundary
edges rather than flattening away every input-related relationship.

### INPUT — F2 admission

Keep `EngineError::TemplateAsScope`. A component with a declared input has no
supplier, even when `In = ()` or its input is unused. Traverse component
declarations inside template plans too, while accepting the template's own
supplied input. Refuse before any root effect. Cover nested scope paths, the
two input constructors, `try_start`, refused `start` reports and simulation.
Keep valid unit roots, valid instances and ordinary components as controls.
Do not introduce a new public component-input API.

### DOCS — F4 onboarding

The initial read-only Spark recommendation invented a `v0.1.0` install tag;
that tag does not exist. **Do not use it.** Both package dependencies must use
the same actual Git repository source; let Cargo lock the common resolved
revision, or use a verified existing revision. Clearly distinguish the private
repository's access requirement and later registry publication.

Document direct Tokio dev-dependency `=1.53.1`, default features disabled,
features `rt`, `time`, `test-util`. Preserve the exact guide source. The new
script runs its complete `simple_hold.rs` as a test in a temporary project
outside the workspace, substituting local paths only for unpublished SDAX
packages. Removing Tokio and removing `test-util` are required negative
controls. README links must also work when the README is copied into a crate.

### RELEASE — F3 source identity

Resolve an actual `refs/tags/vX.Y.Z` once and peel to a commit. Test and publish
must use that same SHA, including manual dispatch; same version on a newer
branch is not sufficient. Refuse missing/invalid tags, mismatched versions and
tag movement. Keep publication downstream of full checks and MSRV. Verify
selection with temporary Git fixtures and command-recording stubs, never with
real registry writes. Preserve environment-based secrets and quoted inputs.

Address registry-error discrimination and existing-version provenance within
the same helper where feasible. If real registry-dependent verification cannot
be executed offline, provide an explicit release prerequisite and a negative
fixture; do not claim an online result. Coordinate any new helper name or
pipeline boundary with INTEGRATE rather than duplicating publication logic.

### PACKAGE — F5 distribution files

Copy both root license texts byte-for-byte to both publishable crate roots.
Check archive member contents and mismatches, including negative archives with
a missing license. Verify the real core package offline. Investigate an
isolated offline staging fixture for the adapter's not-yet-published core
dependency; do not modify the real source manifests or publish either crate.
Return a precise distinction between real package evidence and staged evidence,
and the remaining real adapter verification required during first publication.

### VALUES — F1 runtime

Await the coordinator's frozen contract. Required evidence includes a consumed
join, a component exported by its parent, a parent body consuming a component,
an unexported unit component, nested component exports, components within
separate template instances, failed/skipped exports, and two concurrent runs
with different values. No synthetic body event for structural nodes, no
observer-dependent correctness, no fabricated missing output, no shared slots
across runs or instances. Retain normal cleanup and child-policy semantics.

## 6. F1 architecture decision — stronger-model implementation

ARCH completed a read-only review. The following design was implemented in the
F1 continuation described in section 9. Use one stronger-model coordinator for
this change: it crosses admission, faults, cancellation,
cleanup, instance resolution, and both drivers. Spark can later add bounded
regression cases against the implemented protocol. This task does not silently
switch an interrupted Spark worker to another model.

### Selected protocol

Publishing inside an existing `Emit(Ready)` handler is insufficient when transfer
can fail: the engine may already have admitted downstream `Spawn` effects in that
batch. Do not cancel and discard those effects; the engine already accounts for
their tasks. Use an explicit publication request and acknowledgement instead:

```rust
// Required BodySource method: no default success implementation.
fn publish_ready(
    &self,
    node: RawKey,                     // declaration key
    instance: Option<InstanceId>,
) -> Result<(), Error>;

// Engine Effect: run key.
PublishReady { node: RawKey }

// Engine Event: not a body completion event; run key.
ReadyPublished { node: RawKey, result: Result<(), Error> }
```

These additions are now implemented in the host enums/trait. Contract section 10
and host rustdoc record the obligation. Preserve author semantics and the no-body rule for structural nodes.

### Typed value bridge

Add a sparse `ready_values: Vec<(RawKey, ReadyValue)>` to `Build` and `Bodies`:

```rust
pub(crate) enum ReadyValue {
    Unit,
    Export { source: RawKey, read: ErasedExport },
}
```

Reuse `ErasedExport` and `Slots::set_erased`; `Slots` needs no new storage shape.
`join` records `Unit`. Constrain the existing component registration method to
`component<O: Send + Sync + 'static>(...) -> Key<O>` so its erased reader can
clone the actual `Arc<O>` from the child. Inspect `plan.ir.export`:

- `None`: record `Unit`, for a no-export unit component.
- `Some(source)`: record an export reader calling `slots.get::<O>(source)` and
  boxing the cloned `Arc<O>`. An explicitly exported `()` takes this path too.

A missing declared unit export must fail; it never gets the no-export fallback.

Add `PlanBodies::find_exact(plan: u64, instance: Option<InstanceId>) -> Option<Found>`.
`None` resolves static scopes only; `Some(id)` searches only that instance's
scopes. Keep outward `find` for imports. Publication uses exact lookup for both
destination and source, never an ancestor instance fallback.

Resolve scope handles before taking slot locks. Refresh the export scope's
imports before reading. Lock the source, clone its export, unlock it, then lock
the destination and store it. Never hold child and parent locks together for the
upward transfer: imports copy in the opposite direction. A join or no-export
component installs a fresh per-run `Arc<()>`.

Return a concrete error recording declaration key, instance and optional source
key for unavailable destination, absent structural entry, or missing/wrongly typed
export. Do not panic or fabricate a clean output.

Fix `Machine::instance_nodes` in `host/engine/instances.rs`: enumerate every table
node whose `node.instance == Some(id)`, in table order. Its current direct-root
scope enumeration misses component descendants used by the driver's origins,
kinds and paths maps. Exclude separately spawned nested instances.

### State and cancellation rules

Add `St::Publishing`, public host `NodeState::Publishing`, and
`Slot::publication_pending: bool`, initially false. The flag survives lifecycle
terminalization until an already-issued host obligation is acknowledged.

A join with ready needs releases its grants, enters `Publishing`, sets the flag,
and emits `PublishReady`. It emits no body start or completion. A component with
a steady inner scope retains its existing `Start(Prepare)` observation, then
enters `Publishing` and requests transfer. Neither emits `Ready` yet.

`ReadyPublished` must name a join/component with an outstanding publication.
Unknown keys, wrong kinds, unsolicited and duplicate acknowledgements produce a
typed `Reject` without mutation. For a valid acknowledgement:

1. Clear the pending flag.
2. If cancellation or another fault already terminalized the node, keep its
   terminal state and run normal settlement/cleanup; never resurrect `Ready`.
3. On success while still eligible, enter `Ready`, emit its observation, then
   admit dependents and reconsider scope steady state.
4. On failure, mark the structural node failed, retain the supplied error in the
   run's faults, and apply its containing scope policy. Use `Phase::Run` for a
   join and `Phase::Prepare` for a component. Settle the component's inner scope
   as existing component failure does. Component attempt faults do not belong
   in `Slot::faults`; do not synthesize `NodeErr`.

Update every state-dependent decision explicitly:

- `unsettled` includes `Publishing`; `needs_ready` remains `Ready | Finished`.
- `inner_fault_reaches` treats publishing exports as potentially arriving;
  component-fault detection and terminalization recognize the new state.
- Pending publication blocks release gates, run end and instance closure even
  after cancellation/fault has changed the lifecycle state to terminal.
- Cancelling a publishing component ends its attempt as `Interrupted` and
  settles the child scope. Cancelling a publishing join marks it skipped.
- A publishing join owns no task: cancellation emits no `Abort`, `Signal`,
  `TaskJoined`, or fabricated start for it.
- An earlier failed/skipped declared child export prevents a publication request.
  An unrelated isolated child fault may still permit its viable export to be
  published; retain that fault in the report.

The separate pending flag handles two publications in the same batch: the first
failure may terminalize the second node before its acknowledgement is processed.
Cleanup still owes that second acknowledgement.

### Driver ordering

The Tokio driver resolves run key to declaration key and instance using the
complete origins map, synchronously calls `publish_ready`, and appends its result
to a local FIFO. Perform each effects batch completely before feeding queued
acknowledgements back into the machine. Perform each acknowledgement's effects
completely and append any resulting publication acknowledgements to that FIFO.
Drain it before external/body/timer input and before updating Control, spawn-table
or `Child::ready` snapshots.

Do not process acknowledgement effects recursively ahead of the rest of their
original batch: a resulting abort could precede an independent spawn that the
engine has already issued. Value availability must precede dependency fetch,
user future construction, Ready observations, readiness waiters and final export
reads. Observation flags must have no influence on transfer.

Use the same immediate acknowledgement FIFO in the pure simulator before normal
script events. Simulation demonstrates lifecycle only, not typed values.
`ScriptedBodies` explicitly validates structural declarations and acknowledges
its lifecycle-only model; it must not manufacture typed output. Update the
`BlockingCleanup` source in `review_edges.rs` to delegate the new method. There
are currently two such alternative BodySource implementations. `running.rs`
requires no normal output-path workaround.

### Implementation sequence and ownership

VALUES owns these related files as one implementation unit:

- `crates/sdax/src/host/engine.rs` and
  `host/engine/{state,machine,admit,faults,settle,cleanup,instances}.rs`;
- `crates/sdax/src/builder.rs`, `host/bodies.rs`, required host re-exports;
- `crates/sdax-tokio/src/driver.rs` and custom-source adapters;
- `crates/sdax/src/sim/{simulator,effects}.rs`;
- focused new runtime/engine regressions and contract section 10 documentation.

Reserve `host/engine/table.rs` for INPUT; F1 does not require a table-layout change.
If other files prove necessary, record ownership before editing. Do not overlap
VALUES with a worker editing the same custom sources or contract text.

1. Port the P0 real-body failures and capture RED. Add engine-level publication,
   duplicate-acknowledgement and cancellation fixtures.
2. Implement typed bridge, protocol and both drivers together under the selected
   ordering. Keep the engine/std-only dependency boundary unchanged.
3. Verify focused actual-value tests and both conformance drivers, then add the
   failure-injection/instance-isolation cases below.
4. Have a separate read-only reviewer trace same-batch admission, cancellation
   with an owed acknowledgement, slot lock order and nested-instance origins.
5. Integrate through the full gates only when all required cases pass. If the
   design is contradicted by a minimal counterexample, revise this section before
   continuing rather than hiding the failure behind a default success.

### Required regression matrix

| Case | Observable result |
|---|---|
| P0 join consumer; chained joins | Actual unit values fetched; final output 7 |
| Component exported by root | Actual output 99 |
| Parent consuming child component | Actual output 100 |
| Root input imported and exported through component | Input 42 yields 42 |
| No-export unit component | Consumer receives `Arc<()>` |
| Explicit unit export | Reads exported slot; missing injected slot faults |
| Nested component exports | Value crosses every boundary |
| Component descendants in two concurrent instances | Inputs 11 and 22 retain their distinct values |
| Nested template instance containing a component | Innermost instance and ancestor imports resolve correctly |
| Two runs of one plan | No slot retention or value sharing between runs |
| Failed or skipped declared child export | No component Ready; parent constructor never runs |
| Unrelated isolated child fault with viable export | Actual export delivered and fault retained |
| Deliberately dropped exported slot | Structural fault, no consumer construction, acquired resources cleaned up |
| Custom source fails join publication | No fake body events; dependent absent; cleanup completes |
| Same-batch publication and dependent construction | Instrumented publication occurs strictly before construction |
| Resident run and child readiness | Both waiters observe completed publication |
| Multithreaded run with observation disabled | Same ordering and values, without sleep-based assertions |
| Cancel/fault with another publication outstanding | No resurrection or premature End/instance closure |
| Wrong, unsolicited or duplicate acknowledgement | Typed rejection without mutation |
| Existing scripted conformance | Lifecycle, policy, cleanup and containment remain correct |

## 7. First dispatch and checkpoint (historical)

All five bounded workers were invoked using `gpt-5.3-codex-spark` and fresh task
context (`fork_turns: "none"`). ARCH used the inherited coordinator model and
returned a read-only design. Spark then reported its usage limit; the tool's
reset estimate was **10:26 PM**. That is a reported estimate, not a scheduled
retry. No reset credit was consumed and no worker was silently switched models.

| Assignment | Actual state | Retained result / next action |
|---|---|---|
| ARCH / `runtime_architecture` | Complete, read-only | Selected contract in section 6; no implementation or test claim |
| VIEW / `spark_input_view` | Interrupted by Spark limit | Partial tests saved as a patch; source unchanged; resume F2a |
| INPUT / `spark_input_admission` | Interrupted by Spark limit | No implementation handed off; resume F2b from section 5 |
| DOCS / `spark_onboarding` | Worker returned; coordinator corrected and verified | Source installation and direct Tokio recipe, external consumer gate |
| RELEASE / `spark_release` | Interrupted; draft rejected for integration | Patch retained with specific defects below; stronger-model completion/review recommended |
| PACKAGE / `spark_package` | Interrupted; license copies retained | Core archive checked; draft checker deferred; actual adapter gate remains open |
| VALUES | Planned, not launched | Stronger model needed for section 6 |
| INTEGRATE | Partial draft deferred | CI/release entry-point changes preserved as a patch until prerequisite helpers are valid |

The active source retains the documentation, corrected consumer script and four
license copies. All eight baseline gates and the external-consumer check pass;
332 Rust test/doctest executions passed, with two ignored. See
[integration evidence](SdaxFixer-Integration-Log.md) for commands and limitations. Runtime, CI, publish workflow and release helper remain at their
baseline. This is a reviewed checkpoint, **not closure of all five findings**.

The original `SdaxFixer.md` stays byte-for-byte unchanged, including its original
planned status. This companion is the current execution record. Its dispatch
SHA-256 was `3b2192c453a55492d254dcb3bd10a89101ce1dfc3e30f149953bb1be65b307be`.

### Deferred patches and review notes

The files under [fixer-handoffs/](fixer-handoffs/README.md) preserve interrupted
work rather than placing unfinished tests or publishing logic on an active path.
Each patch passed `git apply --check` against the checkpoint when preserved;
that checks applicability only, not behavior. Review and repair before applying
on a later tree. Do not apply all patches and infer completion from a green
baseline suite.

- **VIEW:** `input-view.partial.patch` contains tests only, no resolver fix.
  Confirm that its API usage compiles and that template boundary expectations
  match current semantics. Add invalid-import and executable-value controls.
- **RELEASE:** `release.partial.patch` resolves the tag SHA but reads versions
  from the working checkout and leaves the test job on its initial branch.
  Its workflow CLI arguments also disagree with the helper's parser. Correct
  source selection before testing, align and exercise the actual workflow command
  shapes, validate real registry response shapes, and establish artifact commit
  provenance for both packages. The draft dependency-name filter can accept an
  unrelated dependency when one name field is absent. Matching a dependency
  version is not sufficient release provenance. Add bounded core-index readiness
  waiting and refuse registry uncertainty. Fix the broken shell fixture (an
  empty stray output file was removed); do not run real publish commands.
- **PACKAGE:** `package-gate.partial.patch` fabricates an adapter tarball in the
  real package output path, omits Cargo's packaging behavior and can later accept
  that archive as real. Replace this with fresh Cargo-produced evidence and a
  clearly separate staged fixture, never a success fallback to an arbitrary
  existing archive. Check archive licenses/README bytes with missing and altered
  license controls. Actual adapter verification against the registry remains a
  first-release prerequisite; content-only fixtures do not establish it.
- **INTEGRATE:** `integration.partial.patch` contains the coordinator's shared
  checker, CI/release wiring and unit tests. Four focused mocked tests passed,
  but the combined gate was not accepted because its worker helpers are
  incomplete. Forward `--allow-dirty` into the package gate, test post-bump version
  behavior, and validate failure propagation before wiring CI or publishing.
  Keep current library-only Rust 1.75 checks as the MSRV evidence; do not replace
  them with a dev-dependency test suite accidentally.

Worker logs are historical reports. Where they use future-tense GREEN claims,
list intended fixtures, or assert unverified behavior, the coordinator disposition
here and in `SdaxFixer-Integration-Log.md` takes precedence. No passing release or
adapter gate is claimed.

## 8. Resume plan from the first checkpoint (historical)

Resume manually after Spark quota is available; no automatic retry is installed.
To invoke a Spark worker, use the exact accepted model id and bounded ownership:

```json
{
  "task_name": "spark_input_admission_resume",
  "model": "gpt-5.3-codex-spark",
  "fork_turns": "none",
  "message": "Work in /Users/owebeeone/limbo/sdax-wz/sdax-rs. Read dev-docs/SdaxFixer-Agents.md and the P2b section of dev-docs/SdaxFixer.md. Implement only INPUT's assigned files, with real RED/GREEN evidence and the required controls. Do not edit another worker's files, the original plan, or release anything. Return changed files, command results and unresolved criteria."
}
```

Use the collaboration agent launcher for subtasks; creating user-owned sidebar
tasks is unnecessary. The invocation above is a template, not a claim that a
resume has already been dispatched. Supply each worker its specific acceptance
section and ownership rather than relying on conversation history.

Recommended next run:

1. A stronger-model VALUES owner implements section 6; a separate read-only
   reviewer checks its runtime ordering and instance isolation.
2. Spark VIEW and INPUT can run concurrently in their disjoint files. A Spark
   PACKAGE worker may replace the checker under the strict evidence distinction
   above. DOCS needs no further implementation unless another fix changes its
   public recipe.
3. A stronger-model RELEASE owner completes F3, using local Git/registry fixtures
   and explicit command-recording stubs. This recommendation is based on the
   observed draft defects and release consequence, not a general Spark limit.
4. INTEGRATE takes ownership only after worker handoffs, completes F5 gate parity,
   runs all eight original gates plus consumer/archive/release checks, and reruns
   changed-source MSRV builds where a Rust 1.75 toolchain is available.
5. Reassess public readiness against all five original acceptance criteria. Local
   path-based guide verification does not prove unauthenticated Git installation;
   verify that after an authorized visibility change. Actual first-release adapter
   registry verification and hosted workflow execution are reported separately.

No commit, tag, push, publish, repository visibility change or credit reset has
been performed. Those actions are not implicit in resuming a worker.

## 9. F1 continuation — implementation checkpoint

The owner accepted the recommendation to fix F1 with a stronger model. The
coordinator implemented the engine protocol and owned integration. Three bounded
workers used the inherited stronger model for the typed bridge, both drivers,
and real-value regressions; a fourth performed read-only engine review. Spark
was not retried and no credit reset was consumed. Disjoint worker ownership was
recorded at dispatch; all implementation ownership has now returned.

Implemented: required structural publication, typed Arc transfer, exact instance
lookup, complete instance-node mapping, FIFO acknowledgements in both drivers,
and pending-transfer accounting across cancellation/fault/budget expiry. Host
contract section 10 records the interface change. Real bodies verify outputs and
isolation; scripted conformance remains a lifecycle check.

The engine reviewer found no confirmed blocker. Additional tests cover publishing
components and component descendants cancelled before acknowledgement, budget
expiry before acknowledgement, and injected publication errors. Driver tests
cover a failed first publication in a batch containing another publication and
an independent spawn, plus missing declared u32/unit exports and required cleanup.

Final validation and exact counts are in
[SdaxFixer-Values-Log.md](SdaxFixer-Values-Log.md). The original `SdaxFixer.md`
remains unchanged; this companion records current implementation status.

Remaining work is **F2a/F2b, F3, and F5 checker/CI-release parity and actual adapter
package verification**. The earlier onboarding and license checkpoint is retained.
The deferred patches in section 7 remain drafts, not integrated fixes. No commit,
tag, push, publication or visibility change is part of this continuation.

## 10. F2 continuation and local-model review

Both input fixes are implemented and all eight baseline gates plus the external
consumer check pass: **369 tests/doctests passed, two ignored**. See
[SdaxFixer-Input-Log.md](SdaxFixer-Input-Log.md) for RED/GREEN evidence, the
sixteen-case admission matrix and positive controls.

The coordinator implemented the fixes. The user's existing inference server was
used serially for two advisory reviews: `qwen3.8:27b` identified useful nested
input/template cases that became passing regressions; `gemma4:26b` exhausted its
response budget without a final answer and is not counted as a completed review.
No model installation, server reconfiguration, Spark request or credit reset
was performed. Review suggestions were validated locally before acceptance.

The old VIEW patch in `fixer-handoffs` has now been restored, corrected and
extended; it is historical and must not be reapplied. INPUT is implemented with
recursive preflight through all template declarations, before root effects.

Next work: **F3 immutable release selection**, then remaining **F5 archive
checker and CI/release gate parity**, followed by the all-findings public-readiness
review. The original remediation plan remains unchanged. No release actions
have been performed.
