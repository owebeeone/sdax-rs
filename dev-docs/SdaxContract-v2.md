# sdax/2 — revised lifecycle contract

This revision adopts the invariants and driver obligations of [sdax/1](SdaxContract-v1.md) except where this document explicitly replaces them. The previous document remains historical evidence; its superseded API spellings and decisions are not compatibility promises. The revision is implemented; the integration report records executed coverage, performance results and the unmet low-cost-model adoption gate.

## Construction and execution

A plan is an immutable typed definition shared across independent runs. Its input type is consistent at every entry point. A unit definition is the ordinary `In = ()` case, not a separate hidden no-input kind. Root start supplies input, static component mount binds it, and dynamic spawn supplies per-instance input.

Each body declaration reserves a node immediately. Every reserved chain must reach its terminal before build; dropped and forgotten intermediate builders remain incomplete findings. No valid plan can silently omit a begun declaration. Type-state errors and warnings supplement this build guarantee.

An exported key must belong to the exporting plan. A blocking node has one execution-pool declaration through `.on(pool)`; also declaring `.limit(pool)` is a build error, never a silent pool override.

Static mounts have distinct identities even when they use the same definition. Formal imported values are explicitly bound to parent keys. All bound resources retain child-before-parent cleanup edges. Immutable metadata/body factories may be shared; slots, locks, pools and mutable instance state are per run and mount. Captured mutable application state is shared only when the author explicitly captures it.

Build resolves and validates known static structure, binding, identity and budgets. Compiled static metadata is reusable; dynamic instantiation and per-run state creation remain execution work. Every error knowable from complete static bindings is rejected before a normal typed start. Unbound formal definitions may be stored for later binding but cannot execute unbound.

## Acquisition and obligation

An attempt receives one non-cloneable acquisition capability. Consuming it invokes at most one lazy acquisition factory, or registers one already-owned effect-free value. Its ordinary shared context has no acquisition authority. A defensive host registration guard rejects repeated registration without replacing the first value or invoking a rejected factory.

The poll that observes acquisition success registers the value and obligation before the user's continuation can run. Cancellation after that handoff does not lose cleanup. An unresolved in-flight external action is not assumed unsuccessful. A retry receives fresh authority only after the preceding attempt has settled and its recorded obligation has been discharged or honestly abandoned.

RAII cleanup removes the engine-owned value and all declared input/import/component-export aliases before the cleanup completes. Destruction occurs inside the guarded cleanup poll if no caller-retained or opaque-value Arc clone remains. Such clones cannot be revoked. Removing an RAII value does not preserve it solely for a post-cleanup Report output.

Held values receive exactly one cleanup attempt unless the shutdown budget prevents completion and records the unresolved obligation. Independent cleanup continues after another cleanup fails. Separate acquisitions are separate nodes; placing handles in a tuple does not by itself solve partial acquisition.

## Known and unknown external outcomes

A recorded receipt permits ordinary receipt-based compensation. An effect whose outcome is unknown has no fabricated receipt. Its explicitly declared recovery handler receives typed operation identity available before the external action, with identity stable across idempotent retries and attempt information separately accessible.

Automatic recovery requires the matching identity, handler and safety declaration. Selecting a policy alone is insufficient. A resolved recovery discharges uncertainty; still-unknown, failure, panic or expiry retains unresolved reporting. The original Ambiguous observation remains in enabled trace history even when recovery succeeds.

Recovery is a distinct phase, not a resource release without an acquired value. This is the explicit exception/clarification to v1's “not held ⇒ no release body”: no receipt release or compensation is called without its receipt, but declared unknown-outcome recovery can execute. Dependency lifetime, cleanup shielding, task joining and shutdown budgets apply to recovery too. A cancelled run remains cancelled even when cleanup/recovery completes. No durable journal or exactly-once external guarantee is implied.

## Services

Initialization establishes readiness and publishes one stable handle per run/instance. A serving episode operates against that handle. On recoverable serving failure, restart invokes another serving episode; it does not rerun successful initialization, replace the published handle or replay dependents.

Initialization retry and serving restart are separate policies. Retry counts total initialization attempts; Restart.max counts recovery episodes after the initial serving episode. Episode identity is explicit and independent of initialization attempt identity. Readiness is latched after successful initialization; availability/health during recovery must not be inferred from the readiness latch alone.

Context deadlines are absolute and phase-aware: an active work timer for prepare/run, the containing budget for cleanup/recovery, and the active stop timer capped by the containing budget after signalling. The shared context deadline is updated before the stop signal becomes observable; repeated queries do not move the timer.

Shutdown stops current initialization/serving work, interrupts restart backoff and follows stop/grace/budget rules. Async resource acquisition belongs in resource nodes or owned child scopes. A stable handle can encapsulate a reconnecting service but does not promise uninterrupted availability.

## Outputs and review

A finite plan exports completed data, not a direct live resource or service handle whose cleanup has ended. A resident child scope may publish a live handle for use under its parent's lifetime edges. This rule is a declaration-level guard, not a proof that arbitrary user data contains no cloned handle.

Report.into_result preserves the entire failed report. Standard text retains real error causes, node paths, phases, cleanup failures and unresolved work. Rendering may bound pathological user error-source chains without discarding typed data from the report.

The lifecycle inspection view shows ordering, readiness, policy and cleanup. A separate dataflow view includes inputs/import bindings. Neither proves what arbitrary closures do, nor establishes idempotency.

## Verification and performance

Behavior changes use failing desired assertions before implementation, on both pure and adapter paths where relevant. Invariant checks are updated for recovery, stable service episodes and independently mounted scopes. Exhaustive schedule exploration is claimed only if actually performed.

Performance reports separate generation, Rust compilation, one-time plan build, engine execution and representative application work. Engine execution includes input binding, run state, scheduling, body dispatch, dynamic instances, cleanup, report production and per-run disposal. No generator or one-time plan-building work is included in the engine tally; no per-run work is hidden as setup.
