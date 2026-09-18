# Handwritten Tokio lifecycle comparator checkpoint

Campaign evidence and experiment runners referenced here moved to the private [SDAX archive](https://github.com/owebeeone/sdax-core-evidence) on 2026-09-10. Recorded paths and commands remain historical; see the archive README and REPLAY.md for current locations and fresh-run setup.

Date: 9 September 2026. Library snapshot: `d20b7c39b1bc6e509f8799dee43f2e364c0d6c8f`.
Fixture revision:
`7e442c2829a899e1f284a34d4c0a30201216a40b33e09933820e2ac2adc55e61`.

## Boundary

The standalone performance harness now carries a correctness-only handwritten
Tokio comparator for the four initial lifecycle cases in the remediation plan.
It does not yet emit benchmark samples.

- Normal release acquires one resource with value 41, computes output 42, then
  releases and disposes it.
- Partial startup failure acquires the upstream resource, records the dependent
  run failure with node, phase and message, then releases the upstream.
- Cleanup failure computes output 41, records the downstream release failure with
  node, phase and message, and continues to the upstream release.
- Cancellation stores the held resource before the acquisition task completes,
  waits for that fact, aborts and joins the acquisition, then releases the resource.

Every acquisition, body and cleanup operation runs as a counted Tokio task. The
fixture awaits every handle, including the aborted acquisition. A drop counter
checks exact resource disposal, an event record checks dependency cleanup order,
and the final active-task count must be zero.

## TDD evidence

The first compile control failed because the requested comparator module did not
exist (`E0583`). After the four cases were added, independent negative controls
were written before their checker; that compile failed with `E0425`. The completed
checker rejects a wrong output, changed cleanup order, wrong run-fault phase,
truncated cleanup-failure message, missing disposal and a remaining active task.
The RED and GREEN logs are retained under
`performance-results/tokio-lifecycle-comparator/`.

## Verification

- Standalone harness unit test: passed, including all negative controls.
- Full harness `verify`: passed, including both SDAX and handwritten cases.
- Harness clippy: passed with the existing `io_other_error` findings allowed;
  the comparator's one direct Tokio spawn has a call-site allow because owning,
  aborting and joining that task is the behavior being compared.
- Harness formatting and `git diff --check`: passed.
- Timing, allocation comparison, resident recovery and dynamic containment remain
  pending. No timing result is claimed.
