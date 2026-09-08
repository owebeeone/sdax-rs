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

The final baseline and candidate rows and source hashes are retained under
`performance-results/component-copy-baseline-ce339a5-plan-build/` and
`performance-results/component-copy-candidate-plan-build/`. Pre-change lifecycle
logs are retained under `performance-results/component-copy-baseline-ce339a5/`;
post-change logs are in the candidate directory.

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
- Rust 1.75 build: pending. The installed `1.75.0-aarch64-apple-darwin` entry has
  no usable manifest, and the `+1.75` offline inventory could not resolve the
  already-pinned `tokio-util` package while rustup attempted to repair that channel.
- Lifecycle-equivalent handwritten Tokio comparators and host-reserved timing remain
  open work from the remediation plan.
