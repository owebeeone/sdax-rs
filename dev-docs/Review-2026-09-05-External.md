# External review of Stage 0 — 2026-09-05

Received by the owner from an external reviewer after commit `6a95537` (Stage 0), relayed
verbatim to the manager. Origin not stated. Recorded here as provenance for the fixes
applied on the same day (see `Stage0Report.md` § "Post-review fixes").

## Reviewer's text (verbatim)

> 1. Shrink the public surface.
> Stop re-exporting engine/host types from the crate root: CxInner, Slots, ChildControl, Scope, compile_fail, engine::{Event, Effect}, take_held / put_output. Authors need prelude plus Plan / Key / Cx / Held / Serving / Invalid / PlanView. Stage 1 still sees the rest as pub(crate) or under sdax::host. After 0.1.0 this is a break.
>
> 2. Close the late-spawns hole.
> PlanBuilder::spawns(key, &template) must reject a non-service (validate V-SPAWN-KIND, or no-op/error if kind != Service). The late form stays — it is how you write the I-34 deadlock — but it must not attach spawns to a resource.
>
> 3. Decide, then freeze (no code unless you change your mind).
> Box<dyn Error + Send + Sync> and try_step → Result<T, Error> are already in the API. Stage0Report still calls them open. Pick them now. If you want a generic error, change it before the first kept commit.
>
> Do not fix before Stage 1: brands, from_json, simulate, close_and_wait, MSRV 1.75 measurement, .exclusive(k).exclusive(k) on V-DUP-ATTR. Those are either deferred on purpose or nits.
>
> Finite-parent + Resident-component: a one-line owner yes in the contract, not a code change.

## Manager's verification (before any fix)

| claim | verified | note |
|---|---|---|
| root re-exports engine/host internals | yes | `lib.rs` re-exported `CxInner` (+ public `take_held`/`put_output`/`take_serve`/`hold_count`), `Slots`, `RawKey`, `Hold`, `StopSignal`, `Scope`, `ChildControl`, `pub mod compile_fail`, `pub mod engine`, `InstanceId`, `SEMANTICS`; the prelude exported `Runtime`/`Observer`/`Clock`/`Time` |
| late `spawns` attaches to any kind | yes | `PlanBuilder::spawns(key, &template)` found the node by key and pushed the template with no kind check |
| adjacent: late `spawns` with an unknown or foreign key | **silent no-op** | not a finding; found by the manager |
| OD-2 / OD-5 already fixed in the API but listed open | yes | `contracts.rs` `Error` alias; `try_step` → `Key<Result<T, Error>>` |
| `.exclusive(k).exclusive(k)` passes `V-DUP-ATTR` | yes | `exclusive()`/`shared()` record into their own lists, not `attrs.declared` |

## Owner decisions (2026-09-05)

- Apply 1 and 2, plus the two adjacent holes (silent no-op → findings; repeated or
  conflicting lock attributes → `V-DUP-ATTR`).
- Item 3: keep the boxed error and `try_step` → `Result<T, Error>`; record both as decided
  (manager's recommendation, accepted by "go ahead and fix").
- Finite parent with a resident component: approved as one contract sentence.
- Not fixed before Stage 1, as the reviewer advised: brands, `from_json`, `simulate`,
  `close_and_wait`, MSRV 1.75 measurement.
