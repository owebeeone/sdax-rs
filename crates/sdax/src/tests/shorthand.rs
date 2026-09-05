//! Adoption A1 — the positional constructors are sugar: they must record the
//! same declaration as the chain form, witnessed by comparing `inspect()`.

use super::corpus::*;
use crate::*;
use std::sync::Arc;

/// I-01 written positionally.
fn i01_positional() -> Result<Plan, Invalid> {
    let mut p = Plan::builder("Startup");
    let transport = p.resource_with(
        "Transport",
        (),
        |cx, ()| async move { Ok(cx.hold_value(Transport { addr: "t" })) },
        |_cx, _t| async move { Ok(()) },
    );
    let peers = p.resource_with(
        "PeerStore",
        transport,
        |cx, _t: Arc<Transport>| async move { Ok(cx.hold_value(PeerStore)) },
        |_cx, _s| async move { Ok(()) },
    );
    let routes = p.resource_with(
        "RoutingTable",
        transport,
        |cx, _t: Arc<Transport>| async move { Ok(cx.hold_value(RoutingTable)) },
        release::by_drop(),
    );
    p.effect_with(
        "Registration",
        (peers, routes),
        Ambiguity::Report,
        |cx, _d: (Arc<PeerStore>, Arc<RoutingTable>)| async move { Ok(cx.hold_value(Receipt("r"))) },
        |_cx, _r| async move { Ok(()) },
    );
    p.build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Finite)
}

#[test]
fn the_positional_form_records_the_same_declaration_as_the_chain_form() {
    let chained = i01().expect("valid").inspect();
    let positional = i01_positional().expect("valid").inspect();
    assert_eq!(chained, positional);
    assert_eq!(chained.to_string(), positional.to_string());
}

#[test]
fn every_body_carrying_kind_has_a_positional_form() {
    let mut p = Plan::builder("All kinds");
    let cpu = p.pool("cpu", 2);
    let db = p.resource_with(
        "Db",
        (),
        |cx, ()| async move { Ok(cx.hold_value(Db)) },
        |_cx, _d| async move { Ok(()) },
    );
    let migrate = p.step_with("Migrate", db, |_cx, _d: Arc<Db>| async move { Ok(()) });
    let probe = p.try_step_with("Probe", db, |_cx, _d: Arc<Db>| async move { Ok(1u8) });
    let verify = p.blocking_step_with("Verify", db, cpu, |_cx, _d: Arc<Db>| Ok(()));
    let _decide = p.step_with("Decide", probe, |_cx, _r| async move { Ok(()) });
    let _api = p.service_with(
        "Api",
        (migrate, verify),
        |_cx, _d: (Arc<()>, Arc<()>)| async move { Ok(Serving::new((), async { Ok(()) })) },
    );
    let _receipt = p.effect_persistent_with(
        "Audit",
        db,
        Ambiguity::Report,
        |cx, _d: Arc<Db>| async move { Ok(cx.hold_value(Receipt("r"))) },
    );
    let plan = p
        .build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Resident)
        .expect("valid");
    let v = plan.inspect();
    let kinds: Vec<String> = v.nodes.iter().map(crate::view::kind_label).collect();
    assert_eq!(
        kinds,
        [
            "resource",
            "step",
            "try_step",
            "blocking_step",
            "step",
            "service",
            "effect (persistent)"
        ]
    );
    assert!(v.node("Api").unwrap().needs.iter().any(|n| *n == *"Verify"));
}

#[test]
fn a_positional_node_can_still_take_attributes_through_the_chain_form() {
    // The shorthand is for the common case; anything with attributes uses the
    // chain, and the two coexist in one plan.
    let mut p = Plan::builder("Mixed");
    let db = p.resource_with(
        "Db",
        (),
        |cx, ()| async move { Ok(cx.hold_value(Db)) },
        |_cx, _d| async move { Ok(()) },
    );
    p.step("Migrate")
        .needs(db)
        .within(secs(2))
        .run(|_cx, _d: Arc<Db>| async move { Ok(()) });
    let plan = p
        .build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Finite)
        .expect("valid");
    assert_eq!(
        plan.inspect().node("Migrate").unwrap().attr("within"),
        Some("2s")
    );
}
