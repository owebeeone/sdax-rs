# Performance measurement protocol — 2026-09-09

Status: preregistered before lifecycle, Windows dynamic-allocation, or Pi
runtime measurements. This protocol does not report results.

## Frozen boundaries

- Before: commit `ce339a59eefcd136562b16f4f20035c29ef0c988`, crate-source
  revision `ee2fa0f90514e6ae168c3d502a3a0aac2ca692d473a0c335daeb4e155ddb3edd`.
- After: crate-source revision
  `3b6229329a8d057979b1d2d580dcfaecc17ef0e6cf3f7f21accbbc87d39e3cee`.
- Locked dependencies, release profile, current-thread Tokio runtime, one worker,
  and full SDAX trace remain fixed. Generation, compilation, plan building,
  execution, allocator traffic, peak live requested bytes, retained requested
  bytes, and native RSS are distinct boundaries.

## Lifecycle comparator

Normal release, partial startup failure, downstream cleanup failure with
upstream cleanup continuing, and cancellation after acquisition must expose the
same external summary from SDAX and handwritten Tokio. The independent checker
requires exact outcome, output, error type/message/source chain, cleanup list,
dependency-order events, task starts/joins, zero active tasks, and payload
disposals. SDAX's full trace is required implementation work; the handwritten
implementation is not given a fabricated copy of that internal representation.
Both sides include external-summary normalization and disposal in the measured
boundary. Cancellation uses the same one-shot acquisition-ready orchestration.

Allocator self-controls must distinguish a buffer dropped inside the measured
closure from one retained across its end. Every finite lifecycle capture must
finish with exactly zero retained requested bytes, equal spawned/joined counts,
and zero active tasks. Allocation calls, requested bytes, peak live delta, and
retained delta are separate fields. Five post-warmup allocation captures must
agree exactly before their data is used. Peak live bytes are not RSS.

Lifecycle timings use eight warmups and 40 samples. Capture SDAX then Tokio and
repeat in the reverse order. These ratios are descriptive because there is no
historical lifecycle-equivalent comparator denominator.

## Before/after host order

Use fresh retained directories and preserve commands, source and fixture hashes,
compiler/profile metadata, stdout, stderr, summaries, and raw rows.

- Pi: baseline then candidate, followed by candidate then baseline. Run fixture
  verification before warmups. Use 40 execution samples and 20 build samples.
  Inspect all matched rows, with explicit attention to sparse 1/10/100/1,000.
- Windows: baseline then candidate, followed by candidate then baseline through
  Git Bash. Compare `dynamic_live_instances_1`, `_10`, and `_100` allocation
  traffic. Sequential churn remains candidate-only unless the baseline fixture
  implements that exact behavior; it receives no fabricated denominator.

The end-to-end `plan_build` row remains unchanged. Additional diagnostics split
graph declaration construction from validation/freeze where the harness can do
so without library changes. Existing `machine_state_setup` and
`body_state_setup` rows remain the table and body-layout diagnostics.

## Frozen decision rule

For each baseline capture, within-capture spread is nearest-rank p95 minus
nearest-rank p05 of its raw samples. Flag a candidate median increase above 5%
or p95 increase above 10% only when the corresponding absolute increase is also
greater than that baseline spread. Flag any unexpected allocation growth or
worse scaling independently of timing. Do not loosen these thresholds after
seeing data, average away order effects, or claim an overall pass while a
material flag remains unexplained.
