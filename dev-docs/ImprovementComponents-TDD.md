# Component/input improvement TDD log

Date: 2026-09-08. Isolated component checkout; no commits, dependencies or network access.

## Contract/API decisions

- `Plan::with_input::<I>` is the canonical definition constructor. `Plan::builder` is exactly `with_input::<()>`; the redundant `Plan::template` constructor is removed. Register dynamic plans with `parent.template`.
- `parent.component(name, &Plan<O,I>, binding) -> Key<O>` requires an explicit binding. A sealed `InputBinding<I>` accepts `Key<I>`; only `I=()` additionally accepts literal `()`.
- `builder.port::<T>(name) -> Key<T>` declares a formal import; `plan.bind(port, parent_key) -> Result<Plan<O,I>, Invalid>` makes an immutable bound definition view with shared factories. Parent build rejects missing/foreign bindings. Concrete `import` remains available when deliberately constructing for one known parent.
- Definitions with imports cannot start as roots; the cached root-layout result records unresolved imports. Mounted definitions are resolved during parent build. Wrong binding types and missing input arguments fail during compilation.
- Definition identity is retained as `PlanIr.origin`; every mount receives fresh scope IDs. Bodies are shared, slot indices remain stable, and captured template handles resolve against the mounting scope's original identity.

## RED / GREEN

1. Added `crates/sdax-tokio/tests/improvement_components.rs` with typed repeated mounts/concurrent runs, formal-port binding across different parents, and ordinary unit input. Ran `cargo test --offline -p sdax-tokio --test improvement_components`; RED: E0061 for three-argument component mounts, E0599 for absent `port`/`bind`. Implemented typed binding, mount identity normalization, and scope-aware shared body factories. GREEN: all three tests.
2. Added nested repeated mounts containing a captured dynamic template handle. An initial fixture budget mismatch was rejected as expected and corrected (leaf 1s, child 2s, middle 3s, root 5s). GREEN: both mounted services spawn distinct inputs and stop cleanly. Existing nested-instance and differential conformance suites retained.
3. Added malformed binding coverage: missing formal port and a foreign typed input key must fail parent build before any bodies execute. GREEN: structured ImportScope findings. Port type mismatch, omitted input argument, and wrong input type are compile-fail witnesses asserting E0308, E0061 and E0277 respectively.
4. Added concurrent resource-input/formal-import test with independent child resources and mount-local pools. Each run's two child releases precede its parent's release; returned computed data is run-specific. GREEN.
5. Added lifecycle inspection assertion for bound child input resolving to its parent resource. Ran focused test; RED: actual need was mount `a`, expected `parent`. Fixed resolver to preserve explicit input-source mapping and treat only constant unit input as supplied non-lifecycle data. GREEN: input and formal port both resolve to parent resource, cleanup order agrees.
6. Existing unit-input launch refusal witnesses were migrated to desired successful bindings across direct, nested, template-contained and nested-template boundaries. White-box tests now include the ordinary unit input declaration, and host tests explicitly resolve mounted keys rather than reusing definition keys.

## Execution layout and measured boundaries

Static engine topology is computed during plan building and stored in `Arc<Table>`. Each run allocates fresh slots, locks, scope status and counters while sharing the immutable topology. The first dynamic instance uses `Arc::make_mut` to copy the table, then appends instance-specific topology privately; that first-copy cost and subsequent dynamic flattening remain runtime work. No speedup number is claimed here.

`BodyLayout` caches immutable mounted scope metadata, import bindings and template scope/factory inventories during build. Starting a run or instance allocates mutable slot tables from that inventory. User closure environments are never cloned or implicitly shared as mutable run state. Caller-explicit captured synchronization remains caller-owned.

## Verification

Commands and final results are reported in the component agent's integration handoff. One intermediate all-workspace run executed every test successfully but its final adapter doctest hit E0460 because a concurrent core rebuild invalidated rustdoc's linked artifact; the final run is sequential. No artifact or test failure is hidden by that diagnosis.

MSRV: this machine's installed toolchains are recorded in the handoff. Code uses Rust 1.75-compatible APIs. A newer local compiler is not claimed as a Rust 1.75 build; root-stage MSRV verification remains required if 1.75 is unavailable locally.

Final local verification: full `cargo test --workspace --locked --offline` passed (including all seven new component regression tests, existing differential/conformance/Monte Carlo tests and doctests). `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`, strict rustdoc, `cargo fmt --check`, architecture and guide-quote gates passed. The compile-fail gate initially identified W-12's obsolete E0308 expectation (the revised typed binding correctly emits E0277); the witness description/expected code were migrated and the gate rerun. All 18 compile-fail code assertions then passed.

Additional regression guard: `sibling_mount_pools_admit_independently` inspects the initial machine effects and requires both one-slot child pools to admit their work simultaneously. This distinguishes independent scope pools from silently shared serialization.

The available local toolchains are stable (rustc 1.96.0), nightly, nightly-2026-02-28, 1.85/1.85.0, and 1.95.0. Rust 1.75 is unavailable locally; MSRV remains pending the root-stage platform check. Full package/external-consumer checks are left to the integrated root-stage gate, since this branch intentionally coexists with the concurrent safety/services API migration.
