# sdax improvement implementation — 8 September 2026

Status: core implementation, native portability checks, independent correctness review and bounded authoring evaluation complete. Mac/Pi/Windows runtime comparisons completed; the Windows comparison resumed after recovery from the serious agent-caused deletion incident. This report records executed evidence only.

Baseline: `bdea94200ae743fc94ea76987cfd4f6927e0ff8d`. The revised semantics tag is `sdax/2`. The normative [v2 contract](SdaxContract-v2.md) defines intentional breaking changes; no compatibility facade is retained. The owner requested a local checkpoint commit after validation. Nothing has been pushed or published.

## Windows incident and recovery

During benchmark workspace preparation, an unsafe nested PowerShell cleanup command expanded the intended repository variable before the inner command ran. The resulting operation deleted contents of `C:\Users\gianni`, outside the authorized workspace. Root is responsible for the delegated action. Windows benchmark work and writes stopped immediately afterward. Previous correctness checks did execute and their logs remain on the Mac; they do not mitigate this incident. The initial Windows current-version runtime comparison was not completed; a fresh comparison was subsequently captured.

The owner authorized recovery from the C: snapshot taken at 11:53:48 AEST. All 152,088 staged files were verified by SHA-256. Restoration copied 140,643 missing files while preserving 11,445 existing files; 109 links were restored and one already matched. Final live verification has two documented cache/log exceptions. Changes between the snapshot and deletion at 16:37 are not covered. Known test tooling was reconstructed. See the [restoration report](../../artifacts/sdax-improvements-20260908/windows-incident/RestorationReport.md). The owner subsequently authorized resuming work; Windows baseline/candidate measurements and a reverse-order repeat now complete the platform comparison.

## Review issue mapping

| Issue | Implemented change | Evidence / remaining gate |
|---|---|---|
| Lost acquisition obligation | Non-cloneable consuming acquisition capability; lazy factory reservation; defensive rejection preserves the original obligation | `ImprovementSafety-TDD.md`; compile-fail and adapter safety tests pass |
| Receipt-less compensation | Typed operation identity and explicit unknown-outcome recovery, distinct from receipt compensation | `safety_recovery.rs`; original uncertainty retained in history and unresolved recovery remains visible |
| Missing terminal silently disappears | Build-time reserved declaration survives dropped/forgotten builders | `incomplete_declarations.rs`: three tests pass |
| Static input and reuse failures | Explicit typed/unit mount input, formal ports, independent mount identities and immutable layouts | `ImprovementComponents-TDD.md`; repeated-mount and run tests pass |
| Service restart changes published handle | Initialize once, restart serving episodes on the stable handle | Integrated; seven stable-service tests pass |
| Unhelpful report formatting | Display includes original cause/source chain, phase and path; existing result conversion keeps the complete typed report | `report_diagnostics.rs` passes |
| Released resource exported as a result | Finite-plan validation follows direct resources/services, component outputs and input/import forwarding | `finite_output.rs`: four tests pass, including forwarding through a mounted child input |
| Input provenance omitted | Separate dataflow inspection with input/import bindings | `dataflow_view.rs` and `dataflow_mounts.rs` pass |
| Callback construction panic escapes normal handling | Invoke user factories inside the guarded poll boundary | Step, try-step and acquisition/recovery tests pass; service factory tests pass |
| Redundant API spellings | Removed positional `_with` shorthand; chain-based authoring is canonical | README, eleven executable guides and AI reference implemented |
| Repeated static setup work | Cache immutable topology/body layout at build; each run retains fresh mutable state | Mac/Pi matched runtime results recorded; dynamic first-write copy remains runtime work |
| Large graph event lookup | Dense per-scope key lookup implemented | Lookup tests pass; 1,000-node graph runtime medians improve 12.6–37.3% across Mac/Pi |

The finite-output check follows declared forwarding; it cannot prove that arbitrary user data or a user-written step contains no cloned live handle. Dynamic children remain owned resident lifecycle units, not a finite fan-out/results API. No durability or exactly-once claim is introduced.

## Integration verification

Safety and component changes were integrated with the root authoring/diagnostic changes. The first combined workspace run exposed three legacy conformance fixtures and one generated child that exported a live resource from a finite scope. The fixtures now declare resident child lifetime; their finite-parent assertions remain. The generator retains its random choice sequence but uses resident mode for a resource output. The affected conformance and Monte Carlo targets then passed.

Cross-feature tests pass. The complete Mac inventory passed all eleven gates after reconciling formatting in two quoted examples and one test-only mutex scope. Final-source Rust 1.75 library builds passed on Pi arm64 and Windows x64 as well. Intermediate agent-checkout results are not substituted for final-source verification.

The fresh independent review reproduced four additional defect families and verified their fixes with eight external tests: RAII destructor ordering, foreign-key export scope, active lifecycle deadlines (including shutdown beginning during retry cleanup), and conflicting blocking pools. Its final suite passed 8/8. Focused repository tests additionally cover declared aliases, concurrent runs, destructor panics, and seven deadline cases. See [review and preserved evidence](../../artifacts/sdax-improvements-20260908/final-review/FinalAdversarialReview.md).

The frozen library source revision is `39bc1174d8a3dc276716adc9bab91fa2a4d0a104adfa9dcb62f7cd81fe4f7d54`. It hashes workspace/crate manifests, lockfile and all crate `src/**/*.rs` files, including new untracked source. The candidate benchmark fixture revision is `ec74f713161f74705c4e7c7a1ca8aa3e954c9cd244cad75bd2eed057b189170a`.

## Performance accounting

A: model/synthetic generation; B: Rust compilation; C: plan build/validation/layout; D: execution, all per-run setup, dynamic instances, cleanup, reports and disposal; E: representative application including its bodies/I/O. These are separate tallies. Generation is never included in D. No per-run mutable state or dynamic expansion is moved outside the runtime timer.

Matched full-disposal comparisons are complete on Mac, Pi and Windows, with raw samples, metadata and reproduction tools in [ImprovementPerformance.md](ImprovementPerformance.md). Runtime medians for 1,000-node chain/wide/sparse graphs improve by 12.6%/27.1%/25.8% on Mac and 19.2%/37.3%/29.6% on Pi. Pi requested allocation bytes for those rows fall 32.8%/44.5%/37.6%; these are allocated-byte totals, not peak live memory.

The gains have visible costs. One-time plan construction is slower at every reported scale (Mac medians +28–141%), because it now prepares cached layouts and additional validation. Its allocation growth is substantial. Cold consumer-harness builds increase 12–14% and executable size 16%, measured once per host/source; revised-only fixture code contributes to the consumer binary, so this is not a library-only size comparison. Mac resident readiness/shutdown regresses 7.6% at median and 20.4% at p95, and some failure-path tails remain noisy. Pi does not reproduce that shutdown regression. These remain flagged rather than being averaged away. No overall performance gate pass is claimed.

Windows fresh baseline/candidate captures show 1,000-node chain/wide/sparse median improvements of 19.8%/28.0%/17.9%; a reverse-order repeat confirms 18.3%/28.1%/18.7%. Dynamic-live-child medians and allocations regress, while service/effect rows vary by capture. Plan construction increases separately (49–131% for the large graphs in the primary capture). See [Windows results](../../artifacts/windows-performance-resume-20260908/WindowsPerformanceResults.md). Generation, compilation and plan building remain outside the runtime tally.

A bounded [disposal investigation](../../artifacts/sdax-improvements-20260908/disposal-investigation/DisposalInvestigation.md) checked an apparent Mac tiny-run regression. Two unchanged full-harness repeats did not reproduce it; exact-fixture instrumentation observed no pending tasks or drain iterations in the tested tiny chains. No runtime or yield-primitive change was justified. The original capture is retained, and the report distinguishes run/order sensitivity from any unproven attribution to CPU frequency or temperature.

## Adoption evidence

The original local-model scenarios are retained, alongside held-out repeated-mount, unknown-outcome recovery, stable-service and cleanup-failure exercises. Each uses external behavioral assertions and a bounded repair budget. The original fixed matrix completed: 1/10 passed (reproduction 1/2, compact 0/6, reduced 0/2). Eight failures were compilation errors; one compiled but hid a resource inside ordinary data and released its parent too early. A live-source compiler failure invalidated one trial; its three calls are excluded from authoring outcomes and retained as infrastructure cost. Remaining trials used an immutable source snapshot.

Valid matrix cost: 28 calls, 160,354 prompt tokens, 21,063 output tokens and 310.09 seconds of server time. Including infrastructure cost: 31 calls, 175,749 prompt tokens, 22,857 output tokens and 334.80 seconds. Full prompts, responses and external assertions are retained in [authoring evidence](../../artifacts/sdax-improvements-20260908/authoring-eval/Results.md).

The failures prompted explicit documentation of automatic Arc wrapping, tuple keys versus tuple dependencies, required output export, resource-port lifetimes and identified-effect ordering. The final bounded exposed-task retest passed 2/3 behavioral checks: the finite pipeline after one repair, recovery after two repairs; composition still failed. It used a 16,554-byte AI-reference-plus-README context. Added cost was eight calls, 49,229 prompt tokens, 6,442 output tokens and 93.52 seconds server time. It is separate from the original matrix and cannot establish generalization. Generated diagnostics still failed to preserve all structured details.

The compact-context adoption gate is **not met**, and the original composition gate remains failed. No small/local-model-readiness claim is made. A final documentation-only clarification now shows the exact declarative retry chain; that last clarification was not retested. We stopped after the declared final retest rather than repeatedly tuning to these exposed tasks. See [post-documentation evidence](../../artifacts/sdax-improvements-20260908/authoring-post-doc-retest/Results.md). The existing 27B quantized model is a comparison point, not evidence of usability with genuinely small models; none smaller is currently installed. All observed failures and token/time costs will be reported separately from runtime measurements.

## Final gates

- Complete shared inventory on Mac: passed all eleven gates again after final performance artifact integration. The standalone optimized benchmark verification also passes.
- Native Pi and Windows complete inventories: passed all eleven gates each. Final-source Rust 1.75 library checks pass on both hosts.
- Matched Mac/Pi/Windows runtime comparisons: completed and integrated; Windows includes a reverse-order repeat. Plan-build, dynamic-child and host-sensitive runtime regressions remain flagged. No complete performance-gate pass is claimed.
- Canonical README/guides: passed. Local-model exercises: completed with a failed adoption gate (original 1/10, exposed-task retest 2/3).
- Fresh independent adversarial review and fixes: passed 8/8 external tests; no reproduced defect remains open in the reviewed scope.

Full inventory logs: [Mac](../../artifacts/sdax-improvements-20260908/final-check-all-mac-after-performance.log), [Pi](../../artifacts/sdax-improvements-20260908/final-check-all-pi.log), [Windows](../../artifacts/sdax-improvements-20260908/final-check-all-windows.log). Final Rust 1.75 library logs: [Pi](../../artifacts/sdax-improvements-20260908/final-msrv-pi.log), [Windows](../../artifacts/sdax-improvements-20260908/final-msrv-windows.log).

## Remaining limits

The API fixes do not imply that all acceptance criteria are met. The bounded local-model adoption gate remains failed. Dynamic child value slots are disposed, but ended-instance topology and history accumulate during churn. Default execution retains a trace; no trace-disabled mode was introduced. Declared lifetime validation and RAII disposal cannot discover or revoke handles hidden in arbitrary application data or retained by caller-owned clones.

The performance report distinguishes observed runtime and allocation results from unmeasured peak live memory and exact wakeups. Its handwritten computation loop is only a lower bound, not a lifecycle-equivalent Tokio implementation. No durability, exactly-once execution or finite dynamic fan-out/results guarantee is added.

Tests and randomized schedule exploration provide regression evidence; they do not enumerate every possible schedule.

## Subsequent dynamic-readiness optimization

After the platform comparison, profiling identified repeated readiness-latch cloning and nested instance lookup in the adapter. The current source revision is now `64ca4858ee2ab919c28df7ff98ebe8c47b11c28124460a483bdea4bc5d24fab2`; earlier sections and their datasets describe the frozen `39bc117` candidate. The new keyed registry answers each readiness latch once and wakes outside its registry lock. Focused tests and all eleven shared Mac checks pass; Windows optimized fixtures and both native Rust 1.75 library builds pass.

Compared with the preceding candidate, Windows 10-child medians improve 7.7–9.5% and 100-child medians 22.5–24.0% in both measurement orders. The original allocation budget is still narrowly missed, one-child timing is mixed, and metadata retention is unchanged. See [the targeted report](../../artifacts/dynamic-profile-20260908/DynamicReadinessResults.md). No adoption gate or overall performance pass is inferred from this fix.
