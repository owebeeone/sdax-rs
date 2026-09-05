//! Suite (c), cleanup and the budget: C-15, C-16, C-18, C-53, C-54, C-56,
//! C-69.

use crate::corpus::*;
use sdax::*;
use sdax_testkit::eol::*;
use sdax_testkit::ScriptedDriver;

fn incomplete_of<O>(d: &sdax_testkit::Driven<O>) -> Vec<String> {
    d.report
        .incomplete
        .iter()
        .map(|r| r.node.to_string())
        .collect()
}

fn cleanup_failures_of<O>(d: &sdax_testkit::Driven<O>) -> Vec<String> {
    d.report
        .cleanup_failures
        .iter()
        .map(|f| f.node.to_string())
        .collect()
}

#[test]
fn c15_the_transport_is_alive_during_the_compensation() {
    let plan = i15(Mode::Resident, None, Ambiguity::Report);
    let script = Script::new()
        .prepare("Transport", Body::ok(At::plus(1.0)))
        .prepare("Registration", Body::ok(At::plus(1.0)))
        .at(5.0, Request::Shutdown)
        .cleanup("Registration", Cleanup::Ok(secs(2)))
        .cleanup("Transport", Cleanup::Ok(secs(1)));
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    let t = d.eol();
    assert!(t.pos(is_cleanup_end, "Registration") < t.pos(is_cleanup_start, "Transport"));
    assert_eq!(t.cleanup_end("Registration"), Some(7.0));
    assert_eq!(
        t.cleanup_start("Transport"),
        Some(7.0),
        "starts after the compensation ends"
    );
    assert_eq!(d.report.outcome, Outcome::Ok);
}

#[test]
fn c16_unrelated_releases_overlap_and_both_precede_the_transport() {
    let plan = i16(Mode::Resident, shutdown10());
    let script = Script::new()
        .prepare("Transport", Body::ok(At::plus(1.0)))
        .prepare("PeerStore", Body::ok(At::plus(1.0)))
        .prepare("RoutingTable", Body::ok(At::plus(1.0)))
        .at(5.0, Request::Shutdown)
        .cleanup("PeerStore", Cleanup::Ok(secs(3)))
        .cleanup("RoutingTable", Cleanup::Ok(secs(1)))
        .cleanup("Transport", Cleanup::Ok(secs(1)));
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    let t = d.eol();
    assert_eq!(t.cleanup_start("PeerStore"), Some(5.0));
    assert_eq!(
        t.cleanup_start("RoutingTable"),
        Some(5.0),
        "unordered: both start at 5"
    );
    assert_eq!(t.cleanup_start("Transport"), Some(8.0));
    assert!(t.pos(is_cleanup_end, "PeerStore") < t.pos(is_cleanup_start, "Transport"));
    assert!(t.pos(is_cleanup_end, "RoutingTable") < t.pos(is_cleanup_start, "Transport"));
    assert!(plan
        .inspect()
        .release_order()
        .unordered("PeerStore", "RoutingTable"));
    assert_eq!(d.report.outcome, Outcome::Ok);
}

#[test]
fn c18_a_worker_that_ignores_stop_is_abandoned_at_its_budget_and_named() {
    let script = Script::new()
        .prepare("Db", Body::ok(At::tick(1.0)))
        .prepare("Worker", Body::ok(At::tick(2.0)))
        .at(10.0, Request::Shutdown)
        .serve("Worker", [Serve::IgnoreStop])
        .cleanup("Db", Cleanup::Ok(secs(2)));
    let d = ScriptedDriver::run(&i18(), &script).expect("runs");
    d.check();
    let t = d.eol();
    assert_eq!(t.stop_requested("Worker"), Some(10.0));
    assert_eq!(
        t.abandoned("Worker"),
        Some(13.0),
        "bounded(stop(Worker), 3)"
    );
    assert_eq!(t.cleanup_start("Db"), Some(13.0));
    assert_eq!(t.cleanup_end("Db"), Some(15.0));
    assert_eq!(
        t.end().map(|(at, _)| at),
        Some(15.0),
        "bounded(shutdown, 10)"
    );
    assert_eq!(incomplete_of(&d), ["Worker"]);
    assert_eq!(d.report.outcome, Outcome::Ok);
    assert!(!d.report.is_clean(), "never silent");
}

#[test]
fn c53_a_panicking_release_is_a_cleanup_failure_and_siblings_proceed() {
    let plan = i16(Mode::Resident, shutdown10());
    let script = Script::new()
        .at(5.0, Request::Shutdown)
        .cleanup("PeerStore", Cleanup::Panic(secs(1)))
        .cleanup("RoutingTable", Cleanup::Ok(secs(1)))
        .cleanup("Transport", Cleanup::Ok(secs(1)));
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    let t = d.eol();
    assert_eq!(cleanup_failures_of(&d), ["PeerStore"]);
    assert_eq!(d.report.cleanup_failures[0].kind.label(), FaultLabel::Panic);
    assert_eq!(t.cleanup_end("RoutingTable"), Some(6.0), "unaffected");
    assert!(t.pos(is_cleanup_end, "PeerStore") < t.pos(is_cleanup_start, "Transport"));
    assert!(t.pos(is_cleanup_end, "RoutingTable") < t.pos(is_cleanup_start, "Transport"));
    assert_eq!(d.report.outcome, Outcome::Ok);
    assert!(!d.report.is_clean());
}

#[test]
fn c54_a_failed_compensation_still_ends_before_the_transport_is_released() {
    let plan = i15(Mode::Resident, None, Ambiguity::Report);
    let script = Script::new()
        .at(5.0, Request::Shutdown)
        .cleanup("Registration", Cleanup::Fail(secs(1), "dereg".into()))
        .cleanup("Transport", Cleanup::Ok(secs(1)));
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    let t = d.eol();
    assert_eq!(t.cleanup_end("Registration"), Some(6.0));
    assert_eq!(
        t.cleanup_start("Transport"),
        Some(6.0),
        "after the failed compensation ended"
    );
    assert_eq!(cleanup_failures_of(&d), ["Registration"]);
    assert_eq!(d.report.outcome, Outcome::Ok);
    assert!(!d.report.is_clean());
}

#[test]
fn c56_the_budget_abandons_what_is_still_running_and_keeps_the_order() {
    let plan = i16(Mode::Resident, Shutdown::within(secs(3)));
    let script = Script::new()
        .at(5.0, Request::Shutdown)
        .cleanup("PeerStore", Cleanup::Ok(secs(5)))
        .cleanup("RoutingTable", Cleanup::Ok(secs(1)))
        .cleanup("Transport", Cleanup::Ok(secs(1)));
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    let t = d.eol();
    assert_eq!(
        t.abandoned("PeerStore"),
        Some(8.0),
        "the budget expires at 8"
    );
    assert_eq!(t.cleanup_end("RoutingTable"), Some(6.0));
    assert_eq!(t.cleanup_start("Transport"), Some(8.0));
    assert_eq!(t.abandoned("Transport"), Some(8.0), "cannot finish in 0");
    assert_eq!(
        incomplete_of(&d),
        ["Transport", "PeerStore"],
        "declaration order"
    );
    assert_eq!(t.end().map(|(at, _)| at), Some(8.0), "bounded(shutdown, 3)");
    assert!(t.pos(is_cleanup_end, "PeerStore") < t.pos(is_cleanup_start, "Transport"));
    assert_eq!(d.report.outcome, Outcome::Ok);
}

#[test]
fn c69_a_release_that_ignores_stop_is_abandoned_at_the_budget() {
    let script = Script::new()
        .prepare("Db", Body::ok(At::tick(1.0)))
        .prepare("Worker", Body::ok(At::tick(2.0)))
        .at(10.0, Request::Shutdown)
        .serve("Worker", [Serve::IgnoreStop])
        .cleanup("Db", Cleanup::IgnoreStop);
    let d = ScriptedDriver::run(&i18(), &script).expect("runs");
    d.check();
    let t = d.eol();
    assert_eq!(t.abandoned("Worker"), Some(13.0));
    assert_eq!(t.abandoned("Db"), Some(20.0), "at the budget");
    assert_eq!(
        incomplete_of(&d),
        ["Db", "Worker"],
        "declaration order (H5)"
    );
    assert_eq!(t.end().map(|(at, _)| at), Some(20.0));
    assert!(!d.report.is_clean());
}
