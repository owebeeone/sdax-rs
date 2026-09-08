# Component mount allocation checkpoint

Date: 9 September 2026. Baseline: `ce339a59eefcd136562b16f4f20035c29ef0c988`.

## Performance witness

The witness builds one exported resource-bearing child definition, mounts it 2,
10 or 100 times in a typed chain, and counts allocation calls and requested bytes
on the construction thread. Each printed result is checked against four immediate
repetitions. All five captures were byte-for-byte identical at every scale. The
checksum observes the finished plan name without constructing an inspection view.

| Mounts | Baseline calls / bytes | Candidate calls / bytes | Change |
|---:|---:|---:|---:|
| 2 | 176 / 26,935 | 162 / 24,407 | -14 / -2,528 |
| 10 | 605 / 108,031 | 535 / 95,391 | -70 / -12,640 |
| 100 | 5,192 / 1,220,841 | 4,492 / 1,094,441 | -700 / -126,400 |

The measured reduction is exactly 7 allocation calls and 1,264 requested bytes
per mount. These are allocator requests during plan construction, not retained or
peak memory. No timing benchmark was run because other lanes were active on the
host.

An earlier exploratory capture included `plan.inspect()` in the counted checksum
and therefore included allocations for constructing an inspection view: 310 /
34,043 bytes at 2 mounts, 1,330 / 147,851 at 10, and 12,498 / 1,699,071 at 100.
Its candidate delta was the same exact per-mount reduction. Those captures remain
retained in the sibling directories without `-plan-build`; the table above is the
final isolated plan-construction boundary.

The final baseline and candidate rows and source hashes are retained under
`performance-results/component-copy-baseline-ce339a5-plan-build/` and
`performance-results/component-copy-candidate-plan-build/`. Pre-change lifecycle
logs are retained under `performance-results/component-copy-baseline-ce339a5/`;
post-change logs are in the candidate directory.

The full library source revisions from `performance-source-revision.py` are
`ee2fa0f90514e6ae168c3d502a3a0aac2ca692d473a0c335daeb4e155ddb3edd`
for baseline and
`3b6229329a8d057979b1d2d580dcfaecc17ef0e6cf3f7f21accbbc87d39e3cee`
for candidate commit `d20b7c39b1bc6e509f8799dee43f2e364c0d6c8f`. The shared fixture revision is
`e98a18c1d9bb352fecde2ec57e57e52a3928579b1fceffccea7099f10ed0cec3`.

## Change and behavior guards

`PlanBuilder::component` previously cloned the freshly remapped child `PlanIr`
before placing it in an `Arc`, solely because it read the child's copyable export
key afterward. The candidate saves that export key first and moves the child into
the `Arc`.

This optimization has no behavior RED. Before the change, the existing
`improvement_components` suite passed all seven tests, including concurrent
repeated mounts, nested repeated mounts, distinct bindings, and child-before-parent
resource cleanup. After the change, that suite and the new resource-bearing harness
guard passed. The harness starts the same ten-mount plan twice with distinct inputs,
observes the corresponding outputs, a clean report, and exactly ten additional
child releases after each run. This is regression coverage, not proof of lifecycle
behavior.

## Verification

- `cargo test --workspace --locked --offline`: passed.
- `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`: passed.
- Workspace and standalone harness formatting plus `git diff --check`: passed.
- Native validation of candidate library snapshot `d20b7c3` passed independently
  on Windows x64 and Pi arm64 at source revision
  `3b6229329a8d057979b1d2d580dcfaecc17ef0e6cf3f7f21accbbc87d39e3cee`:
  each host reports
  446 tests passed, 0 failed and 2 ignored, and both libraries build with Rust
  1.75. The retained Windows logs are at
  `/Users/owebeeone/limbo/sdax-wz/artifacts/remediation-20260909/native-performance/`;
  Pi logs are under its `pi/` directory. No native timing was measured.
- Handwritten Tokio lifecycle correctness controls are recorded in
  [the comparator checkpoint](TokioLifecycleComparator-2026-09-09.md).
  Host-reserved timing remains open work from the remediation plan.
