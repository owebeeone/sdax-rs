# Inspect and simulate

`inspect()` is pure. It reads the declaration and nothing else — no
body runs. The test below names the two nodes, the one declared
`needs` edge, the reverse cleanup order, and `why` each node waits.
The view also prints as text, answers `layers()` (earliest-start
layering, not a barrier), lists what is at the ship boundary
(`effects()`), and diffs against another view.

`release_order()` is the partial order of cleanup: `before(a, b)`
means *a*'s obligation finishes before *b*'s starts. Unrelated nodes
may overlap.

`Plan::simulate` runs a script of body outcomes on a virtual clock.
The interleaving is an input of the run, so a test of *your* plan is
reproducible. A plan that declares a per-run input wants
`simulate_with_input(input, &script)` instead — `simulate` refuses
those. Neither call runs a body or takes an effect. This is the
author-facing simulator, not a tour of the dev-only testkit.

`inspect()` does not list the input as a node. The value is in the
run's slots before the first step, so there is nothing to wait for.

```rust,guide:inspect_plan
use sdax::prelude::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

struct Conn;

#[test]
fn inspect_and_simulate_a_finite_plan() {
    // Nothing here is ever started, so nothing may touch this counter.
    let bodies_run = Arc::new(AtomicUsize::new(0));
    let (a, r, s) = (bodies_run.clone(), bodies_run.clone(), bodies_run.clone());

    let mut p = Plan::builder("Startup");
    let conn = p
        .resource("Conn")
        .acquire(move |cx, ()| {
            a.fetch_add(1, Ordering::SeqCst);
            async move { Ok(cx.hold_value(Conn)) }
        })
        .release(move |_cx, _c: Arc<Conn>| {
            r.fetch_add(1, Ordering::SeqCst);
            async move { Ok(()) }
        });
    p.step("Ping").needs(conn).run(move |_cx, _c: Arc<Conn>| {
        s.fetch_add(1, Ordering::SeqCst);
        async move { Ok(()) }
    });
    let plan = p
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(10)),
            Mode::Finite,
        )
        .expect("valid");

    let view = plan.inspect();
    assert_eq!(view.mode, Mode::Finite);
    let names: Vec<&str> = view.nodes.iter().map(|n| n.path.leaf()).collect();
    assert_eq!(names, vec!["Conn", "Ping"]);

    // Exactly the declared edges: one, `Ping` needs `Conn`.
    assert_eq!(view.edges.len(), 1);
    assert_eq!(view.edges[0].from.leaf(), "Ping");
    assert_eq!(view.edges[0].to.leaf(), "Conn");
    assert_eq!(view.edges[0].reason, Reason::DeclaredNeed);

    // Cleanup is the reverse of `needs`.
    assert!(view.release_order().before("Ping", "Conn"));

    let why = view.why("Ping").expect("Ping is in the plan");
    assert_eq!(why.waits_on.len(), 1);
    assert_eq!(why.waits_on[0].0.leaf(), "Conn");
    assert!(view.why("Conn").expect("Conn is too").waits_on.is_empty());

    let trace = plan.simulate(&Script::new()).expect("script");
    assert!(!trace.events.is_empty());

    assert_eq!(
        bodies_run.load(Ordering::SeqCst),
        0,
        "inspect() and simulate() run no body and take no effect"
    );
}
```
