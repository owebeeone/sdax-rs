# Performance measurement results — 2026-09-09

Status: Pi lifecycle and `ce339a5`/candidate captures and Windows dynamic
allocation confirmation are complete. The isolated component-copy allocation
objective passed. Timing flags remain order-sensitive, so no overall performance
pass is claimed.

## Boundaries and controls

The before source is commit `ce339a59eefcd136562b16f4f20035c29ef0c988`,
crate-source revision
`ee2fa0f90514e6ae168c3d502a3a0aac2ca692d473a0c335daeb4e155ddb3edd`.
The candidate library remains
`3b6229329a8d057979b1d2d580dcfaecc17ef0e6cf3f7f21accbbc87d39e3cee`.
Both Pi full captures and the Windows allocation-only captures used fixture
revision
`7341fedb1370a331c151cbaf066d938e8333c04a886e44714c4ed36138e6193a`,
locked dependencies, the release profile, one current-thread Tokio worker and
full SDAX tracing. Pi used rustc 1.96.0 aarch64; each direction used 40 execution
samples, eight warmups and 20 build samples. The Windows run is
described separately below.

All 212 workload groups were present on both sources and all checksum sets
matched. The baseline supports the current matched workloads because `ce339a5`
is immediately before the isolated component-copy change. No unsupported older
baseline was assigned a fabricated result.

The independent lifecycle checker requires exact output, typed error and source
chain, cleanup error, dependency-order events, lifecycle body starts/completions,
zero active tasks and payload disposal after report drop and task drain. It also
rejects nonempty incomplete or ambiguous report lists. The handwritten side owns
and joins every task. SDAX's full internal trace is real required work inside its
boundary; the comparator does not fabricate the same internal representation.

## Lifecycle-equivalent Tokio comparison

Times are median/p95 in microseconds. The first direction ran SDAX then
handwritten Tokio for each case; the second reversed that order.

| Case | SDAX-first: SDAX | SDAX-first: Tokio | Reverse: SDAX | Reverse: Tokio |
|---|---:|---:|---:|---:|
| normal release | 34.509/35.111 | 3.778/3.852 | 36.963/38.055 | 4.102/4.148 |
| partial startup failure | 36.620/38.203 | 4.389/4.537 | 36.611/40.371 | 4.685/4.815 |
| downstream cleanup failure | 54.991/55.963 | 6.445/6.611 | 55.009/57.055 | 6.398/6.500 |
| cancel after acquisition | 30.046/30.666 | 3.408/3.500 | 29.981/31.111 | 3.444/3.555 |

The median ratios range from 7.81x to 9.13x. These are descriptive ratios, not
a regression gate against a historical lifecycle-equivalent denominator. They
include summary normalization, exact checking, payload disposal and draining on
both sides.

All five allocation captures per case were identical in both orders. Values
below are allocation calls / requested bytes / peak measured-thread net live
bytes; retained net bytes were exactly zero throughout.

| Case | SDAX | Tokio |
|---|---:|---:|
| normal release | 206 / 19,845 / 11,391 | 5 / 856 / 296 |
| partial startup failure | 222 / 21,696 / 11,866 | 14 / 1,180 / 516 |
| downstream cleanup failure | 327 / 27,639 / 13,821 | 16 / 1,743 / 535 |
| cancel after acquisition | 175 / 16,885 / 10,187 | 6 / 688 / 400 |

Peak and retained fields are changes in requested live bytes on the measured
thread relative to the pre-run baseline. They are valid for these current-thread,
nonblocking fixtures. They are not allocations owned by a run, native RSS or a
whole-process peak. SDAX spawned/joined values are lifecycle-body starts and
completions inferred from the full trace, not counts of every driver or joiner
task internal to the adapter.

## Component-copy allocation result

The original resource-bearing plan-build witness remains the direct measurement
witness for the library change: candidate traffic decreases by exactly seven
allocation calls and 1,264 requested bytes per mount at 2/10/100 mounts. The full
paired Pi capture independently exercises the typed repeated-mount fixture and
finds a different, type-specific slope: exactly five calls and 841 requested
bytes removed per mount.

| Typed mounts | Before calls/bytes | Candidate calls/bytes | Change |
|---:|---:|---:|---:|
| 2 | 134 / 19,601 | 124 / 17,919 | -10 / -1,682 |
| 10 | 460 / 93,857 | 410 / 85,447 | -50 / -8,410 |
| 100 | 3,876 / 811,413 | 3,376 / 727,313 | -500 / -84,100 |

These were the only allocation differences across the 212 matched groups.
Graph declaration, validation/freeze, end-to-end build, table setup, body layout
and every execution row had identical allocation calls and requested bytes.

## Pi before/after and sparse tail

Times are median/p95. The forward capture ran baseline then candidate; the
reverse capture ran candidate then baseline.

| Sparse nodes | Forward baseline | Forward candidate | Reverse baseline | Reverse candidate |
|---:|---:|---:|---:|---:|
| 1 | 16.972/17.445 us | 11.148/11.389 us | 11.305/11.611 us | 17.917/18.444 us |
| 10 | 94.000/95.667 us | 62.250/63.111 us | 62.611/64.001 us | 100.296/102.260 us |
| 100 | 0.718/0.727 ms | 0.711/0.719 ms | 0.715/0.722 ms | 0.714/0.720 ms |
| 1,000 | 26.343/27.101 ms | 26.592/27.320 ms | 27.157/27.905 ms | 26.817/27.710 ms |

At one and ten nodes, the first process in each direction is roughly 34–60%
slower and the sign reverses with execution order. The one-node sparse p95
therefore remains unresolved host/order sensitivity and cannot support a source
regression or improvement claim. At 100 and 1,000 nodes both directions remain
within about 1.3%.

The frozen rule defines baseline within-capture spread as nearest-rank p95 minus
nearest-rank p05. Forward order produced one timing flag: sequential churn at ten
instances had a +13.8% p95 shift with an absolute increase above baseline spread.
Its reverse result was 400.835 us baseline versus 400.446 us candidate and did
not reproduce the flag. Reverse order flags many small graph, setup and build
rows because the candidate was the slower first process; forward order reverses
that relationship. These order effects remain visible rather than being averaged
away. Large graph declaration/validation/end-to-end build rows show no
direction-consistent gate failure attributable to the component-copy change.

## Windows allocation-only confirmation

The Windows capture used rustc 1.98.1 on `x86_64-pc-windows-msvc`, the release
profile and locked offline dependencies. The exact baseline, candidate and
fixture revisions were rechecked before building. Each source ran `bench` with
zero timing samples, eight warmups and zero build timing samples. The forward
order was baseline then candidate; the reverse order was candidate then
baseline. Each run emitted 82 allocation rows, and every row repeated exactly
when its source ran in the opposite order.

Dynamic live-instance execution allocations were identical across source and
order:

| Instances | Baseline calls/bytes | Candidate calls/bytes |
|---:|---:|---:|
| 1 | 615 / 63,287 | 615 / 63,287 |
| 10 | 4,382 / 471,069 | 4,382 / 471,069 |
| 100 | 68,263 / 5,172,799 | 68,263 / 5,172,799 |

The only three differences across all 82 rows were the typed repeated-component
plan builds. Their 2/10/100 values exactly match the Pi table above, including
the minus-five-call and minus-841-byte per-mount slope. This confirms that the
isolated clone removal introduces no dynamic-execution allocation growth on the
Windows host. The zero peak and retained fields in these general harness rows
mean **NOT MEASURED**; they are not zero-live-memory results. Only the dedicated
lifecycle profile measures and interprets measured-thread live bytes. No Windows
timing data was collected, and results from its rustc 1.98.1 host are not
compared as ratios with the Pi rustc 1.96.0 host.

## TDD and verification

The RED compile on Pi retained E0425 for the absent allocation `profile` control
and E0609 for the absent structured error-source field. GREEN adds a control that
distinguishes a released 4 KiB buffer from one retained across the profile end,
plus negative lifecycle checks for output, event order, source chain, typed
error, cleanup details, disposal, joins and active tasks.

- Harness unit tests: 2 passed.
- Optimized fixture verification: passed every existing fixture and both
  lifecycle implementations.
- The measured fixture revision `7341fedb...` was found after capture to predate
  an explicit standalone-harness formatting check. The claim that formatting had
  passed at that revision was stale. Formatting only the seven harness source
  files produces fixture revision
  `8017d9ad4695a10d53c59e26922bb40115fa49e60c5ccfbf23418f4e64b19164`.
  On Pi, that exact final revision passed rustfmt 1.96, all 42 Python script tests,
  both harness unit tests and optimized fixture verification. The measured and
  committed pre-format source are identical; no measured logic was changed or
  silently substituted.
- Clippy 1.96: passed with the harness's existing scoped
  `clippy::io_other_error` allowance; the new raw Tokio spawn retains its local
  ownership rationale and allowance.
- The Rust 1.75 harness command cannot parse the existing v4 harness lockfile.
  No toolchain or lockfile workaround was attempted. The library source did not
  change in this step; prior native Rust 1.75 library validation of revision
  `3b622...` remains applicable.

Raw rows, summaries, host-load boundaries and stderr are retained under
`performance-results/pi-remediation-paired-20260909/`. Pi build outputs and the
RED/GREEN working source remain under
`/home/gianni/sdax-performance-lifecycle-20260909-0358-red`; paired source and
capture directories remain under
`/home/gianni/sdax-performance-baseline-ce339a5-20260909` and
`/home/gianni/sdax-performance-pi-paired-20260909`. The final formatting control
is retained under `/home/gianni/sdax-performance-format-verify-20260909`.
Windows raw CSV, build logs, source hashes, process snapshots and analysis are
retained under
`performance-results/windows-remediation-allocation-20260909/` and remotely at
`C:/Users/gianni/git/sdax-remediation-alloc-stage-20260909`. No timings ran on
the local or Windows hosts during inference, and the Windows follow-up collected
allocations only after inference completed.
