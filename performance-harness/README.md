# Current API performance harness

This directory is the API-adapted copy of the frozen baseline harness. It is
kept separate so the `bdea942` fixture bundle remains byte-for-byte
reproducible. Before an integrated measurement, copy this directory to
`performance-harness` in the candidate checkout, or set
`SDAX_PERFORMANCE_HARNESS=performance-harness-current` when invoking
`scripts/run-performance-current.sh` from this isolated checkout.

The shared graph, plan-build, lifecycle, application and allocation workloads
retain the baseline measurement boundaries. Unit-input components now pass an
explicit input binding and resident services use separate `initialize` and
`serve` bodies. The cancellation fixture adds a finite computed output because
the revised validator rejects direct resource exports, so that row remains a
correctness and performance probe but is not a direct before/after comparison.
The known-receipt effect keeps its baseline-equivalent direct receipt export.

`run-performance-current` records both the Git commit and a SHA-256 digest of
all workspace manifests, the lockfile and crate Rust sources. This matters
while the candidate is an uncommitted working tree: a commit name alone does
not identify the measured engine.

The current-only rows cover typed repeated mounts at 2, 10 and 100 mounts,
resolved and failed unknown-outcome reconciliation, stable-handle service
restart and restart exhaustion, and sequential dynamic churn at 1, 10 and 100
instances. The churn fixture waits for each child's resource release before
creating the next instance, while `dynamic_live_instances_*` retains the
original simultaneous-live workload. Allocation calls and requested bytes are
captured for both; they do not represent retained or peak memory.

The adapter always records the full trace. `trace_default` versus
`trace_observer` measures the added counting callback while holding that trace
behavior constant; it is not a trace-disabled comparison. Resident idle CPU
and peak live memory are not inferred from the runtime tables. The separate
`resident-probe` mode holds a ready resident plan for ten seconds; the host
runner records process CPU and native peak resident memory for the whole
optimized process, including startup and shutdown. This is not pure idle CPU
or engine-only memory. Precise wakeup counts remain unmeasured.
