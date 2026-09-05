//! Regressions the Monte Carlo walk found (suite (d) → suite (c)).
//!
//! Every test here is one bug the random walk turned up, written out by hand
//! in the shape that produced it and named for what went wrong. The doc line
//! carries the MC case seed that found it; `tests/monte_carlo.rs` replays the
//! seeds themselves in `every_seed_that_found_a_bug_replays_clean`. These are
//! the durable pins: they do not move when the generator changes.
//!
//! Every trace here is checked by the invariant checker as well.

use crate::corpus::*;
use sdax::*;
use sdax_testkit::eol::{is_abandoned, is_cleanup_end, is_cleanup_start, is_interrupted};
use sdax_testkit::ScriptedDriver;

fn unit_res<D: Deps>(p: &mut PlanBuilder, name: &str, deps: D) -> Key<Unit> {
    p.resource(name)
        .needs(deps)
        .acquire(|cx, _d| async move { Ok(cx.hold_value(Unit)) })
        .release(|_cx, _u| async move { Ok(()) })
}

/// MC case seed 6299039668138085550.
///
/// A `Body` whose `held` is later than its own ending fed the machine a
/// `Held` for a body that had already returned; the machine rejected it. A
/// body registers its value before it returns, so the hold happens at the
/// ending instant, and is delivered first.
#[test]
fn mc_a_hold_later_than_the_body_ending_happens_at_the_ending() {
    let mut p = Plan::builder("Hold");
    unit_res(&mut p, "R", ());
    let plan = p
        .build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Finite)
        .expect("valid");
    let script = Script::new()
        .prepare("R", Body::ok(At::plus(0.0)).held(At::plus(1.0)))
        .cleanup("R", Cleanup::Ok(secs(0)));
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    assert_eq!(
        d.eol().held("R"),
        Some(0.0),
        "the hold is clamped to the ending, not dropped and not rejected"
    );
    assert_eq!(d.eol().ready("R"), Some(0.0));
    assert!(
        d.eol().cleanup_start("R").is_some(),
        "and the resource still owes its release"
    );
    assert_eq!(d.report.outcome, Outcome::Ok);
}

/// MC case seed 15604115191173039358.
///
/// A node abandoned by the shutdown budget while it was retrying lost the
/// faults of the attempts that had already failed, and the run ended `Ok`
/// with a `Fail` plainly in its trace.
#[test]
fn mc_an_abandoned_node_keeps_the_faults_of_its_earlier_attempts() {
    let mut p = Plan::builder("Abandon");
    let pool = p.pool("cpu", 1);
    p.blocking_step("B")
        .retry(Retry::attempts(2))
        .on(pool)
        .run(|_cx, ()| Ok(()));
    let plan = p
        .build(Policy::FailFast, Shutdown::within(secs(2)), Mode::Finite)
        .expect("valid");
    // Attempt 1 fails at 1s; attempt 2 never returns, and a blocking body
    // cannot be aborted (T7), so the budget abandons it at 3s.
    let script = Script::new()
        .body("B", vec![Body::fail(At::plus(1.0), "one"), Body::pending()])
        .at(1.5, Request::Shutdown);
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    assert_eq!(d.eol().fail("B"), Some(1.0));
    assert_eq!(d.eol().abandoned("B"), Some(3.5), "at the budget");
    assert_eq!(
        d.report.faults.len(),
        1,
        "the first attempt's fault survives the abandonment: {}",
        d.report
    );
    assert_eq!(d.report.faults[0].node.to_string(), "B");
    assert_eq!(d.report.faults[0].order.attempt, 1);
    assert_eq!(d.report.incomplete.len(), 1);
    assert_eq!(
        d.report.outcome,
        Outcome::Failed,
        "a run that observed a fault did not end Ok"
    );
}

/// MC case seed 2052455747394511223 (and 13281131733918634243 for the
/// `skip_dependents` half).
///
/// A node whose next attempt was waiting for a grant it never got was marked
/// `Skipped` when the scope settled under it, and its earlier faults went
/// with it.
#[test]
fn mc_a_node_waiting_for_its_next_attempt_keeps_its_faults() {
    let mut p = Plan::builder("Waiting");
    let pool = p.pool("slot", 1);
    // `B` takes the one grant first and fails, freeing it; `H` takes it and
    // never returns, so `B`'s retry waits for a grant that never comes back.
    p.step("B")
        .retry(Retry::attempts(3))
        .limit(pool)
        .run(|_cx, ()| async move { Ok(()) });
    p.step("H").limit(pool).run(|_cx, ()| async move { Ok(()) });
    let plan = p
        .build(Policy::Isolate, Shutdown::within(secs(10)), Mode::Finite)
        .expect("valid");
    let script = Script::new()
        .body("B", vec![Body::fail(At::plus(1.0), "one"), Body::pending()])
        .body("H", vec![Body::pending()])
        .at(2.0, Request::Cancel);
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    assert_eq!(d.eol().fail("B"), Some(1.0));
    assert!(
        d.report.faults.iter().any(|f| f.node.to_string() == "B"),
        "B's first attempt is in the report: {}",
        d.report
    );
}

/// MC case seed 16408307695250133523.
///
/// A service's serve fault was emitted to the trace and then dropped when a
/// restart followed. If the restart never reaches `Ready`, nothing ever
/// records it.
#[test]
fn mc_a_serve_fault_survives_a_restart_that_never_comes_up() {
    let mut p = Plan::builder("Restart");
    p.service("S")
        .idempotent()
        .restart(Restart::on_error(Backoff::fixed(secs(0))))
        .stop_within(secs(1))
        .start(|_cx, ()| async move { Ok(Serving::new(Unit, async { Ok(()) })) });
    let plan = p
        .build(Policy::Isolate, Shutdown::within(secs(10)), Mode::Resident)
        .expect("valid");
    let script = Script::new()
        .body("S", vec![Body::ok(At::plus(0.0)), Body::pending()])
        .serve("S", vec![Serve::Err(At::plus(1.0), "serve".into())])
        .at(2.0, Request::Cancel);
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    assert_eq!(d.eol().fail("S"), Some(1.0), "the serve episode failed");
    assert!(
        d.report
            .faults
            .iter()
            .any(|f| f.node.to_string() == "S" && f.phase == Phase::Serve),
        "and the report says so: {}",
        d.report
    );
}

/// A one-node child plan whose node is `name`, importing nothing.
fn child(name: &str, mode: Mode) -> Plan<Unit> {
    let mut c = Plan::builder(name);
    let k = unit_res(&mut c, "Inner", ());
    c.export(k)
        .build(Policy::Isolate, Shutdown::within(secs(4)), mode)
        .expect("valid child")
}

/// MC case seed 3133216432680936309.
///
/// A component cancelled before its inner graph came up went straight from
/// `Start(Prepare)` to `ReleaseStart`: its own attempt never ended, so the
/// trace claimed a release for something that had not finished (INV-15).
#[test]
fn mc_an_interrupted_component_ends_its_own_attempt() {
    let inner = child("Child", Mode::Finite);
    let mut p = Plan::builder("Outer");
    p.component("C", &inner);
    let plan = p
        .build(Policy::FailFast, Shutdown::within(secs(9)), Mode::Resident)
        .expect("valid");
    let script = Script::new()
        .prepare("C/Inner", Body::pending())
        .at(1.0, Request::Cancel);
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    let t = d.eol();
    assert_eq!(
        t.interrupted("C"),
        Some(true),
        "the component's own attempt ends, and it has an inner graph to tear down"
    );
    assert!(
        t.pos(is_interrupted, "C") < t.pos(is_cleanup_start, "C"),
        "and it ends before its release starts"
    );
    assert_eq!(d.report.outcome, Outcome::Cancelled);
}

/// MC case seed 666241015029950078.
///
/// A component that was `Ready` kept its inner scope `Steady` when the run
/// settled, so `in_flight` counted the component while the component waited
/// for the run to reach cleanup: the run never ended at all.
#[test]
fn mc_a_ready_components_inner_scope_settles_with_its_parent() {
    let inner = child("Child", Mode::Resident);
    let mut p = Plan::builder("Outer");
    p.component("C", &inner);
    let plan = p
        .build(Policy::FailFast, Shutdown::unbounded(), Mode::Finite)
        .expect("valid");
    let script = Script::new()
        .prepare("C/Inner", Body::ok(At::plus(1.0)))
        .cleanup("C/Inner", Cleanup::Ok(secs(1)));
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    assert!(
        !d.stuck,
        "the run reaches End: {}",
        d.problems().unwrap_or_default()
    );
    d.check();
    assert_eq!(d.eol().ready("C"), Some(1.0));
    assert_eq!(d.report.outcome, Outcome::Ok);
}

/// MC case seed 14688502050082075365.
///
/// A component that never started left its inner scope `Planned` with every
/// inner node `Pending`; those nodes held the release gate of the parent key
/// they import shut, and the run could not finish.
#[test]
fn mc_a_skipped_component_does_not_hold_the_release_gate_shut() {
    let mut c = Plan::builder("Child");
    let mut outer = Plan::builder("Outer");
    let r = unit_res(&mut outer, "R", ());
    let imported = c.import(r);
    let k = unit_res(&mut c, "Inner", imported);
    let inner = c
        .export(k)
        .build(Policy::Isolate, Shutdown::within(secs(4)), Mode::Finite)
        .expect("valid child");
    let gate = outer.step("Gate").run(|_cx, ()| async move { Ok(()) });
    outer.component("C", &inner).raw();
    outer
        .step("After")
        .needs(gate)
        .run(|_cx, _g: std::sync::Arc<()>| async move { Ok(()) });
    let plan = outer
        .build(Policy::FailFast, Shutdown::within(secs(9)), Mode::Finite)
        .expect("valid");
    // `Gate` never returns, so the component never starts; the cancel then
    // has to get the run to `End` with `R` released.
    let script = Script::new()
        .prepare("R", Body::ok(At::plus(0.0)))
        .cleanup("R", Cleanup::Ok(secs(0)))
        .body("Gate", vec![Body::pending()])
        .at(1.0, Request::Cancel);
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    assert!(
        !d.stuck,
        "the run reaches End: {}",
        d.problems().unwrap_or_default()
    );
    d.check();
    assert!(
        d.eol().cleanup_end("R").is_some(),
        "R's release ran even though the component that imports it never started"
    );
    assert_eq!(d.report.outcome, Outcome::Cancelled);
}

/// MC case seed 15845907256369010671.
///
/// `RecordOrder.steps` was the builder's own `key.idx`, which counts the
/// `import` nodes `PlanView` does not show. F4 order and view order then
/// disagreed for every component that imports anything.
#[test]
fn mc_record_order_counts_only_the_nodes_the_view_shows() {
    let mut outer = Plan::builder("Outer");
    let r = unit_res(&mut outer, "R", ());
    let mut c = Plan::builder("Child");
    let imported = c.import(r);
    let k = unit_res(&mut c, "Inner", imported);
    let inner = c
        .export(k)
        .build(Policy::Isolate, Shutdown::within(secs(4)), Mode::Finite)
        .expect("valid child");
    outer.component("C", &inner).raw();
    let plan = outer
        .build(Policy::Isolate, Shutdown::within(secs(9)), Mode::Finite)
        .expect("valid");
    let view = plan.inspect();
    assert_eq!(
        view.nodes[2].path.to_string(),
        "C/Inner",
        "the view does not show the import"
    );
    let script = Script::new()
        .prepare("R", Body::ok(At::plus(0.0)))
        .cleanup("R", Cleanup::Ok(secs(0)))
        .prepare("C/Inner", Body::fail(At::plus(1.0), "boom"));
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    let f = d
        .report
        .faults
        .iter()
        .find(|f| f.node.to_string() == "C/Inner")
        .expect("the inner fault is reported");
    let steps: Vec<u32> = f.order.steps.iter().map(|(i, _)| *i).collect();
    assert_eq!(
        steps,
        vec![1, 0],
        "the component is view position 1 and its first shown node is 0"
    );
}

/// MC case seed 4529049039084370172.
///
/// A component that faulted left its inner scope `Admitting` under an inner
/// `Isolate`, so a sibling that became ready afterwards started a body after
/// the run had settled (T5).
#[test]
fn mc_a_failed_component_stops_admitting_inside() {
    let mut c = Plan::builder("Child");
    let x = c.step("X").run(|_cx, ()| async move { Ok(()) });
    let z = c.step("Z").run(|_cx, ()| async move { Ok(()) });
    let y = c
        .step("Y")
        .needs(z)
        .run(|_cx, _z: std::sync::Arc<()>| async move { Ok(()) });
    let _ = (x, y);
    let inner = c
        .build(Policy::Isolate, Shutdown::within(secs(4)), Mode::Finite)
        .expect("valid child");
    let mut outer = Plan::builder("Outer");
    outer.component("C", &inner);
    let plan = outer
        .build(Policy::FailFast, Shutdown::within(secs(9)), Mode::Resident)
        .expect("valid");
    let script = Script::new()
        .body("C/X", vec![Body::fail(At::plus(0.0), "boom")])
        .body("C/Z", vec![Body::ok(At::plus(1.0))])
        .body("C/Y", vec![Body::ok(At::plus(1.0))]);
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    assert_eq!(d.eol().fail("C/X"), Some(0.0));
    assert!(
        d.eol().start("C/Y").is_none(),
        "nothing starts inside a component that has already failed"
    );
    assert_eq!(d.report.outcome, Outcome::Failed);
}

/// MC case seed 6364352641463191117.
///
/// At budget expiry `abandon_all` walked one declaration-order pass, so a
/// component's inner `ReleaseStart` could be emitted before the `Abandoned`
/// of a node that depends on the component (INV-5).
#[test]
fn mc_the_budget_ends_the_dependents_before_it_opens_an_inner_release() {
    let mut c = Plan::builder("Child");
    let k = unit_res(&mut c, "Inner", ());
    let inner = c
        .export(k)
        .build(Policy::Isolate, Shutdown::within(secs(2)), Mode::Resident)
        .expect("valid child");
    let mut outer = Plan::builder("Outer");
    let comp = outer.component("C", &inner);
    let pool = outer.pool("cpu", 1);
    outer
        .blocking_step("B")
        .needs(comp)
        .on(pool)
        .run(|_cx, _c: std::sync::Arc<Unit>| Ok(()));
    let plan = outer
        .build(Policy::FailFast, Shutdown::within(secs(2)), Mode::Resident)
        .expect("valid");
    let script = Script::new()
        .prepare("C/Inner", Body::ok(At::plus(1.0)))
        .cleanup("C/Inner", Cleanup::Ok(secs(0)))
        .body("B", vec![Body::pending()])
        .at(2.0, Request::Shutdown);
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    let t = d.eol();
    assert_eq!(t.abandoned("B"), Some(4.0), "the budget abandons B");
    assert!(
        t.pos(is_abandoned, "B") < t.pos(is_cleanup_start, "C"),
        "and only then does the component's inner release open"
    );
    assert!(t.pos(is_cleanup_start, "C") < t.pos(is_cleanup_end, "C"));
}

/// MC case seed 2796147909731581100.
///
/// At the shutdown budget `abandon_inner` emitted the component's own
/// `ReleaseStart` unconditionally. A *dependent* component abandoned in the
/// same instant only reaches `ReleaseOk` in the sweep that follows, so the
/// held component's inner release graph opened first (INV-5).
#[test]
fn mc_the_budget_does_not_open_an_inner_release_before_a_dependent_component_ends() {
    let mut outer = Plan::builder("Outer");

    let mut held = Plan::builder("Held");
    let hk = unit_res(&mut held, "HR", ());
    let held = held
        .export(hk)
        .build(Policy::Isolate, Shutdown::within(secs(2)), Mode::Resident)
        .expect("valid held child");
    let c2 = outer.component("C2", &held);

    let mut user = Plan::builder("User");
    let imported = user.import(c2);
    unit_res(&mut user, "UR", imported);
    let user = user
        .build(Policy::Isolate, Shutdown::within(secs(2)), Mode::Resident)
        .expect("valid user child");
    outer.component("C4", &user).raw();

    let plan = outer
        .build(Policy::FailFast, Shutdown::within(secs(2)), Mode::Resident)
        .expect("valid");
    // `C4` needs `C2`, so at the shutdown only `C4`'s inner release may open;
    // `UR`'s release never completes, and the budget abandons it at 3s. `C2`
    // is still `Ready` at that instant, and its inner graph must wait for
    // `C4`'s `ReleaseOk`, which the same sweep emits.
    let script = Script::new()
        .prepare("C2/HR", Body::ok(At::plus(0.0)))
        .cleanup("C2/HR", Cleanup::Ok(secs(0)))
        .prepare("C4/UR", Body::ok(At::plus(0.0)))
        .cleanup("C4/UR", Cleanup::IgnoreStop)
        .at(1.0, Request::Shutdown);
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    let t = d.eol();
    assert_eq!(t.abandoned("C4/UR"), Some(3.0), "the budget abandons UR");
    assert!(
        t.pos(is_cleanup_end, "C4") < t.pos(is_cleanup_start, "C2"),
        "C4 finishes before the component it needs opens its release:\n{}",
        t.render()
    );
    assert!(t.pos(is_cleanup_start, "C2") < t.pos(is_cleanup_start, "C2/HR"));
    assert_eq!(d.report.outcome, Outcome::Ok);
}

/// MC case seed 2682709018262434330.
///
/// A `terminal` service inside a component settles the *inner* scope on its
/// own, without the parent settling. The component's own attempt was then
/// never ended: its trace went `Start(Prepare)` → `ReleaseStart`, an orphan
/// (INV-15). `settle::interrupt` already ends the attempt when the parent
/// settles; nothing did when the inner scope settled by itself.
#[test]
fn mc_a_component_whose_inner_scope_settles_by_itself_ends_its_own_attempt() {
    let mut c = Plan::builder("Child");
    let s = c
        .service("S")
        .terminal()
        .start(|_cx, ()| async move { Ok(Serving::new((), async { Ok(()) })) });
    // `R` never returns, so the inner run never reaches steady state and `C`
    // never becomes `Ready`; the terminal service ending settles the inner
    // scope under it.
    c.resource("R")
        .needs(s)
        .acquire(|cx, _s: std::sync::Arc<()>| async move { Ok(cx.hold_value(Unit)) })
        .release(|_cx, _u| async move { Ok(()) });
    let inner = c
        .build(Policy::Isolate, Shutdown::within(secs(4)), Mode::Resident)
        .expect("valid child");
    let mut outer = Plan::builder("Outer");
    outer.component("C", &inner).raw();
    let plan = outer
        .build(Policy::Isolate, Shutdown::within(secs(9)), Mode::Resident)
        .expect("valid");
    let script = Script::new()
        .prepare("C/S", Body::ok(At::plus(0.0)))
        .serve("C/S", [Serve::Ok(At::tick(1.0))])
        .body("C/R", vec![Body::pending()])
        .at(3.0, Request::Shutdown);
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    let t = d.eol();
    assert_eq!(
        t.interrupted("C"),
        Some(true),
        "the component's attempt ends before its release opens:\n{}",
        t.render()
    );
    assert!(t.pos(is_interrupted, "C") < t.pos(is_cleanup_start, "C"));
    assert_eq!(d.report.outcome, Outcome::Ok);
}

/// MC case seed 2300709931059428012.
///
/// A component whose inner graph comes up inside `start` re-enters `admit`
/// for the same scope, from the `Ready` that follows. The outer pass then
/// carried on down a waiter list taken before that, and started a node the
/// nested pass had already started — two attempts of one node in flight at
/// once (INV-12), the first an orphan (INV-15).
#[test]
fn mc_a_waiter_a_nested_admit_already_started_is_not_started_twice() {
    let mut outer = Plan::builder("Outer");
    let e = unit_res(&mut outer, "E", ());
    let mut c = Plan::builder("Child");
    // The import makes `C` need `E`, so `C` and `R` queue together; a join
    // with no needs is Ready as soon as the inner scope admits, so the
    // component becomes Ready inside its own `start`.
    c.import(e);
    c.join("J", ());
    let inner = c
        .build(Policy::Isolate, Shutdown::within(secs(4)), Mode::Finite)
        .expect("valid child");
    // `C` and `R` become need-ready together and queue in declaration order;
    // `C` is granted first, and `R` must be started exactly once.
    outer.component("C", &inner).raw();
    let r = unit_res(&mut outer, "R", e);
    let _ = r;
    let plan = outer
        .build(Policy::Isolate, Shutdown::within(secs(9)), Mode::Finite)
        .expect("valid");
    let script = Script::new()
        .prepare("E", Body::ok(At::plus(1.0)))
        .cleanup("E", Cleanup::Ok(secs(0)))
        .prepare("R", Body::ok(At::tick(2.0)))
        .cleanup("R", Cleanup::Ok(secs(0)));
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    let t = d.eol();
    assert_eq!(t.attempts("R"), 1, "R starts once:\n{}", t.render());
    assert_eq!(t.ready("R"), Some(2.0));
    assert_eq!(d.report.outcome, Outcome::Ok);
}

/// MC case seed 11779147375297488456.
///
/// A blocking body cannot be aborted (T7), so `on_within_timer` fails the
/// attempt and says the thread's later outcome is ignored. The simulator
/// kept delivering it: the stale outcome was credited to the *next* attempt,
/// which then reached `Ready` before its own `Started` — refused as
/// `Started for a node with no body in flight`. A run driver holds one task
/// handle per node and drops a superseded attempt's result; the simulator
/// stands in for one, so it must too.
#[test]
fn mc_a_timed_out_blocking_attempt_does_not_end_the_next_one() {
    let mut p = Plan::builder("Stale");
    let pool = p.pool("cpu", 1);
    p.blocking_step("B")
        .within(secs(2))
        .retry(Retry::attempts(3))
        .on(pool)
        .run(|_cx, ()| Ok(()));
    let plan = p
        .build(Policy::Isolate, Shutdown::within(secs(20)), Mode::Finite)
        .expect("valid");
    // Attempt 1's thread finishes at t=4, two seconds after the deadline that
    // already failed it; attempt 2 starts at t=2 and returns `Ok` at t=4 too.
    let script = Script::new().body(
        "B",
        vec![Body::fail(At::tick(4.0), "late"), Body::ok(At::plus(2.0))],
    );
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    let t = d.eol();
    assert_eq!(t.attempts("B"), 2, "two attempts, no more:\n{}", t.render());
    assert_eq!(t.start_attempt("B", 2), Some(2.0));
    assert_eq!(
        t.ready("B"),
        Some(4.0),
        "attempt 2 ends on its own body, not attempt 1's"
    );
    assert_eq!(d.report.outcome, Outcome::Ok);
}
