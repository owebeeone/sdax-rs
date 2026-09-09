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
| `IncompleteDeclaration` | `V-INCOMPLETE-DECLARATION` | a resource, effect, or service chain did not reach its terminal | complete the chain before `build` |
| `LiveExport` | `V-LIVE-EXPORT` | a finite plan exports a resource or service handle | export completed step data instead |
| `Empty` | `V-EMPTY` | no nodes to run (an input or an import does not count) | add a node |
| `ForeignKey` | `V-FOREIGN-KEY` | a key or pool of another plan | `import` an ancestor's key, or keep the key in this plan |
| `DupName` | `V-DUP-NAME` | two nodes of one scope share a name | rename one |
| `DupAttr` | `V-DUP-ATTR` | an attribute set twice, or a resource locked twice | set each attribute once |
| `ImportScope` | `V-IMPORT-SCOPE` | a child imports a key this plan does not own | import a key you declared or imported |
| `SpawnSelfImport` | `V-SPAWN-SELF-IMPORT` | a template a service spawns imports that service | import a resource the service needs, not the service |
| `SpawnKind` | `V-SPAWN-KIND` | `spawns` on a node that is not a service | only services spawn |
| `LockNeeds` | `V-LOCK-NEEDS` | `exclusive` / `shared` names a resource the node does not need | `.needs` that resource first |
| `IdempotentRequired` | `V-IDEMPOTENT-REQUIRED` | retry, unknown-outcome recovery, or retrying an effect without `idempotent` | `.idempotent()` |
| `PoolStarve` | `V-POOL-STARVE` | a resident holder can starve the pool | do not hold a pool slot in a service |
| `UnusedPool` | `V-UNUSED-POOL` | a pool nobody uses | remove it, or `.on` / `.limit` it |
| `ServiceUnbounded` | `V-SERVICE-UNBOUNDED` | `Shutdown::unbounded()` and a service with no `stop_within` | bound the shutdown, or bound the stop |
| `TryUnconsumed` | `V-TRY-UNCONSUMED` | a try-step nothing depends on | add a node that `needs` it |
| `BudgetOrder` | `V-BUDGET-ORDER` | a stop or child budget larger than the one that contains it | shrink the inner bound |
| `Mode` | `V-MODE` | `Mode::Finite` on a plan with a service or a template | `Mode::Resident` |
| `BlockingCancel` | `V-BLOCKING-CANCEL` | `cooperative` on a blocking step | omit it — a thread is signalled and never aborted |
| `BlockingLimit` | `V-BLOCKING-LIMIT` | both `.limit` and `.on` on a blocking step | remove `.limit`; set the concurrency cap with `.on(pool)` |
| `RecoveryMissing` | `V-RECOVERY-MISSING` | an effect requests recovery without an identity and handler | add `.identified_by(key)` and `.recover_unknown(handler)` |

## After a run → `Report`

`Report` carries the outcome, the exported value, faults, cleanup
failures, abandoned obligations, and ambiguous effects.
`Report::into_result` is `Ok` only when every list is empty and the
outcome is `Ok`. The error *is* the report.
Its success is `Option<Arc<Out>>`, so a clean run may still have no output.

Use `into_required_output()` when output is required. It returns `Arc<Out>` or
`RequiredOutputError::Failed(report)` / `MissingOutput(report)`. Failure wins
over missing output; a successful outcome alone does not hide cleanup failures,
incomplete work or ambiguity. Both variants retain the whole report and its
original typed errors. Use `report()` to inspect it or `into_report()` to take it
back. Convert to text only at a boundary that requires text.

```rust,guide:required_output
use sdax::prelude::*;
use std::sync::Arc;

#[test]
fn required_output_distinguishes_all_three_cases() {
    let mut success = Report::empty(Outcome::Ok);
    success.output = Some(Arc::new(42));
    assert_eq!(
        *success.into_required_output().expect("completed value"),
        42
    );

    let missing = Report::<u32>::empty(Outcome::Ok)
        .into_required_output()
        .expect_err("no output");
    match missing {
        RequiredOutputError::MissingOutput(report) => assert!(report.is_clean()),
        RequiredOutputError::Failed(_) => panic!("the run was clean"),
    }

    let failed = Report::<u32>::empty(Outcome::Cancelled)
        .into_required_output()
        .expect_err("cancelled");
    assert!(matches!(&failed, RequiredOutputError::Failed(_)));
    assert_eq!(failed.report().outcome, Outcome::Cancelled);
    let original = failed.into_report();
    assert_eq!(original.outcome, Outcome::Cancelled);
}
```

`Outcome` is `Ok`, `Failed`, or `Cancelled`. A cleanup failure is a
record, not a lost exception. `FailFast` stops the run at the first
fault; `Isolate` aggregates and continues.

The [Cleanup](Cleanup.md) test is the run-time half: `Boom` faults,
`Conn` still releases, `outcome` is `Failed`.
