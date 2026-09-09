# SDAX experiment 2 — 9 September 2026

The focused executable reference produced a limited Qwen improvement, but the experiment did not meet its predeclared threshold. This is a mixed/inconclusive result, not evidence that the API is intrinsically too difficult.

## What happened

All 48 allocated calls completed: 24 per model, 12 per documentation arm. No initial response passed. After up to two repairs, Qwen passed the retry/cleanup and resident-service executable checks with reference B. Reference A produced no executable passes; Gemma produced none with either reference.

| Model | Reference | Eventual executable passes | Calls | Compile failures | Output-limit responses |
|---|---|---:|---:|---:|---:|
| gemma4:26b | A | 0/4 | 12 | 12 | 0 |
| gemma4:26b | B | 0/4 | 12 | 8 | 4 |
| qwen3.8:27b | A | 0/4 | 12 | 12 | 0 |
| qwen3.8:27b | B | 2/4 | 12 | 10 | 0 |

Across all attempts: 42 compilation failures, four output-limit responses, and two executable passes. Repair attempts are not independent task samples. Both executable successes required the second and final repair.

## Independent structural review and an oracle limitation

The reviewer received 16 opaque final-cell packets with task/support/criteria and source where available, without model/reference labels or executable outcomes. Fourteen final sources were available; two final responses had no evaluable source. Source style could still suggest documentation ancestry, so this was label concealment rather than guaranteed complete blindness. Earlier attempts remain retained and are summarized separately by diagnostic category.

The prompt required “missing-output checks at boundary” but did not explicitly require returning Err instead of panicking. Both executable successes use expect on absent output. The independent task author and reviewer agreed that this alone cannot be retroactively declared a definite violation. The review leaves that interpretation unassessed; the main accounting does not silently turn these into certified primary passes. The exact interpretation and sensitivity are retained in evidence/boundary-interpretation.json.

Definite source findings in other candidates include invoking external factories before the lazy hold closure, using finite mode for a resident service, and omitting the declared request timeout. Invalid API/type shapes are kept separate from demonstrated lifecycle violations.

| Model | Task/reference | Executable | Structural | Primary |
|---|---|---|---|---|
| gemma4:26b | T1-A | fail | unassessed | failed |
| gemma4:26b | T1-B | fail | unassessed | failed |
| gemma4:26b | T2-B | fail | unassessed | failed |
| gemma4:26b | T2-A | fail | unassessed | failed |
| gemma4:26b | T3-A | fail | failed | failed |
| gemma4:26b | T3-B | fail | unassessed | failed |
| gemma4:26b | T4-B | fail | failed | failed |
| gemma4:26b | T4-A | fail | failed | failed |
| qwen3.8:27b | T4-A | fail | failed | failed |
| qwen3.8:27b | T4-B | pass | unassessed | unresolved_specification |
| qwen3.8:27b | T3-B | fail | unassessed | failed |
| qwen3.8:27b | T3-A | fail | failed | failed |
| qwen3.8:27b | T2-A | fail | failed | failed |
| qwen3.8:27b | T2-B | pass | unassessed | unresolved_specification |
| qwen3.8:27b | T1-B | fail | unassessed | failed |
| qwen3.8:27b | T1-A | fail | failed | failed |

The predeclared rule required B to win at least three of eight paired comparisons, lose at most one, and improve at least one task for each model. Even accepting both ambiguous boundaries gives only two B wins, zero losses, six ties, and improvement only for Qwen. The overall conclusion is unchanged under either interpretation.

## What the failures tell us

Repeated compiler problems involved builder versus finished-plan methods, dependency and Arc value shapes, closure capture ownership, context phases, and method ordering. Examples include calling export on Plan, port on a finished Plan, using scalar values where InputBinding is required, and calling hold on Run or Release contexts. Qwen still failed composition and recovery with the new reference. Gemma generated more output with B and hit the fixed output limit four times.

These observations support documentation having some influence on executable authoring for Qwen. They do not separate model capability, Rust/type-system demands, API discoverability, task difficulty, and the fixed repair/output budget. B changed content, organization and length together (A: 15,518 bytes; B: 12,567 bytes), so this was a package comparison, not an isolated wording experiment. Four task families and one deterministic sample per cell cannot establish general reliability.

A useful next experiment would compare free-form authoring with a compiling typed scaffold for the same task and unchanged API. That would help distinguish recalling API/type shapes from expressing lifecycle behavior. Specify and test missing-output error behavior before that experiment. No additional calls or API changes were made here.

## Accounting and reproducibility

| Model | Reference | Prompt tokens | Generated tokens | Ollama-reported seconds |
|---|---|---:|---:|---:|
| gemma4:26b | A | 92,428 | 14,932 | 114.611 |
| gemma4:26b | B | 87,394 | 27,692 | 161.625 |
| qwen3.8:27b | A | 78,797 | 10,276 | 171.248 |
| qwen3.8:27b | B | 72,877 | 11,330 | 161.166 |

Totals: 331,496 prompt tokens; 64,230 generated tokens; 608.650 summed Ollama seconds. All 48 responses and all 48 token records are present. Ledger span from first reservation to final response: 635.899 seconds. These durations are accounting, not a model-speed benchmark.

Preparation passed four correct controls (11 assertions), ten compiling targeted negatives, four exact compiled B examples, and 19 native Windows runner tests. The final strengthened cleanup negative directly failed child-before-parent order without duplicate-release or mutex-poison effects. The exact frozen controls also passed through the production evaluator before inference.

All 230 frozen files were verified unchanged after inference and again at finalization. The run used the committed source snapshot, pinned model digests, temperature 0, seed 42, think false, context 32,768, output limit 4,096, fresh conversations and diagnostic-only repairs. No calls were lost, refunded, added or repeated outside allocation.

All experiment writes, source snapshots, builds, caches, temporary files, responses and reports remain under D:/sdax-exp2 on dabeest. No experiment files were copied back to the Mac and nothing was cleaned up. About 796.8 GiB remained free at completion.

- Protocol: EXPERIMENT.md (its original pre-run status line is retained because it is frozen).
- Exact freeze: evidence/frozen.json; SHA-256 2694ddec5c675135bdcf2cf85de6244739e171c838952f51a8895500c4695207.
- Raw run: runs/run-a2197efe95d14531b72b8faabdaa84bd/.
- Durable call ledger: runs/calls.jsonl.
- Machine-readable merged results: evidence/final-results.json.
- Independent review: evidence/blind-review-results.json; packets: evidence/blind-review/.
- Detailed compiler/token accounting: evidence/executable-accounting.json.
- Retained artifact inventory: evidence/archive-manifest.json.
