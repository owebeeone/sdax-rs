# Execution-context retention checkpoint — 8 September 2026

This change releases adapter execution contexts after dynamic instances end and
removes the allocating historical-instance snapshot from each driver step.
Historical inspection remains available. Total retained memory is still linear
in completed instances; physical active-graph compaction is a separate change.

Performance source revision:
`d55bf2350247570f6e19a7de253db50c2153af36615502aba487b98df51f4f86`.
Computed with `scripts/performance-source-revision.py`; this hashes crate source
and manifests, including in-source unit tests, independently of Git commit IDs.
The preceding committed checkpoint is `f97a719` (source revision `64ca4858ee2ab919c28df7ff98ebe8c47b11c28124460a483bdea4bc5d24fab2`).

## Contract and implementation

On `TraceKind::InstanceEnded`, the adapter removes `Live` entries whose origin
belongs to that exact `InstanceId`, then closes the child scope and body-source
slots. Other instance IDs, including separately spawned nested instances, are
unaffected. Caller-owned or budget-abandoned work may finish later; its body
message fails the existing live-entry/epoch guard before value storage or
`Machine::step`. Retirement does not claim to cancel that external work.

`Snapshot.contexts` counts distinct run keys that have received a body context.
It remains cumulative after retirement, and retries on a run key do not add to
it. Origins, kinds, paths, trace and machine history remain retained.

`Machine::active_instance_states()` borrows non-ended states without allocating
the vector returned by `instances()`. Readiness resolution consumes that iterator.
It still scans historical instance records. Retirement itself scans the adapter's
active context map; this checkpoint makes no asymptotic execution-time claim.

## Regression evidence

- The core simulator test compares active states to the non-ended subset of
  historical states at every step, observes an ended child alongside a live
  sibling, and retains both identities after clean shutdown.
- Adapter tests check context release, ignoring a late same-epoch message,
  sibling isolation, retry counting and cumulative counting after retirement.
  These are regression guards; nested ownership is not separately exercised by
  the new adapter fixture.
- The core test has an executed compile-time RED because the iterator did not
  exist, followed by GREEN. Its log is in
  `../../artifacts/metadata-churn-20260908-core/red.log`.
- No adapter pre-fix RED was executed. The corrected record is
  `../../artifacts/metadata-churn-20260908-adapter/retention-tests-log.md`.
  Do not reinterpret an expected failure as observed test-first evidence.

All eleven shared Mac checks passed on this source before the documentation
continuation: lockfile, format, clippy, workspace tests, architecture,
compile-fail, guide quotations, rustdoc, script tests, consumers and archives.
Evidence: `../../artifacts/metadata-churn-20260908-validation/mac-checks/results.json`.

Native Windows targeted regressions, the first nine shared gates and both Rust
1.75 library builds passed. The full inventory stopped at consumer vendoring:
the isolated Cargo home lacked the registry cache. Windows consumer/archive
completion is not claimed. The same consumer gate passed on the Mac.

Native Raspberry Pi (Debian arm64) validation passed on the exact source hash:
locked offline metadata, formatting, clippy for all workspace targets with
warnings denied, all workspace tests, and rustdoc with warnings denied under
Rust 1.96.0. Both libraries also built offline with Rust 1.75. This continuation
ran the runtime-relevant subset, not a new full eleven-gate Pi inventory.
The fresh workspace and outputs remain at
`/home/gianni/git/sdax-retention-pi-20260908-continuation` on `10.1.1.236`.
Local logs and per-check result files are retained under
`../../artifacts/metadata-retention-continuation-20260908/` in `validation/`,
`msrv/` and `pi-validation.log`. The final local `git diff --check` passed;
documentation edits leave the performance source hash unchanged.

## Retained requested memory

The Windows allocator diagnostic measures currently live requested bytes while
the parent remains resident, after an observer has received every child-end
event and before root shutdown. It includes engine history, adapter, trace and
runtime allocations; it is neither RSS nor an engine-only measurement.

| Completed children | Before bytes | After bytes | Reduction |
|---:|---:|---:|---:|
| 10 | 84,717 | 77,317 | 8.7% |
| 100 | 762,765 | 688,229 | 9.8% |
| 1,000 | 6,829,517 | 6,005,221 | 12.1% |

Both measurement orders produced the same byte counts. Trace counts were
unchanged at 129, 1,209 and 12,009 events. Evidence:
`../../artifacts/metadata-churn-20260908-validation/memory.log`.

## Windows performance

The frozen full fixture used 40 samples and 8 warmups in alternating order:
before, after, after, before. Generation and one-time plan construction remain
separate from runtime execution. Runtime includes per-run setup and disposal.

| Runtime workload | Before calls | After calls | Before bytes | After bytes |
|---|---:|---:|---:|---:|
| 10 live children | 4,384 | 4,281 | 421,871 | 411,887 |
| 100 live children | 68,743 | 67,740 | 5,486,899 | 4,594,915 |
| 10 sequential children | 3,398 | 3,303 | 312,367 | 301,599 |
| 100 sequential children | 31,800 | 30,892 | 3,397,895 | 2,641,527 |

The original ten-child allocation threshold is met: 4,281 < 4,361.
The 100-sequential-child runtime median improved from 1.930–1.943 ms to
1.803–1.830 ms (about 5.8–7.2%); p95 also improved. Ten sequential children
moved from 169.8–171.7 µs to 172.0–172.05 µs, within the 5% median trigger.
The 100-live-child result was order-sensitive: after 4.062–4.173 ms versus
before 4.143–4.158 ms. One-child and unrelated microbenchmarks are too noisy
to attribute confidently. This is not a blanket performance pass.

The remote evidence remains at
`C:/Users/gianni/git/sdax-retention-20260908-205225/` on `dabeest`.
A copy of `retention-results.zip`, including raw samples and summaries, is
retained at `../../artifacts/metadata-retention-continuation-20260908/`.
The original measurement scripts and logs remain in
`../../artifacts/metadata-churn-20260908-validation/`.

## Remaining linear history and next phase

The sequential core diagnostic retained 10, 100 and 1,000 ended instance
records, with 13, 103 and 1,003 table nodes, while zero children remained live.
The three static nodes explain the constant offset. Releasing contexts does
not reclaim engine tables, instance records, adapter metadata or full traces.

Physical compaction must preserve stable raw keys and instance IDs, late-event
handling, duplicate-ID rejection, idempotent stop, historical readers, reports,
nested ownership and parent-release dependencies. Begin with executed failing
contract tests. Rebuild/remap dense active execution rows and all references
together, keeping lightweight historical records and identity tombstones.
Only compact after the host has consumed the complete effect batch and retired
adapter contexts and source slots. Removing dense rows individually is unsafe.
