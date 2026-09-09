# Proposal: make SDAX authoring easier

Date: 9 September 2026

Status: proposals 1 and 2 implemented; proposal 3 deferred; proposal 4 exploratory.
Baseline: sdax-rs `5a62e0bf84c1e8341a3cedc1c727c4a6c0e0a26f`

## Evidence and scope

Experiments 2 and 3 kept the library API unchanged. Experiment 2 compared documentation packages; experiment 3 compared free-form authoring with a compiling, behaviorally unfinished scaffold. Both experiment-3 arms received the same focused reference and the same task requirements.

Experiment 3 produced these results after compilation, executable assertions and independent structural review:

| Model | Free-form | With scaffold | Calls, free-form / scaffold |
|---|---:|---:|---:|
| Gemma | 0/4 | 1/4 | 12 / 10 |
| Qwen | 2/4 | 4/4 | 12 / 6 |

Three paired wins, no losses and an improvement for each model met the predeclared directional threshold. Scaffolds supplied graph architecture, declaration order, policies and capture structure as well as types. This supports the complete scaffold package; it does not isolate type annotations or establish that any particular API change will improve results. The four tasks were exposed regression cases with one deterministic sample per cell.

Local report: [scaffold results](AuthoringScaffoldResults-2026-09-09.md). Full evidence remains on dabeest at `D:\sdax-exp2\exp3`; experiment 2 remains at `D:\sdax-exp2`.

## 1. Permit equivalent configuration orders around identification

**Priority: first API change to prototype.**

Today, `identified_by` changes the builder into `IdentifiedEffect`, which exposes `perform` but not configuration methods such as `needs` and `within`. Authors repeatedly placed valid configuration on the wrong side of this transition.

Forward semantically valid configuration through `IdentifiedEffect`, initially dependencies and body timeout. Assess retry/idempotence configuration under the same principle. Preserve the identity dependency and the requirement to provide unknown-outcome recovery. This does not make terminal operations interchangeable or add declaration methods to a finished `Key`.

Validation:

- Compile equivalent chains with configuration before and after identification.
- Verify identical dependency values, timeouts and stable identity through retries/recovery.
- Verify that replacing ordinary dependencies retains the identity edge and does not duplicate it.
- Retain rejection of incomplete or semantically invalid declarations.

Current implementation: `crates/sdax/src/recovery.rs`, `IdentifiedEffect` and `identified_by`.

## 2. Add a required-output helper to Report

**Priority: second, small additive change.**

`Report::into_result` currently returns `Result<Option<Arc<Out>>, Report<Out>>`. Consumers must separately handle failure reports and absent output. Experiments exposed panic-on-missing-output implementations and ambiguity in the earlier task specification.

Add an opt-in helper that returns completed output or a typed error distinguishing a failed run from missing output. Preserve the full report and its original error objects; avoid automatically reducing errors to strings. A name such as `into_required_output` is illustrative, not a settled API. Keep `into_result` for callers that legitimately accept no output.

Validation: completed output succeeds; clean missing output returns an error without panic; failure preserves the complete report, cleanup details, ambiguity and typed error access. Do not fabricate a default output.

Current implementation: `crates/sdax/src/report.rs`, `Report::into_result`.

## 3. Bring component mounting and bindings together

Decision, 9 September: defer implementation. A mounting builder would add another
public construction path while retaining formal ports and their ownership rules.
The scaffold experiment supports complete starting programs, not this particular
API. Reconsider after a concrete sketch demonstrates fewer decisions at a call
site without hiding lifetime edges, followed by a bounded authoring comparison.

**Priority: design prototype after the smaller changes.**

Authors currently coordinate a formal port on the child builder, `Plan::bind` on the finished definition, and `parent.component` with a parent input key. Failures included calling these methods on the wrong object and confusing scalar inputs with input bindings.

Explore an additive mounting builder that collects the child definition, ordinary input and formal resource bindings in one place, then completes the mount. Formal ports would still be declared in the reusable child definition. Exact names and syntax remain open.

Validation: mount one definition twice with distinct resources and inputs; prove independent outputs and child-before-parent cleanup. Preserve rejection of unbound ports, wrong resource types and invalid ownership. Binding one mount must not mutate another mount or the reusable definition. Preserve existing APIs during the prototype.

Current implementation: `crates/sdax/src/builder.rs` (`port`, `component`) and `crates/sdax/src/plan.rs` (`bind`).

## 4. Explore higher-level declaration helpers

**Priority: exploratory; do not commit to a broad redesign yet.**

Prototype helpers or declaration syntax that generate the dependency/closure types, separate configuration captures and acquisition/release structure supplied by the successful scaffolds. Determine whether library helpers, macros or external code generation provide the clearest result before selecting a public API.

The expansion must retain lazy external acquisition inside `hold`, single-use acquisition authority, explicit lifetime edges, complete cleanup registration, stable service handles and full diagnostic preservation. It must not make incorrect lifecycle behavior appear valid merely by hiding it.

Validate generated declarations against the existing lifecycle and diagnostic assertions, including failure paths. Evaluate authoring improvements separately from implementation correctness.

## Strongest immediate evidence: starter templates

Compiling starter templates or a scaffold generator have stronger experimental support than any of the proposed API changes. Maintain complete examples for resource acquisition, resource-bearing composition, identified recovery and resident services, with compilation and behavioral checks that keep them synchronized with the library.

This is tooling/documentation work and should be distinguished from API redesign. In particular, Gemma still failed three scaffold tasks, so templates are not a general reliability guarantee.

## Suggested implementation sequence

1. Prototype identification-order forwarding and the required-output helper as separate additive changes.
2. Maintain compiling starter templates alongside those prototypes.
3. Design the mounting builder; defer broader declaration syntax until its tradeoffs are concrete.
4. Freeze another bounded comparison before evaluating authoring impact. Use fresh task variants for generalization evidence, explicitly specify missing-output behavior, and retain separate compilation, behavior, diagnostics and structural scores.

The original proposal recorded possible changes, not measured API benefits.
Proposals 1 and 2 were subsequently implemented: `IdentifiedEffect` forwards
`needs`, `within`, `retry` and `idempotent`; `Report::into_required_output` returns
an `Arc` or `RequiredOutputError`, retaining the complete report in both failure
variants. See [implementation evidence](ApiAuthoringImplementation-2026-09-09.md).
No new authoring evaluation or repository restructuring is claimed.
