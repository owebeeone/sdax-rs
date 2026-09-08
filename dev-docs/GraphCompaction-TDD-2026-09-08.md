# Dynamic execution graph compaction — 8 September 2026

Base checkpoint: `b0080ce`. Implementation and platform validation are complete.
See the [checkpoint report](GraphCompactionCheckpoint-2026-09-08.md) for measured
tradeoffs and the remaining performance limitations; no blanket pass is implied.

## Contract and representation

The complete host effect/publication batch is the retirement boundary. Active
raw keys stay stable; dense indices are remapped together. Immutable history
stores node identity, declaration, kind, path, terminal state, deadline and
insertion order, plus instance identity/parent/membership. Cause provenance is
stored as raw keys so a retained node can still name an archived cause. All
report and trace records already own path/order values.

The active instance vector excludes archived instances. Template obligation
history is a boolean on its active slot. The static graph remains resident.
Historical readers are preserved; total history/trace memory remains linear.

## Executed test sequence

1. Core churn and live-sibling tests failed to compile because
   `compact_ended_instances` did not exist (`core-red.log`). Implemented history
   separation and coordinated remapping; both then passed (`core-green.log`).
2. The adapter batch-boundary test failed: three execution nodes remained versus
   two expected (`adapter-red.log`). The driver now compacts after consuming all
   effects and publication acknowledgements, after retiring instance contexts.
3. The simulator integration assertion failed: five execution nodes remained
   versus three expected (`simulator-red.log`). Added the same batch boundary.
4. The first full suite reproduced a late cancellation into compacted history:
   run seed 6746427589533237249, case 231, case seed 8893298172542843099.
   `workspace-first.log` preserves the two unknown-node rejections. The simulator
   now mirrors the adapter's retired-body guard before machine dispatch.
   The Monte Carlo suite passed (`monte-carlo-green.log`); the case seed is added
   to its permanent regression list.
5. Additional regression guards cover nested ownership and re-instantiation
   after indices move, nested parent shutdown, cleanup failure preservation,
   active node/scope timer remapping and harmless forgotten timers. These tests
   were added after core implementation and are not claimed as pre-fix REDs.
   All four focused core tests passed (`core-extended.log`).

Evidence directory: `../../artifacts/graph-compaction-20260908/`. All workspaces,
logs and build outputs are retained. No pushes or publication are part of this
change.

6. The first measured candidate reduced retained memory but regressed the
   100-live-child runtime by roughly 40%. `before-*` / `after-*` preserve this
   rejected implementation. A batching test then failed because a single small
   completion immediately remapped all survivors (`batching-red.log`). Batching
   by retired/live node or instance count and in-place adjacency remapping made
   it pass (`batching-green.log`). `batched-*` retains its intermediate results.
7. History paths are moved rather than cloned after all terminal states have
   been captured. A regression guard checks that a skipped child can still name
   a faulting node retired in the same batch. The guard is not a pre-fix RED.
8. A retained-timer-capacity assertion failed at 4 entries versus zero after
   all child timers ended (`timer-capacity-red.log`). Compaction now shrinks
   timer storage and oversized retained lock-holder vectors. All six focused
   core tests pass (`timer-capacity-green.log`).

The batching rule bounds unarchived dynamic node and instance counts by twice
live entries, in addition to the static graph. Empty child graphs are bounded
by the instance-count trigger. It does not bound historical records or trace.

Final validation: all eleven Mac gates; 446 tests/doctests with zero failures
and two ignored on each of Mac, native Windows and Pi; both Rust 1.75 library
builds on Windows and Pi; an additional 10,000-case Monte Carlo walk. The final
source hash and measured tradeoffs are recorded in the checkpoint report.
