//! Suite (c), retries, restarts and timeouts: C-24, C-24b, C-40, C-57, C-61.

use crate::corpus::*;
use crate::Drv as ScriptedDriver;
use sdax::*;
use sdax_testkit::eol::*;

fn timers<O>(d: &sdax_testkit::Driven<O>) -> Vec<String> {
    d.effects()
        .into_iter()
        .filter(|e| e.starts_with("Timer"))
        .collect()
}

#[test]
fn c24_attempts_are_bracketed_and_backoff_is_a_timer_on_the_fake_clock() {
    let plan = i24(secs(1));
    let script = Script::new()
        .body(
            "Conn",
            [
                Body::fail(At::tick(0.0), "t1").held(At::tick(0.0)),
                Body::fail(At::tick(1.0), "t2"),
                Body::ok(At::tick(3.0)),
            ],
        )
        .cleanup("Conn", Cleanup::Ok(secs(0)));
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    let t = d.eol();
    assert_eq!(t.attempts("Conn"), 3);
    assert_eq!(t.start_attempt("Conn", 2), Some(1.0));
    assert_eq!(t.start_attempt("Conn", 3), Some(3.0));
    // Attempt 1 held and failed: its release ends before attempt 2 starts.
    let first_release_end = t.pos(is_cleanup_end, "Conn");
    let second_start = d
        .trace
        .events
        .iter()
        .position(|e| is_start(&e.kind) && e.order.as_ref().map(|o| o.attempt) == Some(2));
    assert!(
        first_release_end < second_start,
        "before(cleanup(Conn 1), start(Conn 2))"
    );
    let waits = timers(&d);
    assert!(
        waits.iter().any(|w| w.contains("at: Time(1000000000)")),
        "{waits:?}"
    );
    assert!(
        waits.iter().any(|w| w.contains("at: Time(3000000000)")),
        "{waits:?}"
    );
    assert_eq!(d.report.outcome, Outcome::Ok);
    assert!(
        d.report.is_clean(),
        "absorbed attempts are in the trace, not the report"
    );
    assert_eq!(t.count(is_fail, "Conn"), 2);
}

#[test]
fn c24b_when_every_attempt_fails_each_is_reported() {
    let plan = i24(secs(1));
    let script = Script::new().body(
        "Conn",
        [
            Body::fail(At::tick(0.0), "t1").held(At::tick(0.0)),
            Body::fail(At::tick(1.0), "t2"),
            Body::fail(At::tick(3.0), "t3"),
        ],
    );
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    assert_eq!(d.eol().attempts("Conn"), 3);
    let attempts: Vec<u32> = d.report.faults.iter().map(|f| f.order.attempt).collect();
    assert_eq!(attempts, [1, 2, 3]);
    assert_eq!(d.report.outcome, Outcome::Failed);
}

#[test]
fn c40_a_service_restarts_idempotently_across_a_partition() {
    let script = Script::new()
        .prepare("Transport", Body::ok(At::tick(1.0)))
        .prepare("LocalReg", Body::ok(At::tick(2.0)))
        .body(
            "Renew",
            [
                Body::ok(At::tick(2.0)),
                Body::ok(At::tick(4.0)),
                Body::ok(At::tick(6.0)),
                Body::ok(At::tick(10.0)),
            ],
        )
        .serve(
            "Renew",
            [
                Serve::Err(At::tick(3.0), "partition".into()),
                Serve::Err(At::tick(4.0), "partition".into()),
                Serve::Err(At::tick(6.0), "partition".into()),
                Serve::StopsAfter(secs(1)),
            ],
        )
        .at(12.0, Request::Shutdown)
        .cleanup("LocalReg", Cleanup::Ok(secs(1)))
        .cleanup("Transport", Cleanup::Ok(secs(1)));
    let d = ScriptedDriver::run(&i40(), &script).expect("runs");
    d.check();
    let t = d.eol();
    assert_eq!(t.attempts("Renew"), 4);
    assert_eq!(t.start_attempt("Renew", 2), Some(4.0));
    assert_eq!(t.start_attempt("Renew", 3), Some(6.0));
    assert_eq!(t.start_attempt("Renew", 4), Some(10.0));
    assert_eq!(t.attempts("LocalReg"), 1, "never(duplicate registration)");
    assert!(
        t.cleanup_start("LocalReg").unwrap() >= 12.0,
        "acquired throughout"
    );
    assert_eq!(t.stop_requested("Renew"), Some(12.0));
    assert_eq!(
        t.cleanup_end("Renew"),
        Some(13.0),
        "bounded(stop(Renew), 2)"
    );
    assert!(t.pos(is_cleanup_end, "Renew") < t.pos(is_cleanup_start, "LocalReg"));
    assert!(t.pos(is_cleanup_end, "LocalReg") < t.pos(is_cleanup_start, "Transport"));
    assert_eq!(d.report.outcome, Outcome::Ok);
    assert!(d.report.is_clean(), "{}", d.report);
}

#[test]
fn c57_a_restarted_service_does_not_stop_or_restart_its_dependents() {
    let script = Script::new()
        .prepare("Migrate", Body::ok(At::plus(1.0)))
        .body(
            "Exporter",
            [Body::ok(At::plus(1.0)), Body::ok(At::tick(5.0))],
        )
        .prepare("Api", Body::ok(At::plus(1.0)))
        .serve(
            "Exporter",
            [
                Serve::Err(At::tick(4.0), "e".into()),
                Serve::StopsAfter(secs(0)),
            ],
        )
        .at(10.0, Request::Shutdown);
    let d = ScriptedDriver::run(&i07(true), &script).expect("runs");
    d.check();
    let t = d.eol();
    assert_eq!(t.attempts("Api"), 1);
    assert!(
        t.stop_requested("Api").unwrap() >= 10.0,
        "Api is neither stopped nor restarted"
    );
    assert_eq!(
        t.count(is_ready, "Exporter"),
        2,
        "ready(Exporter) is emitted again"
    );
    assert_eq!(t.last_at(is_ready, "Exporter"), Some(5.0));
    assert_eq!(
        t.count(is_fail, "Exporter"),
        1,
        "the restart is in the trace"
    );
    assert_eq!(d.report.outcome, Outcome::Ok);
    assert!(d.report.is_clean());
}

#[test]
fn c61_a_timed_out_effect_is_ambiguous_and_compensated_only_when_declared() {
    let script = Script::new().prepare("Registration", Body::pending());
    let d = ScriptedDriver::run(
        &i15(Mode::Resident, Some(secs(2)), Ambiguity::Report),
        &script,
    )
    .expect("runs");
    d.check();
    let t = d.eol();
    assert_eq!(t.at(is_ambiguous, "Registration"), Some(2.0));
    assert!(t.cleanup_start("Registration").is_none(), "no compensation");
    let amb: Vec<String> = d
        .report
        .ambiguous
        .iter()
        .map(|r| r.node.to_string())
        .collect();
    assert_eq!(amb, ["Registration"]);
    assert_eq!(d.report.outcome, Outcome::Failed);
    assert_eq!(d.report.faults.len(), 1);
    assert_eq!(d.report.faults[0].kind.label(), FaultLabel::Timeout);

    let d = ScriptedDriver::run(
        &i15(Mode::Resident, Some(secs(2)), Ambiguity::Compensate),
        &script,
    )
    .expect("runs");
    d.check();
    let t = d.eol();
    assert!(
        t.cleanup_start("Registration").is_some(),
        "the compensation runs"
    );
    let amb: Vec<String> = d
        .report
        .ambiguous
        .iter()
        .map(|r| r.node.to_string())
        .collect();
    assert_eq!(amb, ["Registration"], "still listed (INV-11)");
    assert!(t.pos(is_cleanup_end, "Registration") < t.pos(is_cleanup_start, "Transport"));
}
