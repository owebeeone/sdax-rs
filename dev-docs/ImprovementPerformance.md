# sdax improvement performance results

Evidence links below point to private campaign archives and require repository access. Historical commands and recorded paths describe the original runs; see the archive README for replay setup.

Status: matched Mac/Pi results are complete; Windows current measurements are incomplete after an agent-caused deletion incident. No overall performance pass is claimed. See [matched results](#matched-drained-baseline-and-frozen-candidate) and the [incident record](WindowsPerformanceIncident-2026-09-08.md). The initial sections below preserve historical v1 baselines, which did not explicitly drain tracked tasks after each report.

Date: 8 September 2026. Baseline commit:
`bdea94200ae743fc94ea76987cfd4f6927e0ff8d`.

The baseline crate-source revision is
`cfd2b4f5d82a1940ae567b89492ffe768901e843c77eb332107eceac318aec1d` and
the fixture revision is
`6e3a4023e05b0e291bf59f0197a16c61e9d1f67af52b1a21603449f4192cc22b`.
The frozen fixture archive is
[`performance-baseline-fixtures.tar.gz`](https://github.com/owebeeone/sdax-core-evidence/blob/main/campaigns/performance/inputs/performance-baseline-fixtures.tar.gz),
SHA-256
`13099e59d2724637ef26a9601ff77630e23dbd23399642cf2b2fe009d5670251`.

This is the M0 measurement baseline for the sdax improvement work. It is a
performance experiment, not a deterministic unit test and not a statement of
an absolute service-level objective. The raw samples are retained so later
revisions can be compared on the same host and fixture revision.

## Accounting boundaries

The harness keeps the five tallies separate.

| Tally | Harness boundary |
|---|---|
| A. Authoring/generation | Synthetic graph names and parent-index vectors only. This always ends before plan declaration begins and is never in an engine sample. No model generation was performed. |
| B. Rust compilation | A release build of the standalone harness and its local path dependencies in a fresh target directory, followed by a no-change build in the same directory. |
| C. Plan building | `Plan::with_input`, all declarations and body factories, validation, and production of the immutable `Plan`. The resulting plan is dropped inside the sample. |
| D. Engine execution | Starts immediately before `plan.start(rt, input)`, so input binding, `Machine` and body-state construction, admission, task dispatch, cleanup, report production, output/report observation, and per-run disposal are included. It ends only after the report is observed and dropped. Runtime and immutable plan construction are outside. Dynamic creation and instance cleanup are inside. |
| E. Representative application | The same complete D boundary with 64 integer mixing operations in every application body. A computation-only handwritten loop is shown as a lower bound and is not called lifecycle-equivalent. |

The isolated `machine_state_setup` and `body_state_setup` rows decompose work
already included in D. `pure_machine_events_preseeded` constructs the mutable
machine before its timer, then measures `begin`, every event/effect transition,
report creation and observation. It is a diagnostic microbenchmark and is not
the primary engine number.

The fixture inventory and baseline support classifications are frozen in
[`performance-harness/fixtures.csv`](https://github.com/owebeeone/sdax-core-evidence/blob/main/campaigns/performance/performance-harness/fixtures.csv).
Unsupported revised-only behavior has no fabricated timing. In particular,
the baseline cannot bind typed static component inputs, mount one definition
more than once, perform callable unknown-outcome recovery, or preserve one
published handle across service recovery.

## Method

The captured run used optimized code (`release`, one codegen unit, thin LTO),
a Tokio current-thread runtime, one worker, eight warm-up runs, 40 timed runs
per execution workload and 20 timed plan builds. Workloads execute in a fixed
order. Time is `std::time::Instant`, a real monotonic clock. Inputs are scalar
values prepared before each call and bound after the engine timer begins.
Outputs and report fields feed a checksum so the work stays observable.

The allocation pass is separate from timing. A harness-only global allocator
counts allocation and reallocation calls and requested bytes while the
measured operation runs. The current-thread runtime keeps engine allocations
on the counted thread. Counts include all per-run state, tasks, trace/report
construction, dynamic slot tables and input binding. Reallocation is one
allocation event and its new requested size is counted; freed bytes and peak
live bytes are not inferred. No allocator or benchmark dependency enters a
library dependency path.

Graph sizes count executable body nodes and exclude the input binding. Each
graph has `N - 1` explicit edges. Chain node `i` needs `i - 1`; wide nodes all
need node 0; sparse node `i` needs `(i - 1) / 2`. The wide fixture therefore
measures fan-out and synchronized completion but has no artificial final join.
The dynamic fixture retains 1, 10 or 100 simultaneously ready resident child
instances, each with three finite bodies and one service, until parent
shutdown.

Fixture verification runs before timing. It asserts successful graph outputs,
startup-failure cleanup, known-receipt compensation, retry success, dynamic
shutdown with zero tracked tasks, and cancellation after acquisition with one
release. Its cleanup-error fixture makes the downstream release fail and
asserts that the upstream release still runs exactly once. These checks are
correctness guards for the measured fixtures; they are not substitutes for the
repository conformance suite.

## Captured host and build

The baseline was captured on `weftpi`, Linux arm64, kernel
`6.12.62+rpt-rpi-2712`, four Cortex-A76 cores with a 2.4 GHz maximum. The
benchmark runtime used one current-thread worker. Rust and Cargo were 1.96.0.
The exact metadata and CPU inventory are in
[`metadata.txt`](https://github.com/owebeeone/sdax-core-evidence/blob/main/campaigns/performance/performance-results/baseline-pi-arm64-bdea942/metadata.txt)
and [`cpu.txt`](https://github.com/owebeeone/sdax-core-evidence/blob/main/campaigns/performance/performance-results/baseline-pi-arm64-bdea942/cpu.txt).

The clean-target optimized build took 22.920 s. The no-change warm build took
34.060 ms. The resulting harness executable was 1,502,248 bytes. Registry
sources were already present for the required offline build; the fresh target
contained no compiled artifacts. Compilation includes the benchmark binary,
`sdax`, `sdax-tokio`, Tokio and their locked transitive packages, so it is a
consumer-plus-dependencies figure rather than library-only compilation.

## Baseline results

Times below are median / nearest-rank p95 over 40 samples. The complete
distribution, min/max, allocations and bytes are in
[`summary.csv`](https://github.com/owebeeone/sdax-core-evidence/blob/main/campaigns/performance/performance-results/baseline-pi-arm64-bdea942/summary.csv),
with every observation in
[`raw-samples.csv`](https://github.com/owebeeone/sdax-core-evidence/blob/main/campaigns/performance/performance-results/baseline-pi-arm64-bdea942/raw-samples.csv).

### Graph scaling

| Shape | Nodes / edges | Plan build | Full execution | Allocations / requested bytes |
|---|---:|---:|---:|---:|
| chain | 1 / 0 | 1.09 / 2.06 µs | 11.60 / 11.91 µs | 110 / 15.1 KiB |
| chain | 10 / 9 | 4.52 / 5.76 µs | 71.44 / 72.93 µs | 654 / 64.5 KiB |
| chain | 100 / 99 | 62.56 / 67.41 µs | 0.893 / 0.915 ms | 5,983 / 751 KiB |
| chain | 1,000 / 999 | 3.578 / 3.615 ms | 44.016 / 45.149 ms | 59,102 / 27.63 MiB |
| wide | 1 / 0 | 1.08 / 1.20 µs | 11.65 / 11.85 µs | 110 / 15.1 KiB |
| wide | 10 / 9 | 4.50 / 4.87 µs | 67.99 / 70.61 µs | 631 / 66.3 KiB |
| wide | 100 / 99 | 64.57 / 65.83 µs | 0.774 / 0.784 ms | 5,618 / 706 KiB |
| wide | 1,000 / 999 | 3.644 / 3.672 ms | 29.658 / 30.994 ms | 55,207 / 20.31 MiB |
| sparse | 1 / 0 | 1.04 / 1.15 µs | 11.60 / 12.17 µs | 110 / 15.1 KiB |
| sparse | 10 / 9 | 4.68 / 5.22 µs | 69.34 / 71.78 µs | 642 / 65.3 KiB |
| sparse | 100 / 99 | 64.09 / 66.67 µs | 0.832 / 0.839 ms | 5,838 / 731 KiB |
| sparse | 1,000 / 999 | 3.698 / 3.717 ms | 36.356 / 37.644 ms | 57,643 / 24.05 MiB |

Allocation-call growth is close to linear, but requested bytes and latency are
not. From 100 to 1,000 nodes, chain latency grows 49.4 times and requested
bytes 37.7 times; wide grows 38.3 and 29.4 times; sparse grows 43.7 and 34.5
times. A linear tenfold increase would be ten times. This is the baseline
scaling problem future layout and queue work should target.

The 1,000-node chain's pre-seeded pure-machine event path is already
30.046 / 30.438 ms. Mutable `Machine::with_input` setup is only
0.603 / 0.754 ms for that chain, and body-state construction is
11.68 / 19.54 µs. For 1,000 wide nodes, machine setup rises to
1.354 / 1.420 ms while body-state construction remains
13.04 / 17.91 µs. The data therefore supports profiling repeated admission,
state transition and release-queue graph work first. It does not support
blaming synthetic generation, which is 0.163 ms for a 1,000-node chain and is
outside D in every case.

### Lifecycle paths

| Workload | Median / p95 | Allocations / requested bytes |
|---|---:|---:|
| one immediate finite body | 11.60 / 11.91 µs | 110 / 15.1 KiB |
| one body that yields once | 12.00 / 12.45 µs | 110 / 15.1 KiB |
| one resource plus consumer and normal release | 23.03 / 23.48 µs | 219 / 22.5 KiB |
| release failure | 24.57 / 24.80 µs | 229 / 23.2 KiB |
| partial startup failure with upstream release | 24.15 / 24.98 µs | 226 / 23.5 KiB |
| cleanup failure followed by upstream release | 36.16 / 36.57 µs | 337 / 29.4 KiB |
| cancel after acquisition and release | 18.57 / 19.83 µs | allocation pass not captured |
| known receipt and compensation | 16.83 / 17.07 µs | 160 / 18.0 KiB |
| one failed attempt then retry success | 17.64 / 18.13 µs | 165 / 18.4 KiB |
| resident ready then shutdown | 17.39 / 17.91 µs | allocation pass not captured |

Normal cancellation and failure-path work remains inside the timer through
cleanup and report disposal. Allocation samples for cancellation and resident
shutdown are intentionally marked unmeasured rather than inferred from nearby
rows.

### Components, dynamic instances and observation

| Workload | Median / p95 | Allocations / requested bytes |
|---|---:|---:|
| flat 10-node chain | 70.65 / 71.52 µs | 654 / 64.5 KiB |
| nested component with 10 inner bodies | 102.45 / 103.65 µs | 983 / 80.4 KiB |
| flat 100-node chain | 0.910 / 0.920 ms | 5,983 / 751 KiB |
| nested component with 100 inner bodies | 1.026 / 1.032 ms | 7,482 / 819 KiB |
| 1 live dynamic instance | 63.29 / 64.33 µs | 604 / 54.7 KiB |
| 10 live dynamic instances | 0.490 / 0.495 ms | 4,361 / 391 KiB |
| 100 live dynamic instances | 10.716 / 10.845 ms | 68,512 / 5.95 MiB |
| 100-node default observer | 0.912 / 0.928 ms | 5,983 / 751 KiB |
| 100-node counting observer | 0.914 / 0.956 ms | 5,983 / 751 KiB |

The 100-instance result is 21.9 times the 10-instance median for ten times as
many instances, and allocation calls grow 15.7 times. Dynamic table lookup,
containment cleanup and reclamation therefore need profiling after the static
event path.

The counting observer changes the 100-node median by +0.32% and p95 by +2.92%
in this capture, with the same allocation count. This is below the proposed
regression gates and does not establish a stable callback cost at this
resolution. The default driver already builds its internal trace and report,
so this comparison measures delivery to a minimal observer rather than
trace-disabled versus trace-enabled storage.

The CPU application pipeline is 71.33 / 72.15 µs for 10 bodies and
0.924 / 0.929 ms for 100 bodies. The computation-only handwritten lower bounds
are 1.24 µs and 13.32 µs. Those ratios include sdax's lifecycle semantics and
per-body task dispatch while the lower bound contains none of that policy; the
numbers are intentionally presented together without calling them equivalent.

## Windows x64 baseline

Windows was captured independently on `DABEEST`, Windows 11 Pro build 26200,
an Intel Core Ultra 9 275HX with 24 physical/logical cores and 128 GiB of
memory. The benchmark still used one current-thread worker. Rust and Cargo were
1.98.1. Exact machine records and all samples are under
[`baseline-windows-x64-bdea942`](https://github.com/owebeeone/sdax-core-evidence/tree/main/campaigns/performance/performance-results/baseline-windows-x64-bdea942).
No model inference or concurrent compiler workload ran during the capture.

The clean-target optimized build took 11.302 s, the no-change warm build
53.059 ms, and the executable was 1,148,928 bytes. Key median / p95 results are:

| Workload | Windows x64 | Pi arm64 |
|---|---:|---:|
| chain 1 | 5.95 / 7.10 µs | 11.60 / 11.91 µs |
| chain 100 | 0.494 / 0.566 ms | 0.893 / 0.915 ms |
| chain 1,000 | 12.764 / 12.990 ms | 44.016 / 45.149 ms |
| wide 1,000 | 10.016 / 10.366 ms | 29.658 / 30.994 ms |
| sparse 1,000 | 11.076 / 11.817 ms | 36.356 / 37.644 ms |
| pre-seeded machine chain 1,000 | 6.808 / 6.975 ms | 30.046 / 30.438 ms |
| 1 live dynamic instance | 28.65 / 30.20 µs | 63.29 / 64.33 µs |
| 10 live dynamic instances | 0.221 / 0.238 ms | 0.490 / 0.495 ms |
| 100 live dynamic instances | 5.116 / 5.409 ms | 10.716 / 10.845 ms |

Hardware rows are never averaged. Windows independently shows the same
superlinear shape: chain execution grows 25.9 times from 100 to 1,000 nodes,
and dynamic execution grows 23.2 times from 10 to 100 instances. Its
pre-seeded machine accounts for 53% of the 1,000-node chain median, again
making state-transition graph work a direct profiling target. Absolute ratios
between Windows and Pi mix hardware, OS and compiler differences and are not
used as sdax regressions.

## Mac arm64 baseline

Mac was captured independently on `Giannis-MacBook-Pro.local`, macOS/Darwin
25.6.0, an Apple M3 Pro with 12 cores and 36 GiB of memory. Rust and Cargo were
1.96.0. Other agents stopped compiler and test work for the capture and no
model inference ran. Exact metadata and samples are under
[`baseline-mac-arm64-bdea942`](https://github.com/owebeeone/sdax-core-evidence/tree/main/campaigns/performance/performance-results/baseline-mac-arm64-bdea942).

The clean-target optimized build took 8.297 s, the no-change warm build
47.097 ms, and the executable was 1,330,192 bytes. Key median / p95 results are:

| Workload | Mac arm64 | Windows x64 | Pi arm64 |
|---|---:|---:|---:|
| chain 1 | 4.92 / 5.13 µs | 5.95 / 7.10 µs | 11.60 / 11.91 µs |
| chain 100 | 0.382 / 0.523 ms | 0.494 / 0.566 ms | 0.893 / 0.915 ms |
| chain 1,000 | 15.289 / 17.032 ms | 12.764 / 12.990 ms | 44.016 / 45.149 ms |
| wide 1,000 | 11.415 / 12.776 ms | 10.016 / 10.366 ms | 29.658 / 30.994 ms |
| sparse 1,000 | 13.706 / 14.223 ms | 11.076 / 11.817 ms | 36.356 / 37.644 ms |
| pre-seeded machine chain 1,000 | 10.016 / 10.436 ms | 6.808 / 6.975 ms | 30.046 / 30.438 ms |
| 1 live dynamic instance | 25.67 / 26.13 µs | 28.65 / 30.20 µs | 63.29 / 64.33 µs |
| 10 live dynamic instances | 0.192 / 0.195 ms | 0.221 / 0.238 ms | 0.490 / 0.495 ms |
| 100 live dynamic instances | 4.888 / 5.472 ms | 5.116 / 5.409 ms | 10.716 / 10.845 ms |

Mac sample spread is materially higher in several fixed-order rows. For
example, chain 100 has p95 37% above median, and wide 1,000 has p95 12% above
median with a 26.8 ms maximum. The raw distribution is retained and those
tails must not be replaced by a smoothed value. Before declaring a small Mac
regression, repeat the same revision in another idle window and compare the
paired captures. The 1,000-node superlinear shape remains large enough to be
visible despite that host noise: chain grows 40.0 times from 100 to 1,000
nodes and the pre-seeded machine is 65.5% of the 1,000-node chain median.

## Matched drained baseline and frozen candidate

The separate [drained baseline fixture archive](https://github.com/owebeeone/sdax-core-evidence/blob/main/campaigns/performance/inputs/performance-baseline-drained-fixtures.tar.gz) reproduces this boundary on the original baseline checkout. SHA-256: `c62509a3f13fada3fd8b49dc06dd4dbb22f7092870fde186dacd878a417f0924`. The earlier v1 archive remains unchanged. The current [performance-harness](https://github.com/owebeeone/sdax-core-evidence/tree/main/campaigns/performance/performance-harness) directory is already adapted to the revised API; it does not require a second copy step in this checkout.

The original three-host baseline above is retained as v1 evidence, but its D
boundary did not explicitly wait for executor-tracked wrapper tasks to retire
after report disposal. It is not used as the denominator below. Baseline v2
and the frozen candidate use the same end-of-sample rule: after the report and
run state are dropped, the timed executor is polled until its tracked-task
count reaches zero. This keeps all per-run disposal inside D.

| Source | Crate-source revision | Fixture revision |
|---|---|---|
| v1 baseline archive | `cfd2b4f5d82a1940ae567b89492ffe768901e843c77eb332107eceac318aec1d` | `6e3a4023e05b0e291bf59f0197a16c61e9d1f67af52b1a21603449f4192cc22b` |
| matched drained baseline | `cfd2b4f5d82a1940ae567b89492ffe768901e843c77eb332107eceac318aec1d` | `5bd68fd521f80941e4ead3e25af70e88aa70f497df6b9ee7a47810f81c0debc7` |
| frozen candidate | `39bc1174d8a3dc276716adc9bab91fa2a4d0a104adfa9dcb62f7cd81fe4f7d54` | `ec74f713161f74705c4e7c7a1ca8aa3e954c9cd244cad75bd2eed057b189170a` |

The candidate was extracted from `final-source.tgz`, SHA-256
`044bf563d7b82eb2fa863aa31af8439eaf4996a9afec6f4029dd493a11f2ad2b`.
Generation remains outside C and D. The matched captures retain 40 samples,
eight warmups and 20 plan-build samples in optimized one-codegen-unit thin-LTO
builds.

An initial candidate run caught 97 tracked tasks at this boundary, which is why
the explicit retirement check remains part of the fixture. A later exact-copy
diagnostic ran 3,000 samples with 1,000 warmups at chain sizes 1, 10 and 100.
It observed zero nonzero tracked handoffs and zero drain iterations in baseline
and candidate runs, both with the harness's self-waking yield and Tokio's
`yield_now`. Two untouched candidate repeats put chain 1 at 4.750/5.000 us,
chain 10 at 26.416/29.396 us, chain 100 at 0.305/0.319 ms and chain 1,000 at
12.943/13.699 ms. There is no evidence that changing the yield primitive
would improve this boundary.

### Full per-run execution

The Mac table uses the final immediate baseline/candidate v3 sequence. The Pi
table uses its matched v2 sequence. Percentages are candidate relative to its
same-host drained baseline.

| Workload | Mac baseline | Mac candidate | Mac change | Pi baseline | Pi candidate | Pi change |
|---|---:|---:|---:|---:|---:|---:|
| chain 1 | 5.58/5.92 us | 4.58/5.04 us | -17.9%/-14.8% | 11.99/12.22 us | 11.00/11.31 us | -8.3%/-7.4% |
| chain 10 | 30.29/30.79 us | 27.90/32.50 us | -7.9%/+5.5% | 70.91/72.46 us | 62.84/63.74 us | -11.4%/-12.0% |
| chain 100 | 0.345/0.357 ms | 0.321/0.331 ms | -6.8%/-7.3% | 0.882/0.914 ms | 0.766/0.777 ms | -13.1%/-14.9% |
| chain 1,000 | 16.868/17.547 ms | 14.746/15.762 ms | -12.6%/-10.2% | 43.961/45.059 ms | 35.502/36.690 ms | -19.2%/-18.6% |
| wide 1,000 | 11.307/12.176 ms | 8.244/8.657 ms | -27.1%/-28.9% | 30.635/31.608 ms | 19.213/19.852 ms | -37.3%/-37.2% |
| sparse 1,000 | 15.222/15.882 ms | 11.295/12.144 ms | -25.8%/-23.5% | 38.889/40.263 ms | 27.361/27.967 ms | -29.6%/-30.5% |
| resource normal | 13.56/15.29 us | 13.83/15.38 us | +2.0%/+0.5% | 23.33/23.93 us | 22.43/22.67 us | -3.9%/-5.3% |
| ready then shutdown | 9.02/9.62 us | 9.71/11.58 us | **+7.6%/+20.4%** | 17.28/17.76 us | 16.91/17.20 us | -2.1%/-3.1% |
| default trace, 100 | 0.379/0.389 ms | 0.331/0.343 ms | -12.9%/-11.9% | 0.904/0.915 ms | 0.774/0.784 ms | -14.4%/-14.3% |

The candidate materially improves the large static graph rows on both hosts.
The Mac ready/shutdown row exceeds both regression triggers; Pi does not
reproduce it. Mac failure-path tails are also noisy: release failure changed
only +2.2% at the median but +102.9% at p95, while startup failure changed
+9.5%/+7.4%. These remain visible as host-sensitive regressions rather than
being averaged away.

The earlier fixed-order Mac v2 candidate began much slower (chain 1 about 13
us and chain 10 about 64 us), while equivalent later rows in that same run and
the two exact repeats were much faster. The final back-to-back v3 result above
also stayed in the faster range. All v2 and diagnostic datasets are preserved;
the evidence supports run/order and host settling sensitivity, not a drain
scheduling defect.

### Plan construction, builds and allocation counts

Plan construction regresses on every reported scale. The cached immutable
layouts shift work from each run into one-time declaration/build, but the cost
is large enough to fail the plan's gates. Counts are deterministic across the
two hosts; this table shows Mac timing and allocation counts.

| Workload | Baseline median/p95 | Candidate median/p95 | Time change | Allocation calls; requested bytes |
|---|---:|---:|---:|---:|
| chain 1 | 0.46/1.21 us | 1.10/1.67 us | +141.0%/+38.0% | 13->36 (+176.9%); 2,317->6,722 (+190.1%) |
| chain 100 | 27.31/34.12 us | 57.12/61.67 us | +109.2%/+80.7% | 429->1,269 (+195.8%); 126,588->304,543 (+140.6%) |
| chain 1,000 | 1.255/1.296 ms | 1.705/1.841 ms | +35.8%/+42.0% | 4,041->12,096 (+199.3%); 1,049,104->2,527,259 (+140.9%) |
| wide 1,000 | 1.392/2.026 ms | 1.954/2.242 ms | +40.4%/+10.7% | 4,041->11,106 (+174.8%); 1,049,103->2,511,641 (+139.4%) |
| sparse 1,000 | 1.373/1.536 ms | 1.759/2.012 ms | +28.1%/+31.0% | 4,041->11,597 (+187.0%); 1,049,105->2,511,293 (+139.4%) |

Pi independently shows plan-build median regressions of +125.9%, +114.7%,
+23.6%, +44.1% and +29.0% for those rows. Candidate full-run allocations
improve despite that one-time cost: on Pi, chain 1000 calls/bytes fall
59,102->52,066 (-11.9%) and 28,976,331->19,479,016 (-32.8%); wide 1000 falls
55,207->49,161 (-11.0%) and 21,300,249->11,818,551 (-44.5%); sparse 1000
falls 57,643->51,106 (-11.3%) and 25,214,541->15,733,193 (-37.6%). The
100-live-instance row grows 68,512->69,635 calls (+1.6%) and
6,238,430->6,293,659 bytes (+0.9%).

| Host | Cold build baseline->candidate | Warm build baseline->candidate | Executable baseline->candidate |
|---|---:|---:|---:|
| Mac arm64 | 10.416->11.661 s (+12.0%) | 51.60->48.23 ms (-6.5%) | 1,347,904->1,558,464 bytes (+15.6%) |
| Pi arm64 | 24.218->27.703 s (+14.4%) | 36.50->35.32 ms (-3.2%) | 1,504,248->1,746,440 bytes (+16.1%) |

Cold and warm compilation are one observation per source and host, so their
percentages are signals rather than distributions. The candidate harness also
contains additional revised-only workloads; the cold-build and binary-size
increases describe these complete consumer harnesses, not an isolated library
build regression with identical consumer code. The increased first-use cost
is retained rather than attributed solely to the library.

### Revised-only behavior

These correctness-verified rows exercise APIs that the baseline cannot
represent, so they have no before/after percentage.

| Candidate-only workload | Mac median/p95 | Pi median/p95 |
|---|---:|---:|
| typed repeated mounts, 2 | 15.52/16.58 us | 29.08/29.65 us |
| typed repeated mounts, 10 | 65.38/70.88 us | 0.142/0.143 ms |
| typed repeated mounts, 100 | 1.225/1.422 ms | 2.607/2.668 ms |
| unknown outcome resolved | 7.62/9.50 us | 16.08/16.54 us |
| unknown outcome failed | 8.39/10.17 us | 17.54/18.43 us |
| stable-handle service restart | 16.42/18.38 us | 26.64/27.04 us |
| service restart exhausted | 11.00/11.62 us | 19.59/20.15 us |
| sequential churn, 1 | 30.94/34.46 us | 58.81/59.98 us |
| sequential churn, 10 | 0.188/0.196 ms | 0.395/0.402 ms |
| sequential churn, 100 | 1.980/2.148 ms | 4.153/4.173 ms |

The churn totals measure all allocations across the sequence; they are not
retained or peak-live bytes. A separate inspection observed 22 machine nodes
after 20 ended instances from an initial two, although ended-instance value
slots were removed. Full metadata reclamation is therefore not claimed.

### Whole-process resident probe and Windows status

The ten-second probe includes process startup, readiness, wait, shutdown,
reporting and exit. On Mac, native maximum RSS was 2,211,840 bytes for
baseline and 2,310,144 for candidate (+4.4%); both rounded user/system CPU to
0.00 s and wall time to 10.00 s. On Pi, both maxima were 10,912 KiB; baseline
recorded 0.001153 s user CPU and candidate 0.001119 s system CPU, with 10.003 s
wall time. These figures are whole-process samples, not engine-only memory or
pure idle CPU.

The initial Windows run was interrupted by an agent-caused deletion during workspace preparation; the incident and recovery remain documented in [WindowsPerformanceIncident-2026-09-08.md](WindowsPerformanceIncident-2026-09-08.md). After owner-authorized recovery and switching Windows OpenSSH to Git Bash, a fresh baseline and candidate were captured with identical Rust 1.98.1 toolchains and offline vendored dependencies. Both passed fixture checks and a reverse-order timing repeat. All workspaces and build outputs are retained.

Windows 1,000-node chain/wide/sparse execution medians improve **19.8%/28.0%/17.9%** in the primary capture and **18.3%/28.1%/18.7%** in the reverse-order repeat. Plan construction is excluded: its corresponding primary medians increase **49.3%/131.1%/111.8%**. Dynamic live-child runtime and allocations regress; 1/10-child medians increase **6.0%/7.2%** initially and **6.9%/14.6%** in the repeat. Service/effect rows vary between captures. There is no blanket Windows performance pass.

Cold consumer-harness compilation is 11.386 → 14.537 seconds; executable size is 1,172,480 → 1,457,152 bytes. Revised-only fixtures contribute, so these are not isolated library-only regressions. Whole-process native peak working set is 5,144,576 → 5,226,496 bytes. CPU counters are coarse and do not establish pure idle CPU or wakeups. See the [complete Windows results and raw evidence](../../artifacts/windows-performance-resume-20260908/WindowsPerformanceResults.md) for boundaries, absolute times, exceptions and reproduction.

### Assessment and remaining gaps

There is no blanket performance pass. Large-graph D latency and allocation
counts improve on Mac, Pi and Windows, while C plan-build time/allocation, cold build,
binary size, and the Mac resident-ready/shutdown row fail one or more initial
gates. The raw distributions remain the decision record.

The study still has no lifecycle-equivalent handwritten Tokio comparison; its
handwritten computation row is only a computation lower bound. It has no exact
wakeup counts or per-workload peak-live memory, and no trace-disabled mode
because the adapter always records the full trace. It has only whole-process
ten-second CPU/RSS probes, one cold and warm build observation per host/source, and no byte measurement for retained dynamic metadata.

## Regression comparison rule

For an integrated before/after comparison, run the API-adapted harness on `weftpi`
with no competing compiler or model workload. Compare matched workload
medians and p95 values, and inspect raw samples before interpreting a ratio.
Flag a median regression above 5% or a p95 regression above 10% when the
absolute shift is larger than the baseline's within-capture spread. Also flag
any unexpected allocation growth or worse scaling, even if time stays inside
those percentages. Correctness changes may carry a cost, but the result must
remain visible under the same boundary.

This single capture does not establish day-to-day host variance, so the 5% and
10% triggers remain the improvement plan's initial engineering gates rather
than statistically calibrated promises. A later integrated run should retain
both captures and can freeze host-specific noise after a same-revision repeat.

## Reproduction

Operational policy update after the incident: current runners retain their temporary build directories and record the path in `retained-target.txt`. No cleanup is performed unless the owner explicitly requests it. Historical fixture archives preserve the scripts used for those captures; when reproducing them, use the current retention-preserving runners. Windows scripts must be delivered through Git/MinGW Bash standard input, with native argument lists, rather than nested PowerShell strings. The historical PowerShell runners are retained for provenance and are not the prescribed Windows execution route.

From an exact checkout of the baseline commit with the untracked improvement
plan and benchmark files present:

```sh
scripts/run-performance-baseline.sh performance-results/baseline-pi-arm64-bdea942
```

The baseline script refuses another Git commit and also refuses any tracked
crate source or manifest content that does not match the baseline source
revision. This prevents a dirty candidate at the same commit from being
mislabelled as baseline. It creates a fresh temporary target for the cold
build, stays offline and locked, performs a warm build in that same target,
runs correctness verification, captures raw CSV, and derives the summary with
Python's standard library. Its result directory also contains the exact build
logs and machine metadata.

The baseline archive can be extracted over an exact `bdea942` checkout to
restore the frozen harness and scripts. The candidate source is in `performance-harness`; it compiles against the revised
explicit component-input and service `initialize`/`serve` APIs. In this candidate checkout, run:

```sh
scripts/run-performance-current.sh performance-results/current-<host>-<revision>
```

The candidate runner records the Git commit, the full crate-source revision,
the fixture revision and the dirty source inventory. A current run is never
labelled as the frozen baseline. Cancellation-after-acquire requires an added
completed-data export under the revised finite-output rule, so that adapted
row is a correctness probe and current-version measurement rather than a
direct timing comparison. Known-receipt compensation retains its direct
effect receipt export and remains comparable. All other shared workload
boundaries remain unchanged.

The adapted harness also adds current-only rows, all marked baseline
unsupported: typed repeated mounts at 2, 10 and 100 mounts; resolved and
failed unknown-outcome reconciliation; stable-handle service restart and
restart exhaustion; and sequential dynamic churn at 1, 10 and 100 instances.
The churn fixture waits for each child's resource release before creating the
next instance, keeping it distinct from the original batch of simultaneously
live instances. Its allocation calls and requested bytes are total churn, not
retained or peak memory.

A separate current-tree engine inspection observed that 20 sequentially ended
instances left 22 machine nodes from an initial two, even though their value
slots were removed. The current benchmark therefore does not claim full
dynamic metadata reclamation. It exposes churn time and allocation scaling;
retained metadata size, peak live memory, resident idle CPU and wakeup rate
are not inferred from the runtime tables. The separate `resident-probe` mode
holds a ready resident plan for ten seconds. Each platform runner records
process CPU and native peak resident memory for the whole optimized harness
process, including startup, readiness, the wait, shutdown, report production
and process exit. It is labelled whole-process activity rather than pure idle
CPU or engine-only memory. Precise wakeup counts remain unmeasured rather than
being recorded as zero.

To check fixtures without collecting performance data:

```sh
cargo run --manifest-path performance-harness/Cargo.toml \
  --release --locked --offline -- verify
```

Per-workload peak live memory, resident idle CPU and wakeups are unmeasured.
The allocation pass gives requested bytes, not peak memory. These remain
separate measurements rather than being estimated from another host's timing
or allocation data.

## Subsequent readiness-registry optimization

The original tables above remain the comparison of the reviewed baseline with candidate `39bc117`. A subsequent targeted fix (`64ca485`) removes repeated readiness-latch cloning and nested instance lookup. In Windows before/after and reverse-order captures, 10-live-child medians improve 7.7–9.5%, and 100-live-child medians improve 22.5–24.0% relative to `39bc117`; 100-child requested bytes decrease 12.8%. The original ten-child allocation budget remains missed by 23 calls and single-child timing is mixed. Engine metadata reclamation was not changed. See [full measurements and remaining flags](../../artifacts/dynamic-profile-20260908/DynamicReadinessResults.md).
