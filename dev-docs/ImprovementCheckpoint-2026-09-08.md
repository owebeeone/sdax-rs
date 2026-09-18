# Improvement checkpoint — 8 September 2026

Evidence links below point to private campaign archives and require repository access. Historical commands and recorded paths describe the original runs; see the archive README for replay setup.

Latest continuation: [dynamic graph compaction](GraphCompactionCheckpoint-2026-09-08.md).
The earlier measurements and status statements below apply to their named revisions.

This is a validated implementation checkpoint, not a release or a claim that every improvement gate passed. It contains the breaking sdax/2 API and lifecycle fixes, explicit component bindings, immutable layouts, diagnostics, executable documentation, performance harness and the subsequent dynamic-readiness optimization.

Earlier committed source revision: `64ca4858ee2ab919c28df7ff98ebe8c47b11c28124460a483bdea4bc5d24fab2`. All eleven shared Mac checks pass. Native Windows full benchmark fixture verification and both Rust 1.75 library builds pass. Earlier broad portability inventories passed on Pi and Windows before the readiness optimization; they are not represented as reruns of the new revision.

The current context-retirement continuation has source revision
`d55bf2350247570f6e19a7de253db50c2153af36615502aba487b98df51f4f86`.
It meets the ten-child allocation threshold at 4,281 calls versus 4,361 and
reduces retained requested memory by 8.7–12.1% in the measured churn fixtures.
Total history remains linear. See the [retention checkpoint](MetadataRetentionCheckpoint-2026-09-08.md)
for contracts, evidence and platform limitations.

## Retained benchmark evidence

- [Windows baseline and candidate resumption](https://github.com/owebeeone/sdax-core-evidence/tree/main/campaigns/performance/performance-results/resumed-windows-39bc117) contains complete raw samples, summaries, build logs and metadata for the pre-readiness-fix candidate.
- [Windows dynamic-readiness before/after](https://github.com/owebeeone/sdax-core-evidence/tree/main/campaigns/performance/performance-results/dynamic-readiness-windows-64ca485) contains both measurement orders, fixture verification and MSRV logs. The before captures use `39bc117`; after uses `64ca485` with the same full benchmark fixture.
- Existing Mac/Pi results remain under `performance-results/`. Additional native profiling, recovery, authoring and full-check artifacts remain in the sibling workspace `artifacts/` directory referenced by the implementation report. Recovery profile data is retained on Windows, not included in Git.

Generation, compilation and one-time plan construction remain separate from runtime. Runtime includes per-run setup, dynamic expansion, cleanup, reports and disposal.

## What follows this checkpoint

1. Bound metadata growth during long-running child churn. The first context-retirement change and measurements are complete; next separate lightweight historical records from the active execution graph. Define identity, late-event and diagnostic invariants before implementing reclamation. Preserve cancellation, cleanup ordering and nested-instance ownership.
2. Address failed authoring patterns: component inputs versus resource imports, automatic Arc wrapping, explicit output export, acquisition retry and complete diagnostics. Use the exposed tasks as regression cases, then a fixed-budget held-out evaluation. The existing local-model adoption gate is still failed.
3. Reduce plan-build allocations separately and investigate remaining host-sensitive timing flags. Dynamic readiness improves 10-child Windows medians by 7.7–9.5% and 100-child medians by 22.5–24.0% versus the preceding candidate, and the subsequent context-retirement checkpoint meets the original ten-child allocation target: 4,281 calls versus 4,361. Physical graph reclamation remains outstanding.

No blanket performance pass, small-model usability claim, publication or push is implied by this checkpoint.
