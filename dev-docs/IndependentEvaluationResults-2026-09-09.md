# Independent evaluation review

Neither model produced a fully passing program under the combined executable and structural criteria. Both scored 0/10 initial executable passes. Qwen passed all executable checks on f2 after one repair, while its resource acquisition still violated the separately reviewed lazy-registration requirement. Gemma had no executable pass. All 58 available responses failed structural review; one additional consumed Gemma call remains unassessed because its response was lost.

Run: `run-8845efbb64c24573a19c9f3290725f21`. Final snapshot: 2026-09-08T18:18:53.706413+00:00.

| Model | Track | Trials started/planned | Initial executable passes | Final executable passes | Final structural passes |
|---|---|---|---|---|---|
| gemma4:26b | reconstructed_exposed | 2/2 | 0 | 0 | 0 |
| gemma4:26b | fresh | 6/6 | 0 | 0 | 0 |
| gemma4:26b | reduction_repeat | 2/2 | 0 | 0 | 0 |
| qwen3.8:27b | reconstructed_exposed | 2/2 | 0 | 0 | 0 |
| qwen3.8:27b | fresh | 6/6 | 0 | 1 | 0 |
| qwen3.8:27b | reduction_repeat | 2/2 | 0 | 0 | 0 |

| Model | Charged / measured calls | Compile passed / failed / not attempted / unknown | Structural failed / uncertain | Known prompt / output tokens | Known request seconds |
|---|---|---|---|---|---|
| gemma4:26b | 30 / 29 | 0 / 25 / 4 / 1 | 29 / 1 | 240939 / 48295 | 310.886 |
| qwen3.8:27b | 29 / 29 | 1 / 28 / 0 / 0 | 29 / 0 | 223124 / 27917 | 457.122 |

Each attempt below is a separate saved response or charged uncertain slot. Attempt 0 is initial; 1 and 2 are diagnostics-only repairs. Final trial outcome uses the final attempted slot, even when earlier source is available.

Exposed cases are reconstructions of documented failures, not exact reproductions. Fresh cases contain six new held-out tasks. Reduction repeats reuse two fresh tasks with the frozen reduced context and are not additional new tasks.

| Model | Trial | Track | Attempts | Final compile / behavior / diagnostic | Final structural | Prompt / output tokens (known) | Request seconds (known) | Cost unavailable |
|---|---|---|---|---|---|---|---|---|
| gemma4:26b | e1 | reconstructed_exposed | 0, 1, 2 | failed / incomplete / incomplete | failed | 21939 / 2283 | 40.530 | 0 |
| gemma4:26b | e2 | reconstructed_exposed | 0, 1, 2 | failed / incomplete / incomplete | failed | 30487 / 3621 | 22.528 | 0 |
| gemma4:26b | f1 | fresh | 0, 1, 2 | failed / incomplete / incomplete | failed | 25996 / 6658 | 37.087 | 0 |
| gemma4:26b | f2 | fresh | 0, 1, 2 | failed / incomplete / incomplete | failed | 29594 / 4055 | 24.505 | 0 |
| gemma4:26b | f3 | fresh | 0, 1, 2 | not_attempted / not_measured / not_measured | failed | 24095 / 6251 | 38.250 | 0 |
| gemma4:26b | f4 | fresh | 0, 1, 2 | failed / incomplete / incomplete | failed | 26934 / 5344 | 32.578 | 0 |
| gemma4:26b | f5 | fresh | 0, 1, 2 | failed / incomplete / incomplete | failed | 25257 / 2895 | 17.654 | 0 |
| gemma4:26b | f6 | fresh | 0, 1, 2 | failed / incomplete / incomplete | failed | 30200 / 7298 | 40.342 | 0 |
| gemma4:26b | r1 | reduction_repeat | 0, 1, 2 | unknown / incomplete / incomplete | not_assessed_uncertain | 9472 / 1995 | 11.800 | 1 |
| gemma4:26b | r2 | reduction_repeat | 0, 1, 2 | failed / incomplete / incomplete | failed | 16965 / 7895 | 45.612 | 0 |
| qwen3.8:27b | e1 | reconstructed_exposed | 0, 1, 2 | failed / incomplete / incomplete | failed | 22296 / 1763 | 52.330 | 0 |
| qwen3.8:27b | e2 | reconstructed_exposed | 0, 1, 2 | failed / incomplete / incomplete | failed | 31281 / 3087 | 52.773 | 0 |
| qwen3.8:27b | f1 | fresh | 0, 1, 2 | failed / incomplete / incomplete | failed | 26603 / 3510 | 53.708 | 0 |
| qwen3.8:27b | f2 | fresh | 0, 1 | passed / passed / passed | failed | 17066 / 2764 | 41.343 | 0 |
| qwen3.8:27b | f3 | fresh | 0, 1, 2 | failed / incomplete / incomplete | failed | 23920 / 3176 | 47.747 | 0 |
| qwen3.8:27b | f4 | fresh | 0, 1, 2 | failed / incomplete / incomplete | failed | 21541 / 2653 | 39.458 | 0 |
| qwen3.8:27b | f5 | fresh | 0, 1, 2 | failed / incomplete / incomplete | failed | 23612 / 2576 | 40.386 | 0 |
| qwen3.8:27b | f6 | fresh | 0, 1, 2 | failed / incomplete / incomplete | failed | 29804 / 3468 | 55.682 | 0 |
| qwen3.8:27b | r1 | reduction_repeat | 0, 1, 2 | failed / incomplete / incomplete | failed | 15262 / 2890 | 42.724 | 0 |
| qwen3.8:27b | r2 | reduction_repeat | 0, 1, 2 | failed / incomplete / incomplete | failed | 11739 / 2030 | 30.973 | 0 |

## Frozen identity and protocol

Member source: `5fbe0ea2608a4ccb5f9eea71852488614ca884ad`. Managed root: `d1b54c92f3fd8c8b7f8cb45605143596d9407199`. The protocol baseline field records the earlier design baseline ce339a5; actual tested source is the integrated member and frozen per-file manifest.

Frozen manifest SHA-256: `38c6874e3dc804373c479b0bf3a13076d8d17e77d4f6e3453612fd6540127870`. Protocol SHA-256: `442d7bde74d00f68ccc78cecd329d776ba7c9918475887f3adc0815f6283c974`. Current read-only verification: 164 frozen files, 0 hash mismatches.

- gemma4:26b: `08ae7ec1744bd7f451c4a530afb39d2673ad9d07a8369b8a33a3613b41212a68`
- qwen3.8:27b: `22130167c4c20e20c7b71454612966ca8e8171e9b3cc8ab6ce8aa6cbfec79643`

Ollama 0.33.0-dabeest through the pre-existing local tunnel. Equal caps: 30 calls/model, 60 total, at most initial plus two repairs per trial, 4096 output tokens, 32768 context tokens, think=false, temperature=0, seed=42. Compact/reduced context caps are 16000/8000 UTF-8 bytes; serialized request cap is 64000 bytes. Only diagnostics are added during repair. Cargo builds are locked and offline.

## Evidence and interpretation

The per-attempt JSON files contain specific structural findings, source hashes, and the executable snapshot observed during review. The aggregate JSON refreshes executable outcomes from actual per-slot result files and verifies every reviewed source hash. Embedded early review snapshots may precede a completed build. The companion attempts.md lists all 59 charged slots with separate execution dimensions and measured costs; aggregate.json also retains full inference metrics and terminal accounting.

Compiler failure leaves behavior and diagnostic tests incomplete; it does not demonstrate that those assertions ran and failed. Malformed/truncated responses are distinguished from compiler failures. Structural defects can be assessed directly in available source even when execution is blocked.

One Gemma reduced-context r1 repair (attempt 2) was consumed when disk space ran out. No response, generated code, tokens, or latency survived. It remains uncertain and structurally unassessed. Its missing cost is not treated as zero. The resumed run retained its charge and did not refund it.

The fixture support type named Resource can collide with the sdax prelude Resource under two wildcard imports. This is an authoring/fixture confound in affected attempts; independent acquisition, binding, lifecycle, and boundary defects remain separately documented. No inference conclusions about runtime library correctness or human reviewability follow from these failed generated programs.

All review operations read saved evidence only. No review builds, inference calls, task hints, ledger edits, or frozen-input changes were performed. Monetary cost was not measured.

## Next authoring hypotheses

These are hypotheses for a subsequent separately authorized revision, not established causal findings or permission to extend this protocol.

- Make the difference between opening inside lazy cx.hold and registering an already-open value through hold_value concrete in the smallest resource example. This matters even for Qwen f2, whose executable assertions pass.
- Put typed input selection, export-before-build, explicit fixture type aliases, and separate acquire/release captures together in one minimal compilable pattern. Several repairs spend their allowance correcting these local Rust/API details while leaving semantic omissions untouched.
- Contrast a known error with a pending body that times out, then show the exact identified effect and recovery argument shapes and independent completed output. The recovery attempts repeatedly substitute known errors for ambiguity.
- Present service initialization retry, serving restart, stable published handle, resident mode, and cooperative stop in one complete declaration. Generated service code often expresses some pieces while missing the mode or conflating initialization with resource retry.
- Keep nested component binding and full Report boundary examples mechanically checkable; reduced-context outputs show invented APIs and report reconstruction, and combined-failure attempts flatten required nested mounts. The two reduction repeats alone cannot establish a general causal effect of context size.

## Retained evidence

The [evidence archive](../evaluation-results/2026-09-09-frozen-wave/evidence.tar.gz) and [file manifest](../evaluation-results/2026-09-09-frozen-wave/manifest.json) preserve the full request/response/diagnostic/review record without generated build directories. Original run and review directories remain retained locally.
