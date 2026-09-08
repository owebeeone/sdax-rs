# Independent evaluation preparation

Date: 9 September 2026. Lane C, local GWZ family `evaluation`. Library source
remained the baseline `ce339a59eefcd136562b16f4f20035c29ef0c988` throughout this
preparation. **No inference, model loading, remote execution or library changes
were performed.** This is preparation evidence, not an adoption result.

## Deliverable and provenance

The [standalone harness](../evaluation-harness/README.md) contains six newly
authored held-out scenarios, immutable executable assertions, eight independent
known-correct implementations, eleven targeted negative mutations, a durable
runner and the proposed pre-inference protocol. It is outside normal workspace
membership. Its only Cargo dependencies are the local libraries and pinned Tokio
with test utilities for deterministic paused time; no normal library dependency
was added. Python uses only its standard library.

The lost raw authoring-eval and exposed generated programs were unavailable.
The two exposed trials are explicitly **reconstructed regressions from documented
failure categories**, not exact reproductions. They cannot establish that the old
composition program was repaired or reproduce the old denominator. Four other
historical exposed scenarios were not recovered. Their absence is not hidden by
counting new cases as old ones.

Fresh tasks were authored independently from abstract required coverage and
baseline public APIs, without the old prompts or generated source. Their exact
text and solutions were withheld from authoring/diagnostic implementation lanes.
Two composition cases, two identity/recovery cases, resident recovery/stop and a
combined body/cleanup failure remain six distinct fresh trials; two preselected
context-reduction repeats are a separate denominator.

## Preregistration and TDD evidence

The initial task/assertion manifest was frozen before control implementation and
before implementation lanes finished:
`21ead0c80e2a6b5c63a80d93d14ba26978a0a32d0ae24c340107e8793c11bf24`.
Its [original manifest/assertions](../evaluation-harness/preregistration-v1/)
remain intact. Missing candidate source produced the first
[compile RED](../evaluation-harness/validation/controls-red.log).

Known-correct control execution exposed three **oracle mistakes**, not library
bugs. The amendments were disclosed to the integration owner before inference:

1. `Recovery::StillUnknown` retains a cleanup failure as well as ambiguity;
   v1 incorrectly expected no cleanup failures.
2. Stopping during serving backoff preserves the unrecovered serving fault;
   v1 incorrectly expected the clean result of a completed recovery episode.
3. A nested body fault also produces containing-mount prepare records;
   v1 incorrectly counted all fault records as originating body faults.

The [three RED outputs](../evaluation-harness/validation/oracle-red-f4.log)
([resident](../evaluation-harness/validation/oracle-red-f5.log),
[nested](../evaluation-harness/validation/oracle-red-f6.log)) are preserved.
Version2 corrected these classifications, formatted fixture code and added
separate diagnostic checks. Version3 added the same separately measured check
for the resident case. No task text, scenario selection or required semantic
behavior was removed. Earlier manifests remain in `preregistration-v1/` and
`preregistration-v2/`; the initial ten-control snapshot is superseded and was
never used for inference.

Final [task/assertion manifest](../evaluation-harness/task-freeze.json):
`21d9607723fd285e6467c07ac99ca1ba5831324917ce161377199eb2a67b2062`.

## Executed validation

- [Frozen controls](../evaluation-harness/validation/review-final-control-results.json):
  **10/10 compile, behavior and diagnostic passes**, including the two explicitly
  separate reduced-context repetitions. Eight unique implementations.
- [Targeted negative controls](../evaluation-harness/validation/negative-results.json):
  **11/11 compile successfully and are rejected by assertions**. They cover
  wrong results/bindings/tuple projection, premature parent cleanup, identity
  instability, retry limits, lost uncertainty, wrong compensation receipt,
  insufficient restart allowance and first-fault-only diagnostics. These are
  executed mutation witnesses, not a blanket correctness proof.
- [Runner tests](../evaluation-harness/validation/all-final-tests.log):
  **21/21 offline runner tests passed**, plus **2/2 preparation tests**. RED/GREEN logs are retained. Witnesses include
  pre-request durable charging, interruption consumption, aggregate budget,
  immutable input changes, post-build evaluator changes, UTF-8/request overflow,
  exact clearance/control binding, resume reconciliation (including zero-call budget
  failures, explicit charged-slot outcomes and corrupt retained responses), partial
  test timeout classification and rejecting successful
  process exit without the frozen tests running.
- Rust fixture formatting passes. Builds use `--locked --offline`; sources,
  lockfiles and evaluator files were hashed before/after final control builds.
  No engine defect or engine RED is claimed. Complete shared project gates and
  native/MSRV checks belong to the integration owner.

The [baseline frozen snapshot](../evaluation-harness/validation/review-final-frozen.json)
has SHA-256
`82aaf66f572b5c6d4c98ba41eb30b53ae8c0ca5930e9300260083ee33c49b3b4`.
Its protocol hash is
`b623e4ea9da3060f18b1306071eae11ccf20d807d7100d93e1ded01f7cd0a1e3`.
Runner SHA-256:
`b8ae1e9e9b9dd7306add58056fa67d41c3cff5bf1d7dac16f220f1a820428722`.
The [evidence inventory](../evaluation-harness/validation/manifest.json) includes
full final generated negative programs and build diagnostics. All temporary
packages/build outputs are retained in the lane's `evaluation-harness/evidence/`
and `evaluation-harness/runs/`; only concise evidence is version controlled.

## Fixed proposed inference protocol

Both `qwen3.8:27b` and `gemma4:26b` receive at most **30 generation calls each,
60 total**: two reconstructed exposed trials, six fresh trials, and two reduction
repeats, each initial plus at most two repairs. This budget is proposed/frozen
before inference and is never extended to rescue outcomes. Exact model digests
were supplied by the integration owner's earlier live `/api/tags` verification;
server version was `0.33.0-dabeest` and both prior smoke requests passed with
`think:false`.

Common settings are temperature 0, seed 42, output 4,096 tokens, `think:false`, and
model context 32,768 tokens. Public context caps remain 16,000/8,000 UTF-8 bytes;
complete serialized requests cap 64,000 bytes. The fixture API and task are
additional task material inside that whole-request limit. Baseline full/reduced
public contexts, including file labels, measure 11,375/6,146 bytes. The original reduced context
selected complete Components and Services sections. The final reference reorganized
those headings, so an explicit pre-inference amendment selects the first two
complete level-two sections in document order, excluding introduction/later
sections. It adds no excerpts or task hints. The original selector is preserved
with its exact prior hash and its failing new-heading preflight; two neutral
selector regression tests pass. Preparation against the finalized authoring
reference passes both caps: **15,535/3,213 bytes**, including file labels.
The [preflight record](../evaluation-harness/validation/final-authoring-context-preflight.json)
records hashes and zero inference calls. The final
integrated source/context still requires its own freeze and control run.

The runner freezes source, protocol, contexts, model digests, tasks, assertions,
controls and lockfiles. It validates hashes before each inference/build and after
attempts, rechecks the model digest before generation, and requires a matching
passed ten-case control run plus exact integration-owner clearance and a host
reservation. The parent-host loopback endpoint forwards to dabeest. Inference
must not overlap performance measurement on that host.

A fsynced append-only ledger charges each call before transmission under an
exclusive lock. Uncertain/infrastructure requests consume slots. Resume requires
explicit bound reconciliation for uncertainty; no refunds or silent replacements.
Repairs contain only the original task/context, latest answer and exact compiler/
test diagnostics. Overflow is recorded without truncation. Requests, answers,
generated code, full logs, token counts, load/inference/wall durations and outcomes
are retained.

## Remaining work

Integrate source/docs, regenerate context, freeze the final candidate, rerun the
matching controls and record clearance before inference. The current baseline
snapshot is not candidate authorization. The README gives exact runner commands
and clearance/resume fields.

Generated-program structural review remains a distinct pending requirement:
formal resource ports, independent bindings, single-use acquisition, declarative
retry/restart, stable handles, completed exports and no handwritten lifecycle
management. No model passes, repaired-original claim, human-reviewability result,
small-model generalization or population success rate is claimed by these controls.
One deterministic sample per case will remain bounded evidence.
