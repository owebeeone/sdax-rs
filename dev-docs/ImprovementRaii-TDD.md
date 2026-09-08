# RAII disposal ordering correction

2026-09-08. Follow-up to the independent final adversarial review.

## RED

Promoted the original desired `raii_dependent_is_actually_dropped_before_parent_release` assertion into `crates/sdax-tokio/tests/raii_release_order.rs`. Ran `cargo test --offline -p sdax-tokio --test raii_release_order` before changing core code. It failed: actual `["parent released", "child dropped"]`, expected `["child dropped", "parent released"]`, although the report was clean.

The terminal release future cloned the held resource but its source slot retained another Arc until final BodySource disposal. Explicit input/import bindings and resident component exports could retain additional engine-owned aliases.

## Change and GREEN

`Slots::take_erased` moves a value out without invoking its destructor under a storage lock. Build-time `BodyLayout` now caches the transitive declared aliases of each RAII resource, covering typed static input, formal/concrete imports, nested component exports and template declarations.

For `ReleaseStyle::Drop`, BodySource first constructs the existing terminal cleanup future, which owns an Arc. It then removes the owner's slot and matching declared aliases, moving those values into an async cleanup wrapper. The wrapper drops storage references during cleanup; the terminal invokes the user's by-drop factory inside its existing guarded poll with the remaining engine Arc. A destructor panic becomes a cleanup failure rather than poisoning a slot lock or skipping upstream cleanup. Terminal, engine and driver code are unchanged.

Dynamic alias selection resolves the candidate's owner scope and compares slot-table Arc identity. It clears only that owner's instance copy. Alias candidates are cached at build; dynamic-instance lookup and removal remain counted runtime work. Flat resources with no aliases allocate no alias vector storage.

Six focused cases pass:

- Original child-destructor-before-parent-release assertion.
- Repeated static mounts retaining both typed-input and formal-import aliases.
- Nested resident component export aliases read by the parent.
- Destructor panic reported under the child ReleaseBody, with parent release still occurring.
- An explicit caller-retained Arc delays destruction, demonstrating the deliberate limit.
- Nested repeated alias scopes in two dynamic instances per run, across two concurrent parent runs; stopping the first run does not destroy the second run's resources.

These extended cases are regression guards. The original desired assertion supplies the observed RED witness; no claim of exhaustive schedule exploration is made.

## Limit

The guarantee covers the resource's own slot and declared engine-owned input/import/component-export aliases. If the caller retains an Arc, including one nested in opaque values, service handles or explicit shared state, that clone can keep the value alive beyond declared cleanup. sdax cannot identify or revoke arbitrary application-held references. Clearing declared aliases does not claim unique Arc ownership or force a destructor while such clones exist.

Final workspace and lint results are reported in the agent handoff. Rust 1.75 remains unavailable on this Mac; the integrated stage must run its existing remote/platform MSRV gate.
