# Authoring remediation and parallel validation plan

Date: 8 September 2026. Baseline: `ce339a59eefcd136562b16f4f20035c29ef0c988`,
pushed to GitHub `main`. Status: implementation lanes started on 9 September under `~/limbo`; the three
initial supporting investigations were read-only. No new adoption or performance result is claimed by this plan.

This is the next execution plan under [the improvement plan](SdaxImprovementPlan-2026-09-08.md).
It preserves that plan's acceptance criteria. The immediately preceding result is
[graph compaction](GraphCompactionCheckpoint-2026-09-08.md).

## Outcome and evidence

Make resource-bearing composition, retry and complete failure reporting practical
to author using the compact public reference. Demonstrate that improvement on
fresh tasks within a fixed budget. In parallel, reduce measured plan-construction
waste and close the remaining runtime comparison gaps.

The original authoring matrix passed 1/10; the later exposed-task retest passed
2/3, with composition still failing. Neither establishes inexpensive-model
readiness. Sources:

- [Original evaluation and costs](../../artifacts/sdax-improvements-20260908/authoring-eval/Results.md)
- [Generated-source review](../../artifacts/sdax-improvements-20260908/authoring-eval/SourceReview.md)
- [Exposed-task retest](../../artifacts/sdax-improvements-20260908/authoring-post-doc-retest/Results.md)
- [Implementation and remaining limits](SdaxImprovementImplementation-2026-09-08.md)
- [Historical performance evidence](ImprovementPerformance.md)

The three parallel investigators found:

1. The failed composition program disconnected its parent resource, computed the
   wrong value and attempted handwritten retries by cloning a single-use
   acquisition context. Existing `port`, `bind` and `component` APIs can express
   the desired behavior; the compact reference lacks a complete resource-import
   example. No component API redesign is justified yet.
2. The old evaluator requires `Result<u32, String>`, while the reference says not
   to replace a report with a string. This is a real guidance mismatch; its causal
   contribution to model failures is unproven. Existing `Report::into_result`
   and `Display` should be the canonical boundary, subject to regression tests.
3. The component builder clones a freshly mounted child solely to retain access
   to its export. Removing that copy is a concrete allocation candidate, not a
   measured optimization. Other suspected costs require profiling first.

## Parallel ownership and integration

| Lane | Owned work | Deliverable | Dependency |
|---|---|---|---|
| A — authoring | Composition consumer tests, executable guide and compact reference | Correct resource composition and report-boundary guidance | Exposed failures only; no fresh evaluation fixtures |
| B — diagnostics | Report consumer regression tests; minimal formatter repair if a RED demonstrates loss | Complete textual failure boundary and preserved typed errors | Coordinate public guidance with A |
| C — independent evaluation | Fresh fixtures, independent assertions, negative controls, runner and protocol | Frozen evaluation package | Inference waits for integrated source/docs freeze |
| D — performance | Allocation probes, component-copy candidate, equivalent Tokio fixtures | Measured plan-build change and comparison evidence | Reserve exclusive host windows for measurements |
| Integration owner | File ownership, review, shared checks, immutable snapshots and final report | One verified candidate with explicit remaining failures | All relevant lanes |

Assign one writer per file. A owns guide/reference files; B owns report code and
diagnostic tests; D owns its isolated performance changes. C works in a fresh
retained artifact directory. If two lanes need one file, agree the handoff before
editing it. The owner authorized local lane member commits on 9 September. No pushes are authorized.

Source inspection, implementation and fixture preparation may overlap. Timing
must not overlap compilation, other benchmarks or inference on the same host.
Inference must not overlap performance measurements on its host. Keep a recorded
host reservation and release it when the run finishes.

### Local clone setup

Use `gwz-fixed local clone` for each implementation/evaluation lane. The family source
is `/Users/owebeeone/limbo/sdax-wz`, registered with member
`mem_sdax_rs` at `sdax-rs`. Before creating lanes, verify this member contains the intended baseline and
prepare its Rust build cache. Keep this source quiet throughout each copy; create
lanes sequentially before starting their writers. All new lanes must be under
`/Users/owebeeone/limbo`, using family names `authoring`, `diagnostics`,
`evaluation` and `performance` with default sibling destinations.

The earlier setup mistakenly used a workspace under `~/Documents`. Those copies
remain historical evidence, not the active lanes for this plan. Do not write there
or reuse them as implementation destinations. See
[the retained setup report](../../artifacts/session-notes/RemediationCloneSetup-2026-09-08.md) for the failed
first attempt, successful native copies and verified cache reuse.

Record gwz's native versus ordinary copy counts, verify member HEAD and inherited
planning changes, and retain every clone. Native copy-on-write shares initial
file storage; later writes still consume space. A copied Rust cache may invalidate
due to relocated paths. If a uv environment is present, verify interpreter paths
and script entry points after copying before using it.

Run workspace status/staging through `gwz-fixed`. Local lane member commits are now authorized; pushes are not. Keep lane changes isolated; copy only
reviewed, owned edits back for an uncommitted integration when no commit is
authorized. Once commits and merges are explicitly requested, use the family
integration workflow and account for both member and root history. Never dispose
of clones or build outputs without an explicit deletion request.

## A. Repair the canonical composition path

First preserve the original failed programs as exposed regression evidence.
Build a consumer test for the desired S2 behavior using current public APIs:

- Bind a parent resource through a formal port; pass ordinary typed input
  separately. Compute `base + rates`, with explicit completed output export.
- Normal completion releases `derived`, then `rates`, then `base`.
- A transient acquisition failure retries declaratively and succeeds; permanent
  failure makes exactly two attempts, releases upstream resources and fails.
- Derived cleanup failure still releases rates and base and remains in the report.
- Two mounts with different offsets retain independent bindings and results.

Run the desired assertions against the preserved failed program before repair
and retain its compile/behavior failure. If correctly authored current API code
already passes, label that coverage a regression guard and the change an
authoring repair; do not invent an engine defect or claim an engine RED.

Turn the passing consumer into a complete executable guide. The compact
reference should include resource port declaration, binding, input, retry,
output extraction and cleanup in one coherent example. Keep repeated scalar
mount coverage separately. Replace redundant prose/examples to respect the
context cap; do not simply append more prompt material.

Show exact `Key<T>`, body `Arc<T>`, `Held<T>` and `Plan<Output, Input>` boundaries.
Distinguish a tuple-valued key from a tuple of dependency keys. Preserve supported
existing-Arc and unsized acquisition forms. Do not weaken single-use acquisition
or suggest arbitrary resource handles hidden inside ordinary data can be detected.

Acceptance: all listed behavior assertions pass; the executable guide is quoted
in full and passes the guide checker; compact context fits the preregistered cap.
The old generated task must still be evaluated under its fixed repair limit.

## B. Preserve diagnostics at the consumer boundary

Add an external string-returning consumer with nested mounted paths, a nested
`Error::source` chain, body and cleanup errors, incomplete work and unresolved
operation identity. Use separate scenarios where these states cannot naturally
coexist. Assert each expected detail, rather than merely checking for `Err`.
Separately assert that the typed-report path preserves the original error and
supports downcasting.

Start with existing `Report::into_result` and report formatting. If source-chain
or other details disappear, execute and retain the failing test before the
smallest formatter fix. Check existing rendering behavior before adding any new
formatter or public API. Keep structured fields available.

Update guidance to retain `Report` internally and use
`map_err(|report| report.to_string())` at an explicitly required string boundary.
Include a complete output-extraction function instead of encouraging manual
field-by-field summaries. A textual rendering preserves useful diagnostics;
it cannot preserve typed error objects or downcasting.

Acceptance: required textual details survive the boundary; typed errors remain
accessible internally; guide and evaluator instructions agree. New evaluation
tasks must state their intended boundary explicitly.

## Optional API follow-up: identified-effect ordering

The exposed recovery programs repeatedly used `.identified_by(...).needs(...)`;
the current identified builder only exposes `perform`. Recovery eventually passed
after a documentation repair, so this is lower priority than A and B.

Decide before freezing the candidate whether to implement a narrowly scoped
`needs` forwarding method. If selected, first execute a compile RED for the new
ordering, then compare both orderings for dependency values, stable identity
through retry, resource retention through recovery and a single identity edge
when ordinary dependencies repeat that key. Add no unrelated aliases. If deferred,
record the decision and retain the documented canonical ordering.

## C. Freeze and run a bounded independent evaluation

All six previous scenarios are exposed. Preserve them as regressions; do not
rename or lightly paraphrase them and call them held-out. The evaluation owner
designs six genuinely new scenarios covering two composition variants, two
identity/recovery variants, resident recovery/stop, and combined body/cleanup
failure. Freeze task text and assertions before authoring changes finish. Do not
send solutions or task-specific fixture details to A/B before candidate freeze.

Use an evaluator-local known-correct implementation to validate each fixture.
Add task-specific negative controls that violate binding isolation, cleanup
order, identity stability, retry count, report detail or restart behavior as
applicable. An implementation that always returns an error is insufficient.
Retain both the passing control and each executed negative result.

The initial protocol budget is fixed as follows; changes must be documented
before any inference, never made to rescue a disappointing result:

| Track | Trials | Public context | Maximum calls |
|---|---|---|---|
| Exposed regression | Original s1 then s2 | Frozen comparable reproduction context | 6 |
| Fresh evaluation | Six new cases | Complete compact reference, at most 16,000 UTF-8 bytes | 18 |
| Context reduction | Two fresh cases selected before execution, repeated separately | Complete selected sections, at most 8,000 UTF-8 bytes | 6 |

For a matched Qwen/Gemma comparison: at most 30 generation requests per model,
60 total. Both models receive the same case matrix; report their costs and outcomes
separately. This replaces the original single-model 30-call proposal before inference. Each trial permits initial generation and
two repairs. Temperature 0, seed 42 and output cap 4,096 tokens, subject to verified
server support. Record the actual settings and model digest. The live server was verified on 9 September: Ollama `0.33.0-dabeest`, accessible
through the existing forward at `http://127.0.0.1:11434`, with `qwen3.8:27b` and
`gemma4:26b`. Both answered a tiny inference smoke test correctly. Disable thinking
for this bounded code-output comparison and record that setting explicitly. No
genuinely small-model claim follows from testing only that model.

Set the whole-request ceiling to 64,000 UTF-8 bytes, including instructions,
task, context, previous answer and diagnostics. Overflow is a recorded budget
failure; do not silently truncate compiler/test diagnostics. Byte caps are not
token budgets: also record actual prompt/output tokens, model load time, inference
time, wall time and repairs. Infrastructure failures and uncertain interrupted
requests consume a call slot and retain their own outcome label; no automatic
replacement calls beyond the 30-call-per-model ceiling.

Repairs receive the original task/context, latest answer and exact compiler/test
diagnostics. They receive no human hints or reference implementation. Do not tune
docs or APIs after seeing fresh answers and then report the same tasks as fresh.

Harness work, based on the retained `authoring-eval` and `authoring-post-doc-retest`
runners:

1. Parameterize cases and copy into a new retained directory; preserve prior runs.
2. Freeze source, context, task/assertion manifests, runner, protocol, model digest
   and lockfiles. Assert hashes before every inference call and build, and verify
   evaluator files before and after every attempt.
3. Replace mere clearance-file existence with validated hashes and recorded host
   clearance. Build fixture controls with `--locked --offline` before inference.
4. Persist per-attempt states and aggregate call accounting before requests.
   On interruption preserve the uncertain request and reconcile it before resume.
5. Report compile, behavior, diagnostic and structural-review outcomes separately.
   Review formal ports, single-use acquisition, stable handles and absence of
   handwritten lifecycle management; executable assertions alone do not cover all.

Acceptance: all canonical examples pass, original composition succeeds within its
repair bound, and all six fresh compact cases pass behavior, diagnostics and
structural review. Publish all denominators, failures and costs. Context-reduction
results are separate; failure there limits reduced-context claims. One deterministic
sample per task is bounded evidence, not a population success rate. Human
reviewability remains unvalidated without actual human review observations.

## D. Measure plan construction and close performance gaps

Freeze `ce339a5` as the before source and retain an isolated candidate. Measure
declaration construction, validation, table compilation, body layout and RAII alias
layout separately while preserving the unchanged end-to-end plan-build timer.
Start with repeated resource-bearing mounts and the existing chain/wide/sparse
scales. Historical construction regressions describe older revisions and are not
a substitute for this baseline.

First candidate: capture the copyable child export before moving the newly
mounted child into its `Arc`, removing the clone in `builder.rs::component`.
Execute an allocation regression witness before changing it; verify repeated and
nested mount isolation afterward. Demonstrate lower allocation traffic on the
target workload and unchanged lifecycle behavior. Profile duplicate-name pair
comparisons and reverse key lookup before considering wider structural changes.

Build a lifecycle-equivalent handwritten Tokio comparison. The current computation-
only hash comparator does not qualify. Begin with normal release, partial startup
failure, downstream cleanup failure while upstream cleanup continues, and
cancellation after acquisition. Freeze independent assertions for outputs,
exactly-once cleanup, dependency order, full failure information and zero remaining
tasks. Count task draining and disposal. Extend to resident recovery and dynamic
containment only after these controls pass.

Recheck the integrated candidate on Windows for dynamic allocation traffic and
on Pi for runtime performance. Investigate the one-node sparse p95 flag. Measure
peak live allocator bytes separately from retained bytes; do not equate either
with RSS. Precise wakeup accounting remains open until measured.

Record source/fixture hashes, locked dependencies, compiler/profile, worker count,
trace policy, scales, warmups and raw samples. Use before/after and reverse order
under exclusive host reservations. Keep generation, compilation, one-time plan
building and execution separate. Execution includes all per-run setup, dynamic
expansion, cleanup, reports and disposal. Investigate regressions under the original
performance gate; do not loosen thresholds after seeing measurements.

## Checkpoints and completion

Each numbered checkpoint below is a milestone. Its independently assigned steps
should target fewer than 500 changed lines per goal; split larger fixture/harness
work into reviewed steps without weakening the complete milestone acceptance gate.

1. **Preparation:** assign file ownership; preserve exposed failures; freeze fresh
   task/assertion manifests and protocol; capture performance baseline.
2. **Implementation:** A/B execute consumer regressions and repair guidance/code;
   D measures its isolated allocation candidate; C validates runner controls.
3. **Integration freeze:** review combined changes, decide optional API scope,
   run shared gates, then freeze source and public context. Compile fixture controls
   against that revision without revealing solutions to implementation authors.
4. **Independent runs:** execute the bounded model wave and host-reserved performance
   comparisons. Keep their results independent; success in one cannot excuse a
   failure in the other.
5. **Report:** record exact source hashes, RED/GREEN evidence, all checks, costs,
   before/after measurements and remaining failed gates. Update the original plan's
   continuation pointer. Do not declare the mission complete with failed adoption
   or unexplained material performance regressions.

For code changes, run the full shared `scripts/check_all.py --allow-dirty` inventory,
relevant native Windows/Pi checks and Rust 1.75 library builds. Preserve temporary
directories and all evidence under the workspace retention rules. Follow the
explicit Git/MinGW Bash stdin route on Windows; never use cleanup scripts.
Label unexecuted work as pending and post-fix-only tests as regression guards.

This plan does not introduce durable workflows, general actors, borrowed task
scopes, or a total-history memory bound. Full trace/history retention remains a
documented limitation. Local lane member commits are authorized; pushing remains a separate owner-requested action.
