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
