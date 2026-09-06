# F1 typed structural value bridge

Implemented in `builder.rs` and `host/bodies.rs`, with direct host regression
coverage in `crates/sdax/tests/fixer_ready_bridge.rs`.

- Builders record sparse structural entries: joins and components without an
  export install unit; declared component exports retain a typed reader which
  clones the real `Arc<O>`.
- `BodySource::publish_ready` is required. Its plan implementation resolves the
  exact static/instance scope, refreshes imported exports, releases the source
  lock before taking the destination lock, and returns errors containing the
  destination, instance, optional source, and reason. Missing declared exports,
  including explicit unit exports, never fall back to unit.
- Outward import resolution is unchanged. Publication cannot fall back to an
  ancestor instance or a static scope.

## Evidence

RED: `cargo test -p sdax --test fixer_ready_bridge --locked --offline` exited
101 with six E0599 errors: `publish_ready` did not exist on `Arc<dyn BodySource>`.
The first two tests then passed after implementation.

Additional controls initially exposed invalid test fixtures (empty plans and a
finite root containing a template); these were corrected with real join nodes
and a resident template root. A transient compile failure while the simulator
agent introduced its pending engine variants was resolved by the coordinated
engine changes, not by weakening tests.

GREEN: the same command passes all five tests: join unit publication;
component export Arc identity, missing/wrong type errors and separate-run slots;
explicit versus implicit unit exports; imported parent export; separate and
closed instance scopes. Runtime lifecycle/failure acknowledgement checks belong
to the engine and adapter portions of F1, not these direct bridge tests.
