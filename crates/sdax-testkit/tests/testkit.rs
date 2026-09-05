//! What the harness itself must do (LBT-009: a test double is only useful if
//! it is contract-faithful).

use sdax::*;
use sdax_testkit::{invariants, FakeClock, TraceRecorder};
use std::sync::Arc;
use std::time::Duration;

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

struct Db;

fn i16_plan() -> Result<Plan, Invalid> {
    let mut p = Plan::builder("Concurrent cleanup");
    let transport = p
        .resource("Transport")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Db)) })
        .release(|_cx, _t| async move { Ok(()) });
    p.resource("PeerStore")
        .needs(transport)
        .acquire(|cx, _t: Arc<Db>| async move { Ok(cx.hold_value(Db)) })
        .release(|_cx, _s| async move { Ok(()) });
    p.resource("RoutingTable")
        .needs(transport)
        .acquire(|cx, _t: Arc<Db>| async move { Ok(cx.hold_value(Db)) })
        .release(|_cx, _r| async move { Ok(()) });
    p.build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Finite)
}

#[test]
fn the_fake_clock_satisfies_the_clock_contract() {
    let clock = FakeClock::new();
    assert_eq!(clock.now(), Time::ZERO);
    let violations = invariants::check_clock(clock.as_ref(), |d| clock.advance(d));
    assert!(violations.is_empty(), "{violations:?}");
}

#[test]
fn the_fake_clock_never_advances_by_itself() {
    let clock = FakeClock::new();
    let mut sleeping = Box::pin(clock.sleep(secs(5)));
    assert!(invariants::poll_once(sleeping.as_mut()).is_pending());
    clock.advance(secs(4));
    assert!(invariants::poll_once(sleeping.as_mut()).is_pending());
    clock.advance(secs(1));
    assert!(invariants::poll_once(sleeping.as_mut()).is_ready());
    assert_eq!(clock.now(), Time::from_nanos(5_000_000_000));
}

#[test]
fn the_trace_recorder_keeps_observation_order() {
    let rec = TraceRecorder::new();
    rec.event(&TraceEvent::at(Time::from_nanos(2), TraceKind::Ready));
    rec.event(&TraceEvent::at(
        Time::from_nanos(1),
        TraceKind::End(Outcome::Ok),
    ));
    let trace = rec.trace();
    assert_eq!(trace.events.len(), 2);
    assert_eq!(
        trace.events[0].kind,
        TraceKind::Ready,
        "order of observation, not of time"
    );
    assert!(rec.contains(&TraceKind::End(Outcome::Ok)));
    rec.clear();
    assert!(rec.trace().events.is_empty());
}

#[test]
fn the_static_checker_accepts_a_well_formed_plan() {
    let v = i16_plan().expect("valid").inspect();
    let violations = invariants::check_plan(&v);
    assert!(violations.is_empty(), "{violations:?}");
}

#[test]
fn the_static_checker_catches_an_edge_the_author_did_not_declare() {
    let mut v = i16_plan().expect("valid").inspect();
    v.edges.push(Edge {
        from: NodePath::root("PeerStore"),
        to: NodePath::root("RoutingTable"),
        reason: Reason::DeclaredNeed,
    });
    let violations = invariants::check_plan(&v);
    assert!(
        violations.iter().any(|x| x.rule == "INV-1"),
        "an undeclared edge must be caught: {violations:?}"
    );
}

#[test]
fn the_static_checker_holds_for_nested_and_dynamic_shapes() {
    // INV-5 and INV-6 cross-check the core's release order against a closure
    // this crate computes itself. They cannot be provoked through the public
    // API today — the order *is* derived from the edges — so their coverage is
    // the positive one: they must hold for every shape the surface can build.
    let mut inner = Plan::builder("Networking");
    let t = inner
        .resource("Transport")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Db)) })
        .release(|_cx, _t| async move { Ok(()) });
    inner
        .resource("PeerStore")
        .needs(t)
        .acquire(|cx, _t: Arc<Db>| async move { Ok(cx.hold_value(Db)) })
        .release(|_cx, _s| async move { Ok(()) });
    let net = inner
        .build(Policy::FailFast, Shutdown::within(secs(5)), Mode::Finite)
        .expect("valid");

    let mut p = Plan::builder("Process");
    let netk = p.component("Net", &net);
    p.effect("Registration")
        .needs(netk)
        .on_ambiguous(Ambiguity::Report)
        .perform(|cx, _n: Arc<()>| async move { Ok(cx.hold_value(Db)) })
        .compensate(|_cx, _r| async move { Ok(()) });
    let composed = p
        .build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Finite)
        .expect("valid");

    let v = composed.inspect();
    assert!(
        invariants::check_plan(&v).is_empty(),
        "{:?}",
        invariants::check_plan(&v)
    );
    let order = v.release_order();
    assert!(order.before("Registration", "Net"));
    assert!(order.before("Net/PeerStore", "Net/Transport"));
    assert!(
        order.before("Net/PeerStore", "Net"),
        "an inner node is cleaned up before its unit"
    );
}

#[test]
fn the_static_checker_agrees_with_the_views_unordered_pairs() {
    let v = i16_plan().expect("valid").inspect();
    let order = v.release_order();
    assert!(order.unordered("PeerStore", "RoutingTable"));
    let pairs: Vec<(String, String)> = order
        .unordered_pairs()
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect();
    assert_eq!(
        pairs,
        [("PeerStore".to_string(), "RoutingTable".to_string())]
    );
    assert!(invariants::check_plan(&v).is_empty());
}
