# F1 real-value regression evidence

Date: 2026-09-06. Test-only worker; implementation supplied concurrently by
engine, typed-bridge and driver owners. No library files changed by this worker.

## Files and assertions

- `crates/sdax-tokio/tests/fixer_values.rs`: seven tests consume actual join units
  (including a chained join), export component 99, consume it as 100, import root
  input 42 through a component, consume both implicit and explicit unit exports,
  cross nested component boundaries, and run the same plan concurrently with
  inputs 11 and 22 (distinct output allocations).
- `crates/sdax-tokio/tests/fixer_values_edges.rs`: five tests cover failed and
  skipped exports without constructing the parent consumer; an isolated unrelated
  child fault while delivering output 100 and retaining that fault; concurrent
  instances containing components and distinct values before Child readiness;
  nested template instances whose component reads both local input and ancestor
  import (111,11 and 122,22); and resident readiness with actual output 99 on a
  two-worker runtime using default observation-disabled RunOptions.

No test sleeps, external network access, raw task spawning, or fixture body
substitution is used. Source-level publication failures and engine rejection /
acknowledgement cancellation cases are other workers' assignments.

## RED

The first live command was blocked by concurrent implementation: the newly
required BodySource::publish_ready method was not yet implemented by
ScriptedBodies (E0046). That is not claimed as behavioral RED.

To recover clean before-change evidence, exported `git archive HEAD` into
`/tmp/sdax-values-red.23HSkL`, copying only the new test files into that baseline.
The source baseline was `8147239ebed2c5a82dc622bdebc39eddfbe9a6de`.

```
CARGO_TARGET_DIR=/tmp/sdax-values-red-target cargo test \
  --manifest-path /tmp/sdax-values-red.23HSkL/Cargo.toml \
  -p sdax-tokio --test fixer_values --locked --offline
```

Result: compiled, exit 101, zero passed / seven failed. Join and component
consumers reported Failed; component export, imported input, nested component,
and concurrent-run tests returned missing output instead of 99, 42, 99 and 11.

The corresponding `--test fixer_values_edges` command ran the first four edge
tests: one passed (failed/skipped negative control), three failed (missing output
100 or readiness Failed). After adding the nested-template regression, the same
command with filter `nested_template` compiled and failed at readiness. Thus
all eleven defect-witness tests failed against unchanged baseline, while the
negative control preserved existing behavior.

## GREEN

```
cargo test -p sdax-tokio --test fixer_values --test fixer_values_edges --locked --offline
```

Result: exit 0; seven plus five tests passed, zero ignored, each target reported
0.00 seconds test execution. This is integration evidence on the shared working
tree while other implementation workers remained active, not an isolated patch
claim. The coordinator owns final whole-workspace verification.

Owned Rust files formatted with `rustfmt --edition 2021` only. No commits,
release actions, source manifest edits, or original-plan edits performed.
