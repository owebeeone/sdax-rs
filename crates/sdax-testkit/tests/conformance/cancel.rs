//! Suite (c), cancellation: C-11, C-12, C-12b, C-26, C-55, C-60, C-68.
//! C-14 (drop of `Running`, the drainer) needs the Stage 2 driver and is
//! not here.

use crate::corpus::*;
use sdax::*;
use sdax_testkit::eol::*;
use sdax_testkit::ScriptedDriver;

#[test]
fn c11_cancel_before_start_has_no_effect_and_the_plan_is_reusable() {
    let plan = i11();
    let d = ScriptedDriver::run(&plan, &Script::new().at(0.0, Request::Cancel)).expect("runs");
    d.check();
    assert!(
        !d.effects().iter().any(|e| e.starts_with("Spawn")),
        "no-effect: {:?}",
        d.effects()
    );
    let t = d.eol();
    assert!(t.cleanup_start("A").is_none() && t.cleanup_start("B").is_none());
    assert_eq!(d.report.outcome, Outcome::Cancelled);
    assert!(!d.report.is_clean());

    // The same plan value starts a second run normally.
    let d = ScriptedDriver::run(&plan, &Script::new().at(5.0, Request::Shutdown)).expect("runs");
    d.check();
    assert_eq!(d.eol().ready("B"), Some(0.0));
    assert_eq!(d.report.outcome, Outcome::Ok);
}

fn c12_script(cancel_at: f64) -> Script {
    Script::new()
        .prepare("A", Body::ok(At::tick(1.0)))
        .prepare("B", Body::ok(At::tick(4.0)).held(At::tick(3.0)))
        .at(cancel_at, Request::Cancel)
}

#[test]
fn c12_cancel_during_an_acquisition_before_it_held() {
    let d = ScriptedDriver::run(&i12(), &c12_script(2.0)).expect("runs");
    d.check();
    let t = d.eol();
    assert_eq!(t.interrupted("B"), Some(false));
    assert!(t.cleanup_start("B").is_none(), "never(cleanup(B))");
    assert!(t.cleanup_start("A").is_some(), "cleanup(A)");
    assert!(t.start("C").is_none());
    assert_eq!(d.report.outcome, Outcome::Cancelled);
    assert!(d.report.faults.is_empty(), "cancelled ≠ failed");
}

#[test]
fn c12b_cancel_after_the_acquisition_held() {
    let d = ScriptedDriver::run(&i12(), &c12_script(3.5)).expect("runs");
    d.check();
    let t = d.eol();
    assert_eq!(t.interrupted("B"), Some(true));
    assert!(t.cleanup_start("B").is_some(), "cleanup(B)");
    assert!(t.pos(is_cleanup_end, "B") < t.pos(is_cleanup_start, "A"));
    assert_eq!(d.report.outcome, Outcome::Cancelled);
}

#[test]
fn c26_cancel_during_backoff_ends_the_wait_at_once() {
    let plan = i24(secs(4));
    let script = Script::new()
        .body(
            "Conn",
            [Body::fail(At::tick(0.0), "t1"), Body::ok(At::plus(0.0))],
        )
        .at(1.0, Request::Cancel);
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    let t = d.eol();
    assert_eq!(t.attempts("Conn"), 1, "never(start(Conn attempt 2))");
    assert!(
        d.effects().iter().any(|e| e.starts_with("CancelTimer")),
        "the backoff timer is cancelled: {:?}",
        d.effects()
    );
    assert_eq!(t.end().map(|(at, _)| at), Some(1.0), "end(run) at 1");
    assert_eq!(d.report.outcome, Outcome::Cancelled);
    assert_eq!(
        d.report.faults.len(),
        1,
        "the attempt that failed is reported"
    );
}

#[test]
fn c55_a_second_request_during_cleanup_does_not_interrupt_a_release() {
    let plan = i16(Mode::Resident, shutdown10());
    let script = Script::new()
        .at(5.0, Request::Shutdown)
        .cleanup("PeerStore", Cleanup::Ok(secs(4)))
        .at(6.0, Request::Cancel);
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    let t = d.eol();
    assert_eq!(t.cleanup_start("PeerStore"), Some(5.0));
    assert_eq!(
        t.cleanup_end("PeerStore"),
        Some(9.0),
        "not interrupted (INV-7)"
    );
    assert!(t.has_run_event(|k| matches!(k, TraceKind::RequestDuringCleanup)));
    assert_eq!(d.report.outcome, Outcome::Ok);
}

#[test]
fn c60_a_cooperative_effect_keeps_being_polled_within_its_grace() {
    let script = Script::new()
        .prepare("Registration", Body::ok(At::tick(3.0)).held(At::tick(3.0)))
        .at(2.5, Request::Cancel);
    let d = ScriptedDriver::run(&cooperative_effect(), &script).expect("runs");
    d.check();
    let t = d.eol();
    assert_eq!(
        t.held("Registration"),
        Some(3.0),
        "the body ran on after the cancel"
    );
    assert_eq!(t.interrupted("Registration"), Some(true));
    assert!(
        t.cleanup_start("Registration").is_some(),
        "the compensation runs"
    );
    assert!(d.report.ambiguous.is_empty(), "ambiguous: {{}}");
    assert!(
        d.effects().iter().any(|e| e.starts_with("Signal")),
        "signalled, not dropped: {:?}",
        d.effects()
    );
    assert_eq!(d.report.outcome, Outcome::Cancelled);
}

#[test]
fn c68_a_blocking_body_cannot_be_aborted_and_is_abandoned_at_the_budget() {
    let script = Script::new()
        .prepare("Snapshot", Body::ok(At::plus(1.0)))
        .prepare("Verify", Body::ok(At::plus(20.0)))
        .at(2.0, Request::Cancel);
    let d = ScriptedDriver::run(&i29(0), &script).expect("runs");
    d.check();
    let t = d.eol();
    assert_eq!(t.abandoned("Verify"), Some(12.0), "at the shutdown budget");
    let incomplete: Vec<String> = d
        .report
        .incomplete
        .iter()
        .map(|r| r.node.to_string())
        .collect();
    assert_eq!(incomplete, ["Verify"]);
    assert!(
        t.cleanup_start("Snapshot").is_some(),
        "cleanup(Snapshot) proceeds"
    );
    assert_eq!(d.report.outcome, Outcome::Cancelled);
    assert!(
        !d.effects()
            .iter()
            .any(|e| e.starts_with("Abort") && e.contains(&format!("{:?}", d.key("Verify")))),
        "a blocking body is never aborted"
    );
}
