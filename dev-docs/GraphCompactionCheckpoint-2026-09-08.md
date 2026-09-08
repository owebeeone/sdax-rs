# Dynamic graph compaction checkpoint — 8 September 2026

Implemented after `b0080ce`; source revision
`ee2fa0f90514e6ae168c3d502a3a0aac2ca692d473a0c335daeb4e155ddb3edd`.
This is execution-graph reclamation, not bounded total history or a blanket
performance/adoption pass. The source, tests and documents form this checkpoint.

## Result and contract

The Tokio driver and simulator compact only after consuming the full effects
batch and all structural publication acknowledgements. Instance contexts and
body-source slots are retired before graph reclamation. Reclamation is batched:
retired nodes or instances must reach half of the corresponding dynamic
execution entries. After the boundary, unarchived dynamic nodes and instances
are bounded by twice live entries, plus the static graph. With no live children,
only static execution nodes remain.

Dense nodes/scopes, state slots, grants, dependency edges, import/template maps,
run-key indexes, ranks and timers are remapped together. Existing active raw
keys never change. Historical node state/path/kind/origin, instance membership,
parent identity and enumeration order survive in immutable records. Stable-key
fault causes preserve skipped-node diagnostics; reports and traces retain their
owned paths/order. Duplicate instance IDs are rejected, ended-instance stop is
idempotent while the root is active, and late body messages are discarded before
storage or dispatch. Direct machine events for retired nodes are rejected.
Forgotten timers remain harmless.

The pure sequential diagnostic measured 10, 100 and 1,000 completed children
while the parent remained resident: **three execution nodes at every scale**,
zero live children, and all 10/100/1,000 historical instance identities and nodes
still readable. Its final report was clean at every scale. This distinguishes
`execution_node_count()` from historical `nodes()` enumeration.

See [contract](SdaxContract-v2.md#dynamic-execution-graph-retirement) and
[TDD record](GraphCompaction-TDD-2026-09-08.md). Tests cover repeated churn,
moving siblings, nested ownership and re-instantiation, parent-release gates,
late messages, ID reuse, stop idempotence, timer remapping/capacity, cleanup
failure preservation, and a skipped cause archived in the same batch.

## Validation

- All eleven shared Mac checks passed on the source revision above, including
  446 Rust tests/doctests passing, zero failures, and two ignored tests.
- Native Windows x64 and Raspberry Pi arm64 each passed locked offline metadata,
  formatting, clippy for all workspace targets with warnings denied, all
  workspace tests, and rustdoc with warnings denied. Both libraries built with
  Rust 1.75 on each host. These are runtime-relevant subsets, not new full
  eleven-gate native inventories.
- The original simulator failure is preserved: run seed 6746427589533237249,
  case 231 / case seed 8893298172542843099. The fixed case is now in the permanent
  regression list. An additional 10,000-case walk passed
  (`monte-carlo-10000.log`). Broader deterministic/random testing is regression evidence,
  not exhaustive schedule exploration.

All evidence is retained under `../../artifacts/graph-compaction-20260908/`:
`mac-verified-checks/`, `pi-verified-checks/`, `windows-verified-checks/`,
`core-probe/`, and the named RED/GREEN logs in the TDD record. Remote workspaces:

- Pi: `/home/gianni/git/sdax-compaction-pi-verified-20260908`.
- Windows: `C:/Users/gianni/git/sdax-compaction-verified-20260908`.

Earlier candidate workspaces, binaries, logs and archives remain available.
No cleanup, push or publication was performed.

## Retained requested memory on Mac arm64

The diagnostic measures currently live requested allocator bytes while the
parent remains resident, after every child-end observation and before root
shutdown. It includes engine history, adapter, runtime and trace. It is not RSS,
peak memory or an engine-only measurement. The baseline is `b0080ce`, whose
source hash is `d55bf2350247570f6e19a7de253db50c2153af36615502aba487b98df51f4f86`.

| Completed children | Before bytes | After bytes | Reduction |
|---:|---:|---:|---:|
| 10 | 78,405 | 53,927 | 31.2% |
| 100 | 689,317 | 475,433 | 31.0% |
| 1,000 | 6,006,309 | 4,332,797 | 27.9% |

Both measurement orders produced identical byte counts. Trace counts remained
129, 1,209 and 12,009. Evidence: `verified-memory.log`.

## Runtime and allocation tradeoffs

The full frozen benchmark source was unchanged. An additional independent
memory-probe binary was declared in both artifact manifests; their identical
harness revision is
`11d624e9b9831724e6bc4256602a6f2c3b5111753c03c86c9c6dd703bebbbfc7`.
Generation, Rust compilation and one-time plan building remain separate from
runtime. Per-run setup, dynamic creation, cleanup, reports and disposal remain
inside runtime. Release profile uses one codegen unit and thin LTO, with a
current-thread Tokio runtime. No model inference ran during measurements.

Primary measurements used 40 samples / 8 warmups, in before-after-after-before
order (`verified-*`). Tiny-workload flags prompted an unchanged-binary repeat
with 80 samples / 16 warmups in after-before-before-after order (`repeat-*`).
The repeat's runtime medians are below; ranges show its two captures per source.

| Workload | Before median | After median | Matched change |
|---|---:|---:|---:|
| 10 live children | 195.750–196.792 µs | 203.187–207.354 µs | +3.8–5.4% |
| 100 live children | 3.829–3.834 ms | 3.871–3.894 ms | +1.1–1.6% |
| 10 sequential children | 152.166–152.854 µs | 154.250–157.167 µs | +1.4–2.8% |
| 100 sequential children | 1.583–1.610 ms | 1.469–1.504 ms | −6.6–7.2% |

The primary 100-sequential-child medians also improved, by 4.3–6.7%, with better
p95. Its primary live-child rows stayed within the 5% median trigger. The repeat
had no runtime workload with a >5% median regression in both orders, although
one ten-live-child capture exceeded that trigger. The primary one-child churn
penalty of about 3 µs did not repeat at that magnitude: the longer repeat showed
roughly 0.5–0.9 µs (2.1–3.8%). A one-node sparse-graph p95 flag remained in the
repeat; attribution is unresolved. All original flagged captures are retained
in `verified-trigger-review.json` and `repeat-trigger-review.json`; no overall
performance-gate pass is claimed.

Reclaiming retained memory costs additional allocation traffic:

| Runtime workload | Before calls | After calls | Before bytes | After bytes |
|---|---:|---:|---:|---:|
| 10 live children | 4,373 | 4,474 | 419,607 | 478,789 |
| 100 live children | 68,552 | 69,075 | 4,665,179 | 5,243,063 |
| 10 sequential children | 3,385 | 3,577 | 308,503 | 331,323 |
| 100 sequential children | 31,604 | 33,694 | 2,703,103 | 2,993,847 |

These are Mac figures; the earlier Windows 4,361-call threshold was met by
`b0080ce`, not re-established for this revision. Creating historical records and
rebuilding compact storage is a deliberate memory/throughput tradeoff, not a
claim to reduce total allocated bytes. The first eager-compaction candidate
caused a roughly 40% 100-live-child runtime regression and was replaced with
batching and in-place remapping. Its source/results are retained as evidence.

## Remaining mission

Total memory still grows with historical records, adapter metadata and the full
trace. This checkpoint bounds active execution retention; it does not introduce
history pruning, trace-off mode, durability or typed finite child results.

The broader remediation plan remains active: inexpensive-model authoring and
composition gates are still failed; one-time plan-build allocation work,
host-sensitive performance investigation and lifecycle-equivalent Tokio
comparisons remain outstanding. Runtime improvement does not satisfy adoption
criteria. The next distinct workstream is the recorded authoring failures and
held-out validation with a fixed inference/repair budget.
