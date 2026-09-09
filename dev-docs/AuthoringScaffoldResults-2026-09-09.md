# SDAX experiment 3 — 9 September 2026

The typed scaffold package met the predeclared directional improvement threshold.
This compares free-form authoring with completing a compiling scaffold using the same API, tasks and focused reference. The scaffold supplies graph architecture, declaration order and configured policies as well as types. The result therefore concerns the complete starting-code package, not type annotations alone.

## Primary results

Primary acceptance requires compilation, all frozen behavioral/diagnostic assertions, and independent structural acceptance.

| Model | Free-form | With scaffold | Free-form calls | Scaffold calls |
|---|---:|---:|---:|---:|
| gemma4:26b | 0/4 | 1/4 | 12 | 10 |
| qwen3.8:27b | 2/4 | 4/4 | 12 | 6 |

Paired primary comparisons: 3 scaffold wins, 0 losses, 5 ties, 0 unresolved. The rule required at least three wins, at most one loss, and at least one improved task for each model.

| Model | Task | Free-form primary | Scaffold primary | Calls A/B |
|---|---|---|---|---:|
| gemma4:26b | Composition | failed | failed | 3/3 |
| gemma4:26b | Retry and cleanup | failed | passed | 3/1 |
| gemma4:26b | Identified recovery | failed | failed | 3/3 |
| gemma4:26b | Resident service | failed | failed | 3/3 |
| qwen3.8:27b | Composition | failed | passed | 3/2 |
| qwen3.8:27b | Retry and cleanup | passed | passed | 3/1 |
| qwen3.8:27b | Identified recovery | failed | passed | 3/1 |
| qwen3.8:27b | Resident service | passed | passed | 3/2 |

## Executable and repair outcomes

| Model | Arm | Initial passes | Eventual passes | Compile failures | Compiled attempts | Output-limit responses |
|---|---|---:|---:|---:|---:|---:|
| gemma4:26b | Free-form | 0/4 | 0/4 | 9 | 0 | 3 |
| gemma4:26b | Scaffold | 1/4 | 1/4 | 4 | 6 | 0 |
| qwen3.8:27b | Free-form | 0/4 | 2/4 | 10 | 2 | 0 |
| qwen3.8:27b | Scaffold | 2/4 | 4/4 | 2 | 4 | 0 |

Gemma passed retry/cleanup on its first scaffold attempt. Qwen passed retry/cleanup and recovery on its first scaffold attempts, and composition and service after one repair each. Qwen needed two repairs for its two free-form successes. Gemma’s scaffold composition and service code compiled but failed behavioral acceptance. Its recovery attempts still appended persistent to an already-finished Key, producing the same method error across repairs.

The remaining Gemma behavioral errors were concrete task-implementation problems: composition reused an acquisition name and triggered duplicate-acquisition rejection; the final service attempt emitted the wrong observation text format while the drain, missing-output and clean-report checks passed. Compilation gains are not counted as successful task completion.

Across all 40 attempts: 25 compilation failures, three output-limit responses, 12 compiled attempts, and seven executable passes. Five compiled attempts failed behavioral acceptance. Successful cells stopped early; the unused eight calls were not spent or reassigned.

## Design and interpretation

Arm A received the exact focused reference from experiment 2 (12,567 UTF-8 bytes). Arm B received that identical reference plus its frozen typed scaffold. These A/B labels differ from experiment 2: this comparison does not use the old long reference. Both arms received identical task/support text and the same output/repair caps; only the scaffold package was added.

The four tasks are exposed regression cases from experiment 2, not new held-out cases. No prior generated answers entered requests. Before freezing, each task was amended to require Err(String) for absent completed output, never panic or invented success. A new executable test enforces this in every task. Earlier experiment-2 scoring remains unchanged.

Scaffolds supplied typed Plan/Key/dependency/closure shapes, separate captures, lazy hold placement, resource and service graph architecture, formal bindings, method ordering, retry/restart parameters, modes and boundary match structure. Fixture calls, arithmetic, observations, releases, recovery choices, service behavior and output handling remained TODOs. Each scaffold compiled but failed behavioral acceptance before inference. Dummy statements that consumed values were removed and scaffolds formatted before the final freeze. Models could modify the starting structure.

This is exploratory directional evidence from eight paired comparisons, one deterministic sample per cell, two models and four exposed task families. It supports supplying compiling starter templates for these patterns. It does not prove population reliability, isolate type information from architecture, establish an intrinsically difficult API, or show that every model can use the API independently. Fresh task variants would be needed before an adoption-readiness claim. No API changes or further inference were performed.

## Independent review

The reviewer inspected final source packets in randomized opaque order with task/support/criteria, without model/arm labels or executable results. Source structure could reveal scaffold ancestry, and the reviewer had reviewed preparation scaffolds; this is label concealment, not guaranteed complete blindness. Missing source and properties unestablishable from invalid API shapes remain unassessed. Definite violations have line-specific findings in evidence/blind-review-results.json. Structural acceptance is separate from test acceptance.

Structural final-source counts: {'unassessed': 5, 'passed': 7, 'failed': 4}. Primary counts after joining executable outcomes: {'failed': 9, 'passed': 7}.

## Accounting and retained evidence

| Model | Arm | Calls | Prompt tokens | Generated tokens | Ollama seconds |
|---|---|---:|---:|---:|---:|
| gemma4:26b | Free-form | 12 | 83,559 | 24,055 | 164.101 |
| gemma4:26b | Scaffold | 10 | 83,820 | 9,912 | 56.898 |
| qwen3.8:27b | Free-form | 12 | 73,721 | 11,983 | 191.255 |
| qwen3.8:27b | Scaffold | 6 | 37,369 | 5,771 | 79.410 |

Total: 40 calls, 278,469 prompt tokens, 51,721 generated tokens and 491.664 summed Ollama seconds. Ledger span: 549.969 seconds. All response and token records are present; no infrastructure interruption, refunded call or extra repair occurred. Durations are accounting, not a model-speed benchmark.

All 40 actual request hashes match their durable ledger entries. Frozen settings and scaffold presence/absence were verified for every request, including repairs. Requests ranged from 18,481 to 44,355 bytes, below the 64,000-byte cap. Settings were temperature 0, seed 42, think false, context 32,768 and output 4,096. Exact model digests match experiment 2.

All 415 frozen files were verified unchanged after inference and again at finalization. The 178 library/source-snapshot files were verified byte-identical to experiment 2. Preparation retained 25 offline native-Windows runner tests, four controls passing 15 assertions, four compiling unfinished scaffold rejections and four compiling panic-boundary negatives rejected only by the new test. Earlier negative-control evidence for unchanged obligations is retained with hash provenance.

All new files, source snapshots, builds, caches, temporary files and evidence remain under D:/sdax-exp2/exp3 on dabeest. Experiment 2 is retained unchanged. No experiment files were copied to the Mac and nothing was cleaned up. About 796.1 GiB remained free.

- Protocol: EXPERIMENT.md.
- Supplied code and architecture inventory: scaffolds/.
- Freeze: evidence/frozen.json; SHA-256 becd4d8bb3459f565be27df89cee21950b87f08bd6cb57a81643985f647c44ae.
- Raw run: runs/run-10bdc140dac747268bb12afd7a49705c/.
- Durable call ledger: runs/calls.jsonl.
- Joined machine-readable results: evidence/final-results.json.
- Independent review: evidence/blind-review-results.json; packets: evidence/blind-review/.
- Compiler/token details: evidence/executable-accounting.json.
- Request audit: evidence/transport-audit.json.
- Retained file inventory: evidence/archive-manifest.json.
