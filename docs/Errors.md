# Errors

Two moments: `build` may refuse the declaration, and a run produces a
`Report`.

## `build` → `Invalid`

`build` reports **every** finding at once. A `Finding` names the rule,
the nodes, the keys, what was found, and a fix. An empty plan is the
smallest rejection; a duplicate name is the next. The test quotes
both.

```rust,guide:validate_reject
use sdax::prelude::*;
use std::sync::Arc;
use std::time::Duration;

struct Conn;

#[test]
fn build_refuses_and_the_finding_names_the_node() {
    let empty = Plan::builder("Empty")
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(1)),
            Mode::Finite,
        )
        .expect_err("empty");
    assert_eq!(empty.checks.len(), 1);
    assert_eq!(empty.checks[0].rule, Rule::Empty);

    let mut p = Plan::builder("Twice");
    let conn = p
        .resource("Conn")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Conn)) })
        .release(|_cx, _c: Arc<Conn>| async move { Ok(()) });
    p.step("Ping")
        .needs(conn)
        .run(|_cx, _c: Arc<Conn>| async move { Ok(()) });
    p.step("Ping")
        .needs(conn)
        .run(|_cx, _c: Arc<Conn>| async move { Ok(()) });
    let err = p
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(1)),
            Mode::Finite,
        )
        .expect_err("two nodes named Ping");
    let dup = err
        .checks
        .iter()
        .find(|f| f.rule == Rule::DupName)
        .expect("V-DUP-NAME");
    assert_eq!(dup.nodes, vec!["Ping".to_string(), "Ping".to_string()]);
    assert!(dup.detail.contains("Ping"), "{}", dup.detail);
    assert!(!dup.fix.is_empty());
}
```

| Rule | Id | What it means | Usual fix |
|---|---|---|---|
| `Empty` | `V-EMPTY` | no nodes to run (an input or an import does not count) | add a node |
| `ForeignKey` | `V-FOREIGN-KEY` | a key or pool of another plan | `import` an ancestor's key, or keep the key in this plan |
| `DupName` | `V-DUP-NAME` | two nodes of one scope share a name | rename one |
| `DupAttr` | `V-DUP-ATTR` | an attribute set twice, or a resource locked twice | set each attribute once |
| `ImportScope` | `V-IMPORT-SCOPE` | a child imports a key this plan does not own | import a key you declared or imported |
| `SpawnSelfImport` | `V-SPAWN-SELF-IMPORT` | a template a service spawns imports that service | import a resource the service needs, not the service |
| `SpawnKind` | `V-SPAWN-KIND` | `spawns` on a node that is not a service | only services spawn |
| `LockNeeds` | `V-LOCK-NEEDS` | `exclusive` / `shared` names a resource the node does not need | `.needs` that resource first |
| `IdempotentRequired` | `V-IDEMPOTENT-REQUIRED` | retry or evidence-free compensation without `idempotent` | `.idempotent()` |
| `PoolStarve` | `V-POOL-STARVE` | a resident holder can starve the pool | do not hold a pool slot in a service |
| `UnusedPool` | `V-UNUSED-POOL` | a pool nobody uses | remove it, or `.on` / `.limit` it |
| `ServiceUnbounded` | `V-SERVICE-UNBOUNDED` | `Shutdown::unbounded()` and a service with no `stop_within` | bound the shutdown, or bound the stop |
| `TryUnconsumed` | `V-TRY-UNCONSUMED` | a try-step nothing depends on | add a node that `needs` it |
| `BudgetOrder` | `V-BUDGET-ORDER` | a stop or child budget larger than the one that contains it | shrink the inner bound |
| `Mode` | `V-MODE` | `Mode::Finite` on a plan with a service or a template | `Mode::Resident` |
| `BlockingCancel` | `V-BLOCKING-CANCEL` | `cooperative` on a blocking step | omit it — a thread is signalled and never aborted |
| `PersistAmbig` | `V-PERSIST-AMBIG` | `on_ambiguous(Compensate)` on a `persistent` effect | `Report` or `Retry`, or compensate instead of persisting |

## After a run → `Report`

`Report` carries the outcome, the exported value, faults, cleanup
failures, abandoned obligations, and ambiguous effects.
`Report::into_result` is `Ok` only when every list is empty and the
outcome is `Ok`. The error *is* the report.

`Outcome` is `Ok`, `Failed`, or `Cancelled`. A cleanup failure is a
record, not a lost exception. `FailFast` stops the run at the first
fault; `Isolate` aggregates and continues.

The [Cleanup](Cleanup.md) test is the run-time half: `Boom` faults,
`Conn` still releases, `outcome` is `Failed`.
