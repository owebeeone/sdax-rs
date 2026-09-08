# sdax improvement plan

Latest continuation: [dynamic graph compaction](GraphCompactionCheckpoint-2026-09-08.md).
The earlier measurements and status statements below apply to their named revisions.

Date: 8 September 2026. Baseline: `bdea94200ae743fc94ea76987cfd4f6927e0ff8d`.

Status: implementation executed; available snapshot data has been restored after the agent-caused Windows deletion incident, with documented exceptions and a post-snapshot gap. See [implementation report](SdaxImprovementImplementation-2026-09-08.md) for completed checks, measurements and the failed low-cost-model adoption gate. The numbered sections below preserve the intended acceptance criteria rather than retroactively weakening them. API compatibility with previous private versions is not a constraint. The owner requires runtime performance to be important and plan generation to be excluded from the runtime performance tally.

## Intended result

Make sdax a dependable, inexpensive-to-author lifecycle API: acquired values cannot silently lose their cleanup obligations; compensation works for the outcome actually known; components accept explicit inputs and can be reused independently; service recovery has predictable handle semantics. Preserve the dependency-driven cleanup that outperformed handwritten control flow in the review.

Build and validate reusable execution structure once. Measure the engine with that structure already built, while still counting all work required for each run and dynamic instance. Maintain separate results for generation, compilation, plan building and execution. A fast generator is useful, but is not evidence of a fast engine; a slow generator must not inflate an engine latency figure.

The deliverable is a coherent revised API, not a compatibility layer containing both old and new spellings. Update the normative contract and semantics tag alongside behavioral changes. Keep the core std-only, the existing crate boundaries and the Rust 1.75 minimum unless a separate, evidenced design decision changes them.

## Evidence and scope

The [adversarial review](../../artifacts/sdax-adversarial-review-20260908/AdversarialReview.md), [README proposal](../../artifacts/sdax-adversarial-review-20260908/ProposedREADME.md) and [alternatives assessment](../../artifacts/sdax-adversarial-review-20260908/AlternativesInsights.md) are the input evidence. Their experiment artifacts are local to this workspace; implementation must promote the relevant witnesses into the repository so future contributors do not depend on those external paths.

The review established failures and authoring friction, not runtime throughput or latency. Its 13 final tests are current-behavior witnesses: four pass by asserting adverse behavior. Promote the original desired assertions and observe them fail before implementing fixes. Do not mistake the witness suite passing for the issues being resolved.

| Issue | Planned response | Completion evidence |
|---|---|---|
| Receipt-less compensation cannot invoke its handler | Explicit unknown-outcome recovery using identity recorded before the external action | Cancellation/timeout before receipt invokes recovery; normal compensation still receives a real receipt |
| Multiple `hold` calls overwrite an obligation | Single-use acquisition capability plus a defensive registration state machine | Second acquisition cannot execute through the capability; original value remains recoverable |
| Static components reject typed/unit input late | Explicit typed input binding, with unit handled consistently | Valid input-bearing mounts run; missing/wrong bindings fail during authoring/build |
| Reusing one child plan twice fails at launch | Separate declaration identity from mount and run identity | Two mounts and concurrent runs have distinct mutable state and correct imports |
| Restart creates a new handle without updating dependents | Publish a stable handle once; restart a serving episode explicitly | Existing long-lived dependents work across recovery; no implicit dependent replay |
| Missing terminal silently omits a declaration after a warning | Record unfinished declarations and refuse build | Dropped/forgotten resource/effect builders produce structured build findings |
| Model loses useful errors while converting a report | Lossless error conversion and concise actionable formatting | Nested cause, phase, path and cleanup/ambiguity details survive the standard path |
| Sparse examples and too many authoring spellings | One canonical authoring route, complete executable examples | Examples compile, behavior checks pass, and held-out inexpensive-model tasks improve |
| Exported resource survives logical release | Distinguish live resource keys from completed value outputs | Direct resource export is rejected; completed data exports remain straightforward |
| Inspection omits input provenance | Separate data-provenance and lifecycle views | Both show mount/input bindings without implying that inputs require cleanup |
| Dynamic children are resident lifecycle units, not typed finite task results | State that boundary clearly; retain containment tests | No unsupported fan-out/results claims; existing ownership behavior remains correct |

## 1. Establish contracts, witnesses and baselines first

Create an improvement-stage TDD log and copy the review's original failing expectations into focused core/adapter tests. Preserve the reviewed commit as the comparison baseline, without resetting the current checkout. Record toolchain, dependencies, build profile, CPU, OS, runtime configuration and benchmark fixture revision.

Before each behavior change, update the relevant contract decision and write its desired outcome. Existing clauses need reconciliation, particularly held-value obligations versus unknown-outcome recovery, component load validation versus build validation, and service readiness versus restart. Introduce the next semantics tag as part of the implemented revision; this planning document does not change `sdax/1`.

Capture a runtime baseline before optimizing. Also preserve the original local-model tasks as regression exercises, but use new held-out tasks to assess generalization after documentation changes. The already-exposed composition task alone cannot establish that the API became intuitive.

**Gate:** reproduced failures, explicit desired semantics, reproducible performance fixtures, and recorded baseline measurements. Unsupported baseline scenarios are reported as unsupported, never as zero-cost successes.

## 2. Make cleanup obligations impossible to overwrite

### Single-use acquisition

Recommended design: give each resource/effect attempt a non-cloneable acquisition capability. Consuming it starts exactly one acquisition or registers one already-owned, effect-free value. Ordinary cancellation/identity context can remain shareable, but cloning that context must not recreate acquisition authority. Retries receive a fresh capability only after the previous attempt is fully settled.

Retain the essential guarantee: the poll observing acquisition success registers the value and cleanup obligation before the continuation can run. Check/reserve the capability before invoking the acquisition factory, not merely after polling the returned future. A lazy factory prevents a rejected second call from executing synchronous work while constructing that future. This does not make arbitrary user code outside the factory safe.

Use a defensive state machine such as unused → reserved → held → discharged. A reservation whose future never completes must not erase another obligation. The adapter/host boundary must reject invalid repeated registration without replacing the stored value, even when callers bypass the ordinary typed surface. Host APIs and raw context access must not provide an accidental public escape from the typed guarantee.

Prefer separate nodes for separately acquired resources. Do not introduce an automatic bundle API unless partial bundle acquisition has a complete cleanup contract.

**Acceptance tests:** second use fails to compile on the author surface; concurrent/repeated host registration is rejected; rejected factories are never invoked; cancellation at every acquisition handoff releases a successful value exactly once; error/panic after registration preserves cleanup; a new retry cannot overlap the prior attempt or inherit its capability. Exercise `hold_value` as well as asynchronous acquisition. Retain the existing cleanup-error audit: one failed release must not suppress upstream releases.

**Cost constraint:** avoid a general vector of obligations on the normal single-resource path. Measure allocation and synchronization costs before selecting the defensive representation.

### Incomplete declarations

Reserve a declaration entry when a resource/effect builder begins, and mark it complete only at its terminal operation. `build` must reject every unfinished declaration, naming its node and missing terminal. Keep type-state enforcement and `must_use` as immediate feedback, but stop relying on a warning to prevent omission.

The record must survive `drop`, `let _ = ...` and `mem::forget`; a builder destructor alone is insufficient. Abandoning a declaration should require an explicit removal operation if there is a demonstrated need, not happen silently.

**Acceptance tests:** ordinary complete chains build; discarded acquire/perform chains fail build; multiple incomplete declarations are all reported in deterministic order; using an unfinished builder as a key still fails to compile. These bookkeeping costs belong to plan building, not each run.

## 3. Make unknown outcomes recoverable

Separate two operations in the public model:

- **Known success compensation:** receives the recorded receipt and undoes that successful effect.
- **Unknown-outcome recovery:** receives an operation identity and attempt context recorded before the external action; it can reconcile or undo an operation even though no success receipt exists.

Recommended direction: retain a simple receipt-based compensation handler and add an explicit unknown-outcome handler. Remove the policy-only choice that claims to compensate without a callable handler. An unknown outcome defaults to an explicit report/reconciliation obligation; automatic recovery requires the matching handler and a declared safety argument.

Operation identity must be typed per-run data, not shared captured mutable state. Define whether identity is stable across retries: the normal idempotent-operation case uses one logical operation identity across attempts and a separate attempt number for diagnostics. Do not fabricate an `Arc<Receipt>`, pass a magic default receipt, or infer external success from cancellation.

A recovery handler must distinguish resolved from still-unknown outcomes. Only successful resolution discharges the unresolved obligation. Preserve the original uncertainty in trace/history; failure or timeout leaves the operation visibly unresolved and includes its recovery failure. Define how cancellation outcome and recovered cleanup coexist in `Report` and its result conversion.

Keep recovery within cleanup shielding, dependency lifetimes and shutdown budgets. The engine must settle/join the interrupted body before beginning recovery; unabortable work must be reported honestly rather than raced with a compensator. No process-durability or exactly-once guarantee is introduced.

**Acceptance tests:** cancellation and timeout before acknowledgment; external effect occurred versus did not occur; known receipt; persistent effects; handler failure, panic and budget expiry; concurrent runs with distinct identities; retry identity stability; no double compensation after recovery. Missing recovery configuration fails build. Update pure-machine, adapter and invariant-checker semantics together, including the existing “not held ⇒ no release” rule's relationship to explicit unknown-outcome recovery.

## 4. Unify plan inputs and reusable component instances

Recommended design: one canonical typed plan definition, with `()` as the ordinary unit input. Root start, static mount and dynamic spawn must each explicitly supply or bind that input. Remove the semantic distinction where identically typed unit-input plans have different admission requirements.

A static mount binds its child input to a typed parent value/dependency. Binding a parent resource must retain its lifecycle edge: copying an `Arc` into an input must not sever the parent's obligation to remain alive through child cleanup. Use typed formal imports for additional borrowed resources and bind them at mounting, so a reusable child definition does not permanently capture one parent's concrete keys. The common single-input case should not require a separate vocabulary of templates, factories and no-input constructors.

Separate **definition**, **mount**, **run** and **dynamic instance** identity internally. A child definition can be mounted as `Left` and `Right`; each mount has unique paths, slots, locks, pools and output bindings within each run. Immutable declarations/body factories can be shared. Per-run mutable state cannot be shared accidentally. Caller-captured shared state remains the caller's explicit responsibility.

Perform static binding, scope checks, cycle checks, duplicate-name checks, budget validation and mount layout derivation during parent build. Keep defensive checks at the host boundary, but normal typed starts must not discover deterministic structural errors that build could have rejected. Preserve nested failure policy, import ordering and child-before-parent cleanup.

**Acceptance tests:** typed and unit input; repeated sibling and nested mounts; mounting the same definition in two different parents; simultaneous parent runs with different inputs; nested mounts inside dynamic instances; imports referring to the correct parent mount; distinct pool/lock scope; malformed bindings rejected before effects; deterministic inspect/report paths. Re-run the review's composition change with independent assertions.

**Performance direction:** normalize static mount layouts once and address runtime state by compact indices. Reuse immutable body factories through a scope-aware lookup instead of cloning closure environments or rebuilding whole static graphs per run. Measure the memory cost of repeated mounts and the remaining work per dynamic instance.

## 5. Make service restart mean something precise

Recommended contract: initialize and publish the service handle once; recover by restarting a serving episode against that same stable handle. The serving factory receives the handle and fresh episode context. Existing dependents continue to use the published handle and are not silently re-executed.

This is a deliberate API/semantics change from rerunning the current start closure. State explicitly when initialization may retry, when readiness is first established, what readiness means during recovery, how health is observed, and what ends a terminal service. Startup retry and serving restart are separate decisions. A stable handle may internally represent a reconnecting client or mailbox; stability does not promise uninterrupted availability.

Keep asynchronously cleaned resources in resource nodes or owned child scopes. Do not move untracked acquisitions into a restart factory for convenience. Idempotency/restart safety remains an application claim, not a guarantee created by an attribute. Defer automatic replacement-handle distribution and cascading dependent restart: those require a different explicit contract.

**Acceptance tests:** a long-lived dependent continues through multiple serving failures; the published handle is unchanged; initialization runs once on normal recovery; restart exhaustion follows the declared failure policy; shutdown interrupts backoff; stop during startup/recovery honors budgets; recovered episodes do not leak tasks or resources. Add a real stop-aware guide, replacing the immediately-completing resident example.

## 6. Improve reviewability and the canonical authoring surface

Use one primary chain-based construction route. Remove redundant aliases and positional `_with` variants where they add vocabulary without capability. Keep a second form only when a concrete use case demonstrates its value. Exact public signatures are selected with small compiling consumer examples before the implementation spreads across the crate; proposed names in this document are design descriptions, not existing methods.

Provide a standard conversion from a report to a result that preserves the complete report on failure. Its human-readable error should include the nested path, phase, cause and suggested fix, followed by cleanup failures and unresolved obligations. Keep structured fields accessible; models should not need to handwrite a lossy summary to get useful output.

Distinguish resource/service keys from completed value keys sufficiently to reject direct export of a live capability from a finite run. Exporting a computed result remains simple. This restriction cannot prove that arbitrary user data contains no cloned handle; document that limit rather than claiming full static lifetime enforcement. Where type complexity outweighs benefit, retain one key type with deterministic build rejection and clear diagnostics instead of introducing a misleading type guarantee.

Give inspection a lifecycle view and a data-binding view. Include typed input/import bindings and mount identity in the latter; inputs remain non-cleanup entities. Show effective policies and whether they were explicit, stable service publication/recovery, and unknown-outcome handling. Keep deterministic output suitable for diffs. Do not imply inspection proves side effects, idempotency or the behavior of closures.

Adapt the proposed README to the revised API, retaining its concrete success/failure cleanup example. Add complete executable guides for static extraction with input, repeated mounts, resident stop/recovery, unknown-outcome recovery and dynamic child containment. Supply a compact AI authoring reference with exact common signatures, error annotations, one-acquisition rules and full-report handling. Keep review history, benchmark methodology and internal invariant IDs in `dev-docs`, not user guides.

Do not expand into actors, durable workflows, borrowed task scopes or general finite fan-out/results in this tranche. Clarify existing dynamic-child limits and preserve their ownership behavior. These alternative capabilities are not required to fix the demonstrated problems.

## 7. Performance accounting and optimization

### Separate tallies

“Plan generator” covers both an AI that writes plan source and any synthetic/programmatic fixture generator. Neither belongs inside the engine benchmark timer. One-time library validation/normalization is also separate. Work that the current implementation repeats at every start remains runtime cost until the implementation actually moves it out.

| Tally | Included | Reported measures |
|---|---|---|
| A. Authoring/generation | Model load, prompt ingestion, generation, repairs, tool interactions; separately, synthetic fixture generation | Tokens, context size, calls, load/inference/wall time, bounded task outcomes; generator time separately |
| B. Rust compilation | Dependency and application compilation, incremental rebuild | Cold/warm build time and artifact size, with cache/toolchain details |
| C. Plan building | Declaration construction, validation, static mount binding, execution-layout preparation | Time, allocations, bytes, scaling with nodes/edges/mounts |
| D. Engine execution | Per-run input binding and state creation, admission, scheduling, body dispatch, retries, dynamic instantiation, readiness, cancellation, cleanup, report production and per-run disposal | Throughput, latency distribution, allocations/bytes per run and per instance, peak memory, idle CPU and shutdown latency |
| E. Representative application | D plus real application bodies and I/O | End-to-end latency/throughput, presented alongside D rather than labeled pure engine overhead |

Never combine A–E into a headline “sdax performance” score. If useful, show first-use or amortized deployment economics separately with their components visible. Excluding a generator does not permit pre-creating mutable run state, pre-binding each request, pre-spawning children or disabling required cleanup outside the measured interval.

### Benchmark boundaries

For repeated-run throughput, construct the fixture, runtime and immutable plan outside the timer. Begin each measured run immediately before input binding/run creation; end after report collection and per-run disposal. Generate input payloads ahead of time, but count binding them. Observe outputs so work cannot disappear through optimization. Keep correctness assertions outside the timed loop where possible, with an equivalent verified fixture.

Also publish isolated setup/admission/teardown measurements and a pure-machine event benchmark. A pre-seeded machine-only microbenchmark must be labeled as such; it is not the primary repeated-run figure. Dynamic instance creation belongs inside the runtime measurement, even when the child template was prepared beforehand. Record adapter/runtime creation and final host shutdown separately, while counting any per-run drain they must perform for the run to finish correctly.

Use real monotonic time for performance; injected time remains for deterministic correctness tests. Report optimized builds, worker count, tracing/inspection settings, warm-up, repetitions and distribution. Run allocation instrumentation separately if it perturbs timing. Keep generator/GPU activity idle during CPU benchmarks on the Windows host.

### Workload matrix

| Workload | Scales / variants | Primary question |
|---|---|---|
| Tiny finite run | One step; one resource plus step; immediately-ready versus yielding bodies | Fixed per-run cost and allocation floor |
| Dependency graph | Chain, wide fan-out/fan-in, sparse mixed DAG; 1, 10, 100, 1,000 nodes, explicit edge counts | Is work proportional to affected graph structure or repeatedly scanning everything? |
| Component reuse | Equivalent flat graph versus nested graph; repeated mounts; concurrent parent runs | Cost of abstraction and isolation |
| Resource teardown | Normal release, partial startup failure, release failure, cancel after acquisition | Correct cleanup cost and tail behavior |
| Resident services | Idle, readiness, stop, recovery and exhausted restart | Idle CPU, wakeups and recovery/shutdown latency |
| Dynamic children | 1, 10, 100 live instances; repeated churn; imported resources | Per-instance time/memory, containment and reclamation |
| Retry / ambiguity | Known receipt, unknown outcome, successful/failed recovery | Safety-path cost without omitted work |
| Observability | Production default versus trace enabled | Explicit tracing allocation and latency cost |

Compare to handwritten Tokio for the same observable behavior, including partial startup, cleanup errors and cancellation. Report the amount of manually implemented lifecycle policy. An intentionally minimal Tokio lower bound may be shown separately, but must not be labeled equivalent. Compare new sdax against the reviewed baseline on shared supported behavior; classify bug-fixed/new behavior separately where the baseline cannot satisfy the contract.

Run Mac arm64, Pi arm64 and Windows x64 independently. Do not average across hardware. Use the Pi to expose scaling and memory cost, and Windows to catch adapter/platform differences. No model-inference time goes into these tables.

### Optimization sequence and gates

The current `Machine::{new, with_input}` calls `Table::build`, and `host::bodies::build_bodies` traverses scopes/templates on start. Profile these first. Separate immutable derived tables from mutable run state and cache the former at plan build. This is a measured target, not a claim that either function is already the bottleneck.

Then profile affected-node admission, readiness and release queues, dependency lookup, allocation/boxing, reference counting, dynamic-instance reclamation and diagnostic formatting. Prefer dense indexed state and precomputed adjacency where they preserve deterministic ordering. Do not weaken cleanup, cancellation, fairness or reporting to win a microbenchmark. Do not add a cache that shares mutable state across runs.

Initial engineering gate: flag a repeatable median regression above 5% or p95 regression above 10% on an existing supported workload when it exceeds measured noise. Investigate allocation growth and scaling changes even when timing stays within the gate. These are proposed regression triggers, not current measured precision or promised speedups. Establish host noise floors and freeze the gate after baseline collection. Changes required for correctness may have a documented cost; surface that tradeoff rather than hiding it or silently redefining the workload.

An optimization is complete only when a recorded before/after run shows improvement on its target workload, no unexplained material regression elsewhere, and all semantic checks still pass. Publish raw samples and absolute times as well as ratios. Set absolute service-level budgets after baseline measurement; none can honestly be inferred from the review's inference timings.

## 8. Validate inexpensive authoring independently

Retain the original quantized 27B model as one comparison point. Add a genuinely smaller local model when available, recording its provenance, size, quantization and execution settings. Do not equate 27B success with small-model usability.

Use frozen scenarios, bounded repair loops and external behavioral assertions. Include new composition changes, repeated mounts, resident stop/recovery, cancellation before receipt, and cleanup failure from the start. Separate documentation discovery from code generation where measured. Count invented APIs, compile repairs, semantic repairs, full diagnostics preserved, context tokens and reviewer corrections.

Run both a reproduction track with comparable context to the review and a compact-context track that tests the new authoring reference. Reduce context in controlled steps rather than declaring one arbitrary token budget sufficient. Use several tasks/runs and report denominators; zero observed failures is not a population guarantee. Human reviewability needs actual review observations, not just source-line counts.

**Gate:** every canonical example passes; the original composition regression succeeds within the fixed repair bound; held-out results and all failures are published separately. Improvement in A cannot compensate for a regression in D, and runtime improvements do not excuse unsafe model-generated code.

## Delivery order

| Milestone | Work | Gate to proceed |
|---|---|---|
| M0: evidence and measurement | Promote failing expectations; contract decisions; baseline engine/build/generation measurements | Known failures reproduced; timing boundaries and fixtures frozen |
| M1: obligation correctness | Single-use acquisition, incomplete declarations, explicit unknown-outcome recovery | Desired safety assertions pass on pure and Tokio drivers; reports preserve unresolved work |
| M2: reusable definitions | Input binding, mount identities, build-time structural validation, immutable execution layout | Repeated mounts/concurrent runs isolated; no static composition trap remains at normal launch |
| M3: service and review surface | Stable-handle recovery, output lifetime rules, inspection and lossless diagnostics | Long-lived dependent recovery and full error-path tests pass |
| M4: performance | Profile and optimize the revised engine against M0; dynamic churn and observability costs | Recorded performance report with separate A–E tallies and explained regressions |
| M5: adoption validation | Canonical docs/README; independent local-model tasks; portability and full gates | Reproducible examples, authoring evidence and complete verification report |

Performance runs accompany M1–M3; M4 is the focused optimization stage, not the first time performance is checked. Update examples with each API change rather than leaving a broken public guide until M5. Keep each behavioral unit reviewable and recorded in the TDD log.

At implementation completion, run the repository's shared `scripts/check_all.py` inventory and explicit Rust 1.75 library checks, including native Windows and Pi verification for changed runtime code. Use `uv` for any Python environments. Document the remaining schedule-exploration limits; deterministic and random testing must not be described as exhaustive enumeration.

Produce a final implementation report mapping every row above to tests, decisions and measurements, plus revised README/guides and a benchmark reproduction guide. No compatibility aliases are required. Committing, pushing and publication remain separate actions under repository rules.

## Definition of done

The receipt-less and double-acquisition failures are fixed rather than merely documented. Incomplete nodes cannot vanish unnoticed. One reusable input-bearing definition can be mounted independently more than once. Service restart has one explicit handle contract. Standard reports retain enough information for an inexpensive model to repair an error and for a person to review its consequences.

The original cleanup advantages still hold under failure and cancellation. Runtime numbers include every per-run obligation and exclude plan generation, Rust compilation and one-time plan building, each of which has its own visible measurement. The final claims distinguish what was executed, what was inferred and what remains unmeasured.

## Follow-up priorities after Windows validation

Windows validation resumed through Git Bash in fresh retained workspaces. Baseline and candidate fixture checks, builds, full measurements and a reverse-order timing repeat passed execution; this completes platform coverage, not all performance acceptance gates. See [Windows evidence](../../artifacts/windows-performance-resume-20260908/WindowsPerformanceResults.md).

1. Profile dynamic child admission, first-write layout copying, instance lookup and teardown. Windows 1/10-child medians regress in both orders; allocation growth is reproducible. Measure retained topology/history bytes over long churn and define reclamation semantics before changing them. Keep full lifecycle/disposal inside runtime timing.
2. Address authoring shapes and diagnostics using the recorded failed examples as regression cases: component input versus resource import, automatic Arc wrapping, required output export, acquisition retry and complete report formatting. Prefer a smaller canonical surface or clearer compiler diagnostics over expanding the context prompt. Validate improvement on new held-out composition/recovery tasks with a fixed inference/repair budget; retain the original failed matrix.
3. Profile one-time plan validation/layout allocations separately. Seek shared immutable representation without moving repeated runtime work outside the timer or weakening validation. Record compile/build costs separately from execution.
4. Investigate host/order-sensitive service and known-receipt timing before asserting a regression is fixed. Retain both Windows captures and the earlier Mac flags. Complete lifecycle-equivalent Tokio and peak-live-memory comparisons before making competitive throughput or memory claims.

No further model calls were made during Windows resumption. Current library source remains the previously verified revision; no new runtime fix is implied by this measurement pass.

### Dynamic-child follow-up: first measured fix

The profile identified repeated readiness-latch cloning and nested lookup in the adapter. The keyed, once-answered readiness registry is implemented and validated. Windows 10-child runtime medians improve 7.7–9.5% and 100-child medians 22.5–24.0% versus the previous candidate. Requested bytes for 100 children fall 12.8%. The original ten-child allocation budget remains unmet (4,384 versus 4,361 calls), the one-child timing is mixed, and metadata/history reclamation remains open. This is partial progress on priority 1, not completion of the optimization gate. See [measurements and validation](../../artifacts/dynamic-profile-20260908/DynamicReadinessResults.md).
