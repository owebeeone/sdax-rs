//! Suite (c), faults and policies: C-08, C-08b, C-09, C-20, C-21, C-23,
//! C-52, C-59, C-66, C-67.

use crate::corpus::*;
use crate::Drv as ScriptedDriver;
use sdax::*;
use sdax_testkit::eol::*;

fn faults_of<O>(d: &sdax_testkit::Driven<O>) -> Vec<String> {
    d.report.faults.iter().map(|f| f.node.to_string()).collect()
}

fn c08_script(c_held: f64) -> Script {
    Script::new()
        .prepare("Base", Body::ok(At::tick(0.0)))
        .prepare("A", Body::ok(At::tick(1.0)))
        .prepare("B", Body::fail(At::tick(2.0), "boom"))
        .prepare("C", Body::ok(At::tick(5.0)).held(At::tick(c_held)))
        .cleanup("A", Cleanup::Ok(secs(1)))
        .cleanup("Base", Cleanup::Ok(secs(1)))
        .cleanup("C", Cleanup::Ok(secs(1)))
}

#[test]
fn c08_fail_fast_cancels_the_in_flight_sibling_and_releases_what_was_acquired() {
    let d = ScriptedDriver::run(&i08(Policy::FailFast), &c08_script(4.0)).expect("runs");
    d.check();
    let t = d.eol();
    assert!(t.start("Down").is_none(), "never(start(Down))");
    assert_eq!(
        t.at(is_interrupted, "C"),
        Some(2.0),
        "cancelled at the fault, not at 5"
    );
    assert_eq!(
        t.interrupted("C"),
        Some(false),
        "the effect had not completed"
    );
    assert!(t.cleanup_start("C").is_none(), "nothing to release");
    assert!(t.cleanup_start("A").is_some() && t.cleanup_start("Base").is_some());
    assert!(t.pos(is_cleanup_end, "A") < t.pos(is_cleanup_start, "Base"));
    assert_eq!(d.report.outcome, Outcome::Failed);
    assert_eq!(faults_of(&d), ["B"], "C is interrupted, never a fault");
    assert_eq!(t.skipped_because("Down").as_deref(), Some("B"));
}

#[test]
fn c08b_a_sibling_that_held_before_the_fault_is_released() {
    let d = ScriptedDriver::run(&i08(Policy::FailFast), &c08_script(1.0)).expect("runs");
    d.check();
    let t = d.eol();
    assert_eq!(t.interrupted("C"), Some(true));
    assert!(t.cleanup_start("C").is_some(), "cleanup(C)");
    assert!(t.pos(is_cleanup_end, "C") < t.pos(is_cleanup_start, "Base"));
    assert_eq!(faults_of(&d), ["B"]);
    assert_eq!(d.report.outcome, Outcome::Failed);
}

#[test]
fn c09_a_resource_that_held_and_then_failed_is_released() {
    let script = Script::new().prepare(
        "Port",
        Body::fail(At::tick(2.0), "policy").held(At::tick(1.0)),
    );
    let d = ScriptedDriver::run(&i09(), &script).expect("runs");
    d.check();
    let t = d.eol();
    assert_eq!(t.held("Port"), Some(1.0));
    assert!(t.ready("Port").is_none(), "never became Ready");
    assert!(
        t.cleanup_start("Port").is_some(),
        "cleanup(Port) although never Ready"
    );
    assert!(t.start("Check").is_none());
    assert_eq!(faults_of(&d), ["Port"]);
    assert_eq!(d.report.outcome, Outcome::Failed);
}

fn c20_script(p1: Body) -> Script {
    Script::new()
        .prepare("A", Body::ok(At::tick(1.0)))
        .prepare("P1", p1)
        .prepare("P2", Body::ok(At::tick(5.0)))
        .cleanup("A", Cleanup::Ok(secs(1)))
}

#[test]
fn c20_isolate_lets_siblings_finish_and_still_releases() {
    let d =
        ScriptedDriver::run(&i20(), &c20_script(Body::fail(At::tick(3.0), "p1"))).expect("runs");
    d.check();
    let t = d.eol();
    assert_eq!(t.ready("P2"), Some(5.0), "ok(P2.run)");
    assert_eq!(
        t.cleanup_start("A"),
        Some(5.0),
        "after both P1 and P2 finished"
    );
    assert_eq!(faults_of(&d), ["P1"]);
    assert_eq!(d.report.outcome, Outcome::Failed);
}

#[test]
fn c21_two_faults_in_one_tick_are_both_reported() {
    let script = Script::new()
        .prepare("Base", Body::ok(At::tick(1.0)))
        .prepare("B", Body::fail(At::tick(2.0), "b"))
        .prepare("C", Body::fail(At::tick(2.0), "c"));
    let d = ScriptedDriver::run(&i21(), &script).expect("runs");
    d.check();
    assert_eq!(faults_of(&d), ["B", "C"]);
    assert!(d.eol().cleanup_start("Base").is_some());
    assert_eq!(d.report.outcome, Outcome::Failed);
}

#[test]
fn c23_try_step_failures_are_values_and_the_run_is_ok() {
    let script = Script::new()
        .prepare("Transport", Body::ok(At::tick(1.0)))
        .prepare("Probe1", Body::ok(At::tick(2.0)))
        .prepare("Probe2", Body::fail(At::tick(2.0), "p2"))
        .prepare("Probe3", Body::ok(At::tick(2.0)))
        .prepare("Probe4", Body::fail(At::tick(2.0), "p4"))
        .prepare("Probe5", Body::ok(At::tick(2.0)))
        .prepare("Decide", Body::ok(At::tick(3.0)));
    let d = ScriptedDriver::run(&i23(), &script).expect("runs");
    d.check();
    let t = d.eol();
    for p in ["Probe1", "Probe3", "Probe5"] {
        assert_eq!(t.ready(p), Some(2.0));
    }
    assert_eq!(
        t.start("Decide"),
        Some(2.0),
        "with the five results, two of them Err"
    );
    for n in [
        "Probe1",
        "Probe2",
        "Probe3",
        "Probe4",
        "Probe5",
        "Decide",
        "Transport",
    ] {
        assert!(t.interrupted(n).is_none(), "never(cancelled(*))");
    }
    assert_eq!(d.report.outcome, Outcome::Ok);
    assert!(d.report.is_clean(), "{}", d.report);
}

#[test]
fn c52_panics_are_faults_never_re_raised() {
    let script = c08_script(4.0).prepare("B", Body::panic(At::tick(2.0)));
    let d = ScriptedDriver::run(&i08(Policy::FailFast), &script).expect("runs");
    d.check();
    assert_eq!(faults_of(&d), ["B"]);
    assert_eq!(d.report.faults[0].kind.label(), FaultLabel::Panic);
    assert_eq!(d.report.panics().len(), 1);
    assert_eq!(d.eol().interrupted("C"), Some(false));

    let d = ScriptedDriver::run(&i20(), &c20_script(Body::panic(At::tick(3.0)))).expect("runs");
    d.check();
    assert_eq!(faults_of(&d), ["P1"]);
    assert_eq!(d.report.panics().len(), 1);
    assert_eq!(d.eol().ready("P2"), Some(5.0));
}

#[test]
fn c59_isolate_skips_the_transitive_dependents_only() {
    let script = Script::new()
        .prepare("A", Body::ok(At::plus(1.0)))
        .prepare("B", Body::fail(At::tick(2.0), "b"))
        .prepare("E", Body::ok(At::plus(3.0)));
    let d = ScriptedDriver::run(&chain_isolate(), &script).expect("runs");
    d.check();
    let t = d.eol();
    assert_eq!(t.skipped_because("C").as_deref(), Some("B"));
    assert_eq!(t.skipped_because("D").as_deref(), Some("B"));
    assert!(t.start("C").is_none() && t.start("D").is_none());
    assert_eq!(t.ready("E"), Some(3.0), "E runs");
    assert!(
        t.pos(is_ready, "E") < t.pos(is_cleanup_start, "A"),
        "cleanup(A) after E finished"
    );
    assert_eq!(faults_of(&d), ["B"]);
    assert_eq!(d.report.outcome, Outcome::Failed);
}

#[test]
fn c66_fail_fast_still_applies_to_the_resources_around_try_steps() {
    let script = Script::new().prepare("Transport", Body::fail(At::tick(1.0), "t"));
    let d = ScriptedDriver::run(&i23(), &script).expect("runs");
    d.check();
    let t = d.eol();
    for p in ["Probe1", "Probe2", "Probe3", "Probe4", "Probe5", "Decide"] {
        assert!(t.start(p).is_none(), "never(start({p}))");
    }
    assert_eq!(faults_of(&d), ["Transport"]);
    assert_eq!(d.report.outcome, Outcome::Failed);
}

#[test]
fn c67_a_service_whose_start_fails_is_never_ready_and_never_stopped() {
    let script = Script::new()
        .prepare("Endpoint", Body::ok(At::plus(1.0)))
        .prepare("AcceptLoop", Body::fail(At::tick(2.0), "bind"))
        .cleanup("Endpoint", Cleanup::Ok(secs(1)));
    let d = ScriptedDriver::run(&i05(), &script).expect("runs");
    d.check();
    let t = d.eol();
    assert!(t.ready("AcceptLoop").is_none());
    assert!(t.stop_requested("AcceptLoop").is_none(), "nothing to stop");
    assert!(t.start("PublishAddr").is_none());
    assert!(t.cleanup_start("Endpoint").is_some());
    assert_eq!(faults_of(&d), ["AcceptLoop"]);
    assert_eq!(d.report.outcome, Outcome::Failed);
}
