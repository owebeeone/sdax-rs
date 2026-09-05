//! Suite (c), start ordering: C-01, C-02, C-04, C-05, C-07, C-27, C-29,
//! C-41, C-62, C-63. Every trace is checked by the invariant checker.

use crate::corpus::*;
use sdax::*;
use sdax_testkit::eol::*;
use sdax_testkit::ScriptedDriver;

fn ok_after(secs: f64) -> Body {
    Body::ok(At::plus(secs))
}

fn every_body(plan: &Plan, secs: f64) -> Script {
    let mut s = Script::new();
    for n in &plan.inspect().nodes {
        s = s.prepare(&n.path.to_string(), ok_after(secs));
    }
    s
}

#[test]
fn c01_dependency_ordered_startup_and_finite_cleanup() {
    let plan = i01(Mode::Finite);
    let d = ScriptedDriver::run(&plan, &every_body(&plan, 1.0)).expect("runs");
    d.check();
    let t = d.eol();
    assert!(t.pos(is_ready, "Transport") < t.pos(is_start, "PeerStore"));
    assert!(t.pos(is_ready, "Transport") < t.pos(is_start, "RoutingTable"));
    assert_eq!(t.start("PeerStore"), Some(1.0));
    assert_eq!(
        t.start("RoutingTable"),
        Some(1.0),
        "unordered: both start at tick 1"
    );
    assert!(t.pos(is_ready, "PeerStore") < t.pos(is_start, "Registration"));
    assert!(t.pos(is_ready, "RoutingTable") < t.pos(is_start, "Registration"));
    // Finite: Registration, then PeerStore ‖ RoutingTable, then Transport.
    assert!(t.pos(is_cleanup_end, "Registration") < t.pos(is_cleanup_start, "PeerStore"));
    assert!(t.pos(is_cleanup_end, "Registration") < t.pos(is_cleanup_start, "RoutingTable"));
    assert!(t.pos(is_cleanup_end, "PeerStore") < t.pos(is_cleanup_start, "Transport"));
    assert!(t.pos(is_cleanup_end, "RoutingTable") < t.pos(is_cleanup_start, "Transport"));
    assert_eq!(d.report.outcome, Outcome::Ok);
    assert!(d.report.is_clean(), "{}", d.report);
}

#[test]
fn c02_no_global_barriers_between_unrelated_chains() {
    let plan = i02();
    let script = every_body(&plan, 1.0).prepare("B", ok_after(10.0));
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    let t = d.eol();
    assert!(t.pos(is_ready, "X") < t.pos(is_start, "Y"));
    assert_eq!(t.start("Y"), Some(1.0));
    assert!(t.pos(is_start, "Y") < t.pos(is_ready, "B"));
    assert_eq!(t.ready("B"), Some(11.0));
    let why = plan.inspect().why("Y").expect("Y");
    assert_eq!(why.waits_on.len(), 1);
    assert_eq!(why.waits_on[0].0.to_string(), "X");
    assert_eq!(d.report.outcome, Outcome::Ok);
}

#[test]
fn c04_two_runs_of_one_plan_value_share_nothing() {
    let plan = i04();
    let r1 =
        ScriptedDriver::run(&plan, &Script::new().prepare("SvcA", ok_after(2.0))).expect("runs");
    let r2 =
        ScriptedDriver::run(&plan, &Script::new().prepare("SvcA", ok_after(1.0))).expect("runs");
    r1.check();
    r2.check();
    assert_eq!(r1.eol().ready("SvcA"), Some(2.0));
    assert_eq!(
        r2.eol().ready("SvcA"),
        Some(1.0),
        "r2's schedule is its own"
    );
    for d in [&r1, &r2] {
        let t = d.eol();
        assert!(t.pos(is_ready, "Flags") < t.pos(is_start, "SvcA"));
        assert!(t.pos(is_ready, "Db") < t.pos(is_start, "SvcA"));
        assert!(t.pos(is_ready, "Flags") < t.pos(is_start, "SvcB"));
        assert!(t.pos(is_ready, "SvcA") < t.pos(is_start, "Aggregate"));
        assert!(t.pos(is_ready, "SvcB") < t.pos(is_start, "Aggregate"));
        assert!(t.pos(is_ready, "Aggregate") < t.pos(is_cleanup_start, "Db"));
        assert!(t.pos(is_ready, "Aggregate") < t.pos(is_cleanup_start, "Flags"));
        assert_eq!(d.report.outcome, Outcome::Ok);
    }
    fn is_send_sync<T: Send + Sync>(_: &T) {}
    is_send_sync(&plan);
}

#[test]
fn c05_a_service_is_ready_on_its_return_and_the_endpoint_outlives_it() {
    let plan = i05();
    let script = Script::new()
        .prepare("Endpoint", ok_after(1.0))
        .prepare("AcceptLoop", ok_after(1.0))
        .prepare("PublishAddr", ok_after(1.0))
        .at(6.0, Request::Shutdown)
        .serve("AcceptLoop", [Serve::StopsAfter(secs(1))])
        .cleanup("Endpoint", Cleanup::Ok(secs(1)))
        .cleanup("PublishAddr", Cleanup::Ok(secs(1)));
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    let t = d.eol();
    assert!(t.pos(is_ready, "AcceptLoop") < t.pos(is_start, "PublishAddr"));
    assert_eq!(t.start("AcceptLoop"), Some(1.0), "spawned at 1");
    assert_eq!(
        t.ready("AcceptLoop"),
        Some(2.0),
        "ready on the body's return, at 2"
    );
    assert!(t.pos(is_cleanup_end, "PublishAddr") < t.pos(is_cleanup_start, "Endpoint"));
    assert!(t.pos(is_cleanup_end, "AcceptLoop") < t.pos(is_cleanup_start, "Endpoint"));
    assert_eq!(t.cleanup_start("AcceptLoop"), Some(7.0));
    assert_eq!(t.cleanup_end("AcceptLoop"), Some(8.0));
    assert_eq!(
        t.cleanup_start("Endpoint"),
        Some(8.0),
        "alive during the stop"
    );
    assert_eq!(d.report.outcome, Outcome::Ok);
}

#[test]
fn c07_finite_and_long_lived_nodes_in_one_plan() {
    let plan = i07(false);
    let script = Script::new()
        .prepare("Migrate", ok_after(2.0))
        .prepare("Exporter", ok_after(1.0))
        .prepare("Api", ok_after(1.0))
        .at(10.0, Request::Shutdown)
        .serve("Exporter", [Serve::StopsAfter(secs(1))])
        .serve("Api", [Serve::StopsAfter(secs(1))]);
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    let t = d.eol();
    assert!(t.pos(is_ready, "Migrate") < t.pos(is_start, "Api"));
    assert!(t.pos(is_ready, "Exporter") < t.pos(is_start, "Api"));
    assert_eq!(t.start("Api"), Some(2.0));
    assert!(
        t.pos(is_stopped, "Exporter").is_none()
            || t.stop_requested("Exporter") < t.cleanup_end("Exporter"),
        "a service does not complete by itself"
    );
    assert!(
        t.stopped_before(10.0, "Exporter").is_none(),
        "never(ok(Exporter.run)) before shutdown"
    );
    assert!(t.pos(is_cleanup_end, "Api") < t.pos(is_cleanup_start, "Exporter"));
    assert!(
        t.cleanup_start("Migrate").is_none(),
        "a step has nothing to release"
    );
    assert_eq!(d.report.outcome, Outcome::Ok);
}

#[test]
fn c27_exclusive_users_are_serialised_in_either_order() {
    let plan = i27(false);
    let base = Script::new()
        .prepare("Db", ok_after(1.0))
        .prepare("MigA", ok_after(3.0))
        .prepare("MigB", ok_after(2.0))
        .at(20.0, Request::Shutdown);
    let ab = ScriptedDriver::run(
        &plan,
        &base.clone().schedule(Schedule::order(["MigA", "MigB"])),
    )
    .expect("runs");
    ab.check();
    let t = ab.eol();
    assert_eq!(t.start("MigA"), Some(1.0));
    assert_eq!(
        t.start("MigB"),
        Some(4.0),
        "MigB waits for MigA's exclusive grant"
    );
    assert!(t.pos(is_ready, "MigA") < t.pos(is_start, "App"));
    assert!(t.pos(is_ready, "MigB") < t.pos(is_start, "App"));
    assert_eq!(
        ab.why_at("MigB", 2.0),
        vec![("MigA".to_string(), Reason::Exclusive)]
    );
    assert_eq!(ab.report.outcome, Outcome::Ok);

    let ba = ScriptedDriver::run(&plan, &base.schedule(Schedule::order(["MigB", "MigA"])))
        .expect("runs");
    ba.check();
    let t = ba.eol();
    assert_eq!(t.start("MigB"), Some(1.0));
    assert_eq!(
        t.start("MigA"),
        Some(3.0),
        "the mirrored schedule is also valid"
    );
    assert_eq!(ba.report.outcome, Outcome::Ok);
}

#[test]
fn c29_a_blocking_step_runs_on_its_pool_with_at_most_two_at_once() {
    let plan = i29(0);
    let script = Script::new()
        .prepare("Snapshot", ok_after(1.0))
        .prepare("Verify", ok_after(2.0))
        .prepare("Serve", ok_after(1.0))
        .at(10.0, Request::Shutdown);
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    assert_eq!(
        plan.inspect().node("Verify").expect("Verify").attr("pool"),
        Some("cpu")
    );
    assert!(d
        .effects()
        .iter()
        .any(|e| e.starts_with("SpawnBlocking") && e.contains(&format!("{:?}", d.key("Verify")))));
    assert!(!d
        .effects()
        .iter()
        .any(|e| e.starts_with("Spawn {") && e.contains(&format!("{:?}", d.key("Verify")))));
    let t = d.eol();
    assert!(t.pos(is_ready, "Verify") < t.pos(is_start, "Serve"));
    assert_eq!(d.report.outcome, Outcome::Ok);

    // Variant: three blocking steps on cpu(2); the third starts only after
    // one of the first two ended.
    let plan = i29(2);
    let script = Script::new()
        .prepare("Snapshot", ok_after(1.0))
        .prepare("Verify", ok_after(2.0))
        .prepare("Verify2", ok_after(3.0))
        .prepare("Verify3", ok_after(1.0))
        .at(10.0, Request::Shutdown);
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    let t = d.eol();
    assert_eq!(t.start("Verify"), Some(1.0));
    assert_eq!(t.start("Verify2"), Some(1.0));
    assert_eq!(t.start("Verify3"), Some(3.0), "after Verify ended at 3");
    assert!(t.max_concurrent(&["Verify", "Verify2", "Verify3"]) <= 2);
}

#[test]
fn c41_ports_only_the_library_sees() {
    let plan = i41();
    let script = every_body(&plan, 1.0).at(5.0, Request::Shutdown);
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    let t = d.eol();
    for port in ["Storage", "Signer", "Transport", "Clock"] {
        assert!(t.pos(is_ready, port) < t.pos(is_start, "Discovery"));
    }
    assert_eq!(d.report.outcome, Outcome::Ok);
}

#[test]
fn c62_shared_readers_overlap_each_other_but_never_a_migrator() {
    let plan = i27(true);
    let script = Script::new()
        .prepare("Db", ok_after(1.0))
        .prepare("MigA", ok_after(3.0))
        .prepare("MigB", ok_after(2.0))
        .prepare("R1", ok_after(2.0))
        .prepare("R2", ok_after(2.0))
        .at(30.0, Request::Shutdown)
        .schedule(Schedule::order(["R1", "R2", "MigA", "MigB"]));
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    let t = d.eol();
    assert_eq!(t.start("R1"), Some(1.0));
    assert_eq!(t.start("R2"), Some(1.0), "shared readers overlap");
    assert!(t.max_concurrent(&["MigA", "MigB"]) <= 1);
    assert_eq!(t.overlap_count(&["MigA", "MigB"], &["R1", "R2"]), 0);
    assert_eq!(d.report.outcome, Outcome::Ok);
}

#[test]
fn c63_a_pool_grants_fifo_and_why_names_the_pool() {
    let plan = three_limited_steps();
    let script = Script::new()
        .prepare("S1", ok_after(2.0))
        .prepare("S2", ok_after(2.0))
        .prepare("S3", ok_after(2.0));
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    let t = d.eol();
    assert_eq!(t.start("S1"), Some(0.0));
    assert_eq!(t.start("S2"), Some(0.0));
    assert_eq!(t.start("S3"), Some(2.0), "when the first ends");
    assert_eq!(d.why_at("S3", 1.0), vec![("cpu".to_string(), Reason::Pool)]);
    assert_eq!(d.report.outcome, Outcome::Ok);
}
