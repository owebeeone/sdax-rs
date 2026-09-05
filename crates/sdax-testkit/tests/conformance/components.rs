//! Suite (c), components, determinism and the seam's first-run signals:
//! C-32, C-58, C-33, C-38, C-50.
//!
//! C-50 (`DoubleHold`) as B wrote it is a first-run signal the driver
//! raises from `CxInner::hold_count`; the pure machine sees only the
//! `Event::NodeErr(_, FaultKind::DoubleHold)` the driver reports, and that
//! is what is tested here.

use crate::corpus::*;
use sdax::host::engine::{Effect, Event, Machine};
use sdax::*;
use sdax_testkit::eol::*;
use sdax_testkit::ScriptedDriver;

#[test]
fn c32_a_component_is_ready_when_its_inner_run_is_steady_and_cleans_up_as_a_unit() {
    let plan = i32();
    let mut script = Script::new();
    for n in [
        "Net/Transport",
        "Net/PeerStore",
        "Net/RoutingTable",
        "Net/Handle",
        "Registration",
    ] {
        script = script.prepare(n, Body::ok(At::plus(1.0)));
    }
    for n in [
        "Net/Transport",
        "Net/PeerStore",
        "Net/RoutingTable",
        "Registration",
    ] {
        script = script.cleanup(n, Cleanup::Ok(secs(1)));
    }
    let script = script.at(6.0, Request::Shutdown);
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    let t = d.eol();
    assert_eq!(
        t.ready("Net"),
        Some(3.0),
        "when the inner run reaches Steady"
    );
    assert!(t.pos(is_ready, "Net") < t.pos(is_start, "Registration"));
    assert!(t.pos(is_cleanup_end, "Registration") < t.pos(is_cleanup_start, "Net"));
    assert!(t.pos(is_cleanup_end, "Net/PeerStore") < t.pos(is_cleanup_start, "Net/Transport"));
    assert!(t.pos(is_cleanup_end, "Net/RoutingTable") < t.pos(is_cleanup_start, "Net/Transport"));
    assert!(t.pos(is_cleanup_start, "Net") <= t.pos(is_cleanup_start, "Net/PeerStore"));
    assert!(t.pos(is_cleanup_end, "Net/Transport") < t.pos(is_cleanup_end, "Net"));
    assert!(
        plan.inspect().node("Net/Transport").is_some(),
        "inner nodes are addressable"
    );
    assert_eq!(d.report.outcome, Outcome::Ok);
    assert!(d.report.is_clean(), "{}", d.report);
}

#[test]
fn c58_an_inner_fault_faults_the_component_and_the_outer_policy_applies() {
    let plan = i32();
    let script = Script::new()
        .prepare("Net/Transport", Body::ok(At::plus(1.0)))
        .prepare("Net/PeerStore", Body::fail(At::tick(2.0), "ps"))
        .prepare("Net/RoutingTable", Body::ok(At::plus(3.0)))
        .cleanup("Net/Transport", Cleanup::Ok(secs(1)))
        .at(20.0, Request::Shutdown);
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    let t = d.eol();
    let faults: Vec<String> = d.report.faults.iter().map(|f| f.node.to_string()).collect();
    assert_eq!(
        faults,
        ["Net", "Net/PeerStore"],
        "the component's own record first (F4)"
    );
    assert!(
        t.start("Registration").is_none(),
        "never(start(Registration))"
    );
    assert_eq!(
        t.interrupted("Net/RoutingTable"),
        Some(false),
        "inner in-flight cancelled"
    );
    assert!(
        t.cleanup_start("Net/Transport").is_some(),
        "inner releases run"
    );
    assert!(t.pos(is_cleanup_start, "Net") < t.pos(is_cleanup_start, "Net/Transport"));
    assert_eq!(d.report.outcome, Outcome::Failed);
    assert!(
        t.end().map(|(at, _)| at).unwrap() < 20.0,
        "the outer FailFast settled the run"
    );
}

#[test]
fn c33_inspection_before_a_run_has_no_effect() {
    let plan = i01(Mode::Finite);
    let view = plan.inspect();
    assert_eq!(view.nodes.len(), 4);
    assert_eq!(view.edges.len(), 4);
    assert_eq!(view.layers().len(), 3);
    let why = view.why("Registration").expect("Registration");
    let names: Vec<String> = why.waits_on.iter().map(|(p, _)| p.to_string()).collect();
    assert_eq!(names, ["PeerStore", "RoutingTable"]);
    assert!(view.release_order().before("Registration", "Transport"));
    // No machine was built, so no effect list exists at all; a machine that
    // *is* built emits nothing before `begin`.
    let mut m = Machine::new(&plan).expect("static");
    assert!(m
        .step(Event::ShutdownRequested)
        .iter()
        .all(|e| !matches!(e, Effect::Spawn { .. })));
}

#[test]
fn c38_the_same_script_gives_a_byte_identical_trace_and_another_schedule_another() {
    let plan = i08(Policy::FailFast);
    let script = Script::new()
        .prepare("Base", Body::ok(At::tick(0.0)))
        .prepare("A", Body::ok(At::tick(1.0)))
        .prepare("B", Body::fail(At::tick(2.0), "boom"))
        .prepare("C", Body::ok(At::tick(5.0)).held(At::tick(4.0)));
    let one = ScriptedDriver::run(&plan, &script).expect("runs");
    let two = ScriptedDriver::run(&plan, &script).expect("runs");
    one.check();
    two.check();
    assert_eq!(format!("{:?}", one.trace), format!("{:?}", two.trace));
    assert_eq!(one.effects(), two.effects());
    assert_eq!(
        one.eol().interrupted("C"),
        Some(false),
        "C-08's expectation holds"
    );
    assert_eq!(one.report.outcome, Outcome::Failed);

    let mirrored = script.schedule(Schedule::order(["C", "B", "A"]));
    let three = ScriptedDriver::run(&plan, &mirrored).expect("runs");
    let four = ScriptedDriver::run(&plan, &mirrored).expect("runs");
    three.check();
    assert_eq!(format!("{:?}", three.trace), format!("{:?}", four.trace));
    assert_ne!(
        one.effects(),
        three.effects(),
        "another schedule, another trace"
    );
    assert_eq!(three.report.outcome, Outcome::Failed);
    let t = three.eol();
    assert!(
        t.pos(is_start, "C") < t.pos(is_start, "A"),
        "C is spawned first"
    );
}

#[test]
fn c50_a_double_hold_is_a_fault_and_the_first_value_is_released() {
    let plan = i09();
    let mut m = Machine::new(&plan).expect("static");
    m.begin();
    let port = m.key_of("Port").expect("Port");
    m.step(Event::Held(port));
    let fx = m.step(Event::NodeErr(port, FaultKind::DoubleHold));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Release(k) if *k == port)),
        "the first value is released: {fx:?}"
    );
    let fx = m.step(Event::NodeOk(port));
    assert!(
        fx.iter().any(|e| matches!(e, Effect::End(Outcome::Failed))),
        "{fx:?}"
    );
    let report = m.take_report().expect("ended");
    assert_eq!(report.faults.len(), 1);
    assert_eq!(report.faults[0].kind.label(), FaultLabel::DoubleHold);
    assert_eq!(report.faults[0].node.to_string(), "Port");
}
