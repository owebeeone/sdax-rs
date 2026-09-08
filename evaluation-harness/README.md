# Independent authoring evaluation

This unpublished, standalone Cargo consumer is outside the normal workspace.
It adds no dependencies to the libraries. Python uses only the standard library;
Cargo dependencies are the two local libraries and exactly pinned Tokio for the
paused test runtime. All builds use `--locked --offline`.

The eight distinct task/assertion pairs comprise two **reconstructed exposed
regressions**, not reproductions of unavailable prompts/programs, and six newly
authored held-out scenarios. Two preselected cases repeat with reduced context.
No model inference was performed during harness development. Source and compact
reference must be integrated and frozen before inference. Keep held-out tasks
and controls away from implementation authors until that freeze.

`task-freeze.json` records the preregistered criteria. The original v1 assertion
files and manifest are retained in `preregistration-v1/`; v2 explicitly corrects
three baseline-oracle assumptions and adds separately named diagnostic checks.
See the lane report in `dev-docs/IndependentEvaluationPreparation-2026-09-09.md`.
Version3 adds a separate resident diagnostic test for an existing requirement; v2
is also retained. The task text and scenario selection did not change.

## Preparing the integrated candidate

Run these from the member repository; choose a fresh retained output directory.
`prepare_freeze.py` refuses to overwrite an existing directory. It copies the
complete compact reference and selects the first two complete level-two sections
in document order for reduced context, excluding introduction and later sections.
This heading-independent rule replaced the original Components/Services selection
before inference because the finalized reference reorganized those headings.
The original selector, its failing preflight, and old protocol are retained.

```sh
python3 -B evaluation-harness/prepare_freeze.py --source . --output /absolute/new-freeze
python3 -B evaluation-harness/runner.py freeze --protocol /absolute/new-freeze/protocol.json --output /absolute/new-freeze/frozen.json
python3 -B evaluation-harness/runner.py control --frozen /absolute/new-freeze/frozen.json
```

The control command prints its retained output directory. Inference requires an
integration-owner clearance JSON, recording the authorization already obtained
from the user, with these fields:

- `frozen_sha256`: SHA-256 of the exact frozen JSON file.
- `protocol_sha256`: the digest recorded in that frozen file.
- `model_digests`: the exact mapping recorded in that frozen file.
- `human_approved`: true, and `approved_by`: the responsible integration owner.
- `host_reservation`: `id`, `host` (exact local hostname), `exclusive`: true.
  Record the reserved inference host dabeest and its measurement exclusion in
  the reservation record as well; the local endpoint forwards to that host.
- `control_run`: absolute directory of the successful matching ten-case control
  run. Its frozen copy and all expected passed results are validated.

A clearance file's mere existence never authorizes a request. This harness does
not create clearance, load models, start a server, or establish a tunnel.

```sh
python3 -B evaluation-harness/runner.py run --frozen /absolute/new-freeze/frozen.json --clearance /absolute/clearance.json
```

Both named models run under identical settings: temperature 0, seed 42,
`think:false`, output 4,096 tokens, and model context window 32,768 tokens.
Complete public reference context is at most 16,000 UTF-8 bytes; reduction is at
most 8,000 bytes. Supplied task/fixture API is additional task material, counted
in the total serialized request ceiling of 64,000 bytes. Actual prompt/output
tokens and server load/inference durations are recorded; byte caps are not
represented as token budgets. Overflow records a trial budget failure without
truncation or an inference request.

The fixed ceiling is **30 generation calls per model, 60 total**: ten trials,
initial plus at most two repairs each. A success ends its trial. Repairs receive
only original task/context, the latest answer and exact diagnostics. Model
responses, full requests, generated code, logs, elapsed time, token counts and
individual outcomes remain in retained run directories. Source, context, runner,
assertions and lockfiles are checked before each request/build and after attempts.
The local model digest is rechecked immediately before inference.

Each request is charged in an exclusively locked, fsynced shared ledger before
transmission. Interruptions and infrastructure failures consume their slots;
there is no replacement-call exemption. Resume with `--resume /absolute/run`.
If any calls are uncertain, supply `--reconciliation /absolute/record.json` with
`frozen_sha256`, `human_approved:true`, `approved_by`, and the exact
`consumed_call_ids` list of uncertain calls. Reconciliation never refunds calls.
Do not change the frozen ledger path or restart a new protocol to reset a budget.

Compiler, behavior and diagnostic results are separate. A zero process exit
cannot pass without every frozen assertion reporting success. Structural review
is always pending until an actual reviewer examines generated programs for
formal imports, isolated bindings, single acquisition, declared retry/restart,
stable handles, completed outputs and absence of handwritten lifecycle management.
Executable results alone never establish that review or human reviewability.

## Local harness validation

```sh
python3 -B -m unittest discover -s evaluation-harness -p test_runner.py
python3 -B evaluation-harness/check_controls.py
python3 -B evaluation-harness/check_controls.py --negative
```

Known-correct controls are independent evaluator implementations. Targeted
negative mutations test wrong binding, output, tuple projection, premature
cleanup, unstable identity, attempt limits, unresolved outcomes, receipt handling,
restart count and lossy reports. They must compile and fail assertions. The
checker records source/input hashes and retains generated packages and all logs.
No temporary directory or build artifact is automatically removed.
