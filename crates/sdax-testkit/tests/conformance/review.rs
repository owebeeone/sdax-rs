//! Remediation of `dev-docs/Review-Stage1-Semantics.md`.
//!
//! One test per finding the review predicted, written in the shape § 6 gives
//! (`R-1` … `R-8`) and named for the finding it pins. Each was watched failing
//! with the predicted symptom before the machine was changed;
//! `dev-docs/Stage1-Review-Remediation-Log.md` carries the RED evidence.
//!
//! Like every module here, this one is compiled twice — once against the
//! scripted driver and once against the tokio adapter (LBT-009).

use crate::corpus::*;
use crate::Drv as ScriptedDriver;
use sdax::*;
use sdax_testkit::eol::is_abandoned;
use std::sync::Arc;

/// R-1 / F-01 — a waiter for a free pool is not queued behind a waiter for a
/// full one. One `blocked_pool` flag for the whole scope made every pool one
/// queue: INV-1 allows arbitration only "among nodes that declare the same
/// lock or pool".
///
/// R-1 / F-06 — and the delay is not silent: a `Waiting` node names a reason.
#[test]
fn review_r1_a_free_pool_is_not_queued_behind_a_full_one() {
    let mut p = Plan::builder("HOL");
    let cpu = p.pool("cpu", 1);
    let io = p.pool("io", 1);
    p.step("A1").limit(cpu).run(|_cx, ()| async move { Ok(()) });
    p.step("A2").limit(cpu).run(|_cx, ()| async move { Ok(()) });
    p.step("B").limit(io).run(|_cx, ()| async move { Ok(()) });
    let plan = p
        .build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Finite)
        .expect("valid");
    let script = Script::new()
        .prepare("A1", Body::ok(At::plus(10.0)))
        .prepare("A2", Body::ok(At::plus(0.0)))
        .prepare("B", Body::ok(At::plus(0.0)));
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    let t = d.eol();
    assert_eq!(
        t.start("B"),
        Some(0.0),
        "INV-1: io is free and B has no needs\n{}",
        t.render()
    );
    assert_eq!(t.start("A2"), Some(10.0), "cpu is full until A1 is done");
}

/// R-1's companion / F-06 — an honest FIFO wait, with no F-01 flag involved:
/// `A` is queued first and blocked on a lock a resident service holds; `B`
/// wants only the pool, which has room. T1's FIFO refuses `B`, and before this
/// fix `why(B)` was empty — the contract § 2 says `on` lists each reason.
#[test]
fn review_r1b_a_queued_waiter_names_the_waiter_ahead_of_it() {
    let mut p = Plan::builder("Queue");
    let cpu = p.pool("cpu", 1);
    let db = p
        .resource("Db")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Unit)) })
        .release(|_cx, _u| async move { Ok(()) });
    // A resident service holds `db` exclusively for the whole run, so `A`
    // never gets its lock and `B` is refused behind it.
    p.service("Holder")
        .needs(db)
        .exclusive(db)
        .stop_within(secs(1))
        .initialize(|_cx, _d: Arc<Unit>| async move { Ok(()) })
        .serve(|_cx, _handle| async move { Ok(()) });
    p.step("A")
        .needs(db)
        .exclusive(db)
        .limit(cpu)
        .run(|_cx, _d: Arc<Unit>| async move { Ok(()) });
    // `B` becomes need-ready with `A` and is declared after it, so T1 queues
    // it behind `A`; `cpu` itself is free.
    p.step("B")
        .needs(db)
        .limit(cpu)
        .run(|_cx, _d: Arc<Unit>| async move { Ok(()) });
    let plan = p
        .build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Resident)
        .expect("valid");
    let script = Script::new()
        .prepare("Db", Body::ok(At::plus(0.0)))
        .prepare("Holder", Body::ok(At::plus(0.0)))
        .prepare("A", Body::ok(At::plus(0.0)))
        .prepare("B", Body::ok(At::plus(0.0)))
        .cleanup("Db", Cleanup::Ok(secs(0)))
        .at(5.0, Request::Shutdown);
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    let why = d.why_at("B", 1.0);
    assert!(
        why.iter()
            .any(|(p, r)| *r == Reason::QueuedBehind && p == "A"),
        "F-06: a FIFO-blocked waiter names the waiter ahead of it, got {why:?}"
    );
}

/// F-24 — a backing-off node's grants are free the instant its attempt failed.
///
/// Found by the new `WHY` checker rule at case 2127 of the default walk
/// (`SDAX_MC_SEED=6746427589533237249 SDAX_MC_CASES=2128`): `B1 is Waiting at
/// t=0ns and names no reason`. A non-zero backoff set the slot and armed the
/// timer and re-admitted nothing, so the pool slot the failed attempt had just
/// released sat idle until an unrelated timer fired — an ordering INV-1 does
/// not allow, and silent, exactly like F-01.
#[test]
fn review_f24_a_backoff_does_not_hold_the_grants_its_attempt_released() {
    let mut p = Plan::builder("Backoff");
    let cpu = p.pool("cpu", 1);
    p.step("A")
        .retry(Retry::attempts(2).backoff(Backoff::fixed(secs(3))))
        .limit(cpu)
        .run(|_cx, ()| async move { Ok(()) });
    p.step("B").limit(cpu).run(|_cx, ()| async move { Ok(()) });
    let plan = p
        .build(Policy::Isolate, Shutdown::within(secs(20)), Mode::Finite)
        .expect("valid");
    let script = Script::new()
        .body(
            "A",
            vec![Body::fail(At::plus(0.0), "one"), Body::ok(At::plus(0.0))],
        )
        .prepare("B", Body::ok(At::plus(0.0)));
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    let t = d.eol();
    assert_eq!(
        t.start("B"),
        Some(0.0),
        "cpu is free the instant A's attempt failed\n{}",
        t.render()
    );
}

/// F-25 — a lock a parent node drops re-admits the child scope waiting for it.
///
/// Found by the new `WHY` rule at case 6200 of the 50 000-case walk
/// (`SDAX_MC_SEED=20260906`): `C5/C5/E0 is Waiting at t=5s and names no
/// reason`. `table.rs` resolves an import to the parent's node, so a child
/// locking an imported resource contends with the parent for one lock — but
/// `after_settle` re-admitted the *releasing node's own* scope only, and the
/// child waited for a lock nobody held, for ever and without a reason.
#[test]
fn review_f25_a_released_lock_re_admits_every_scope_waiting_for_it() {
    let plan = imported_lock_plan();
    let script = Script::new()
        .prepare("R0", Body::ok(At::plus(0.0)))
        .prepare("S", Body::ok(At::plus(5.0)))
        .prepare("C/E", Body::ok(At::plus(0.0)))
        .cleanup("R0", Cleanup::Ok(secs(0)));
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    let t = d.eol();
    assert_eq!(
        t.start("C/E"),
        Some(5.0),
        "the child takes the lock the instant the parent drops it\n{}",
        t.render()
    );
}

/// R-2 / F-02 — a nested scope's shutdown budget starts when its release graph
/// may open, not when the inner scope settles. The child settles at t=3 with a
/// 2 s budget; its release cannot open until the parent's cleanup reaches the
/// component at t=20, and 1 s of release fits inside the parent's 30 s.
#[test]
fn review_r2_a_nested_budget_does_not_start_before_the_inner_release_can() {
    let plan = nested_budget_plan();
    let script = Script::new()
        .prepare("C/Conn", Body::ok(At::plus(0.0)))
        .prepare("C/Svc", Body::ok(At::plus(0.0)))
        .serve("C/Svc", [Serve::Err(At::plus(3.0), "dies".into())])
        .prepare("Sess", Body::ok(At::plus(0.0)))
        .prepare("Api", Body::ok(At::plus(0.0)))
        .cleanup("C/Conn", Cleanup::Ok(secs(1)))
        .cleanup("Sess", Cleanup::Ok(secs(0)))
        .at(20.0, Request::Shutdown);
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    let t = d.eol();
    assert_eq!(t.cleanup_start("C/Conn"), Some(20.0), "{}", t.render());
    assert_eq!(
        t.cleanup_end("C/Conn"),
        Some(21.0),
        "1 s of release inside 30 s of parent budget\n{}",
        t.render()
    );
    assert!(
        d.report.incomplete.is_empty(),
        "nothing abandoned: {:?}\n{}",
        d.report.incomplete,
        t.render()
    );
}

/// R-2's `FailFast`-parent variant: parent budget 10 s, child 2 s, a parent
/// resource whose own release takes 3 s. The child's clock must not have run
/// out before the parent's graph reached it.
#[test]
fn review_r2b_a_nested_budget_survives_a_slow_parent_release() {
    let plan = nested_budget_failfast_plan();
    let script = Script::new()
        .prepare("C/Conn", Body::ok(At::plus(0.0)))
        .prepare("C/Svc", Body::ok(At::plus(0.0)))
        .serve("C/Svc", [Serve::Err(At::plus(3.0), "dies".into())])
        .prepare("Sess", Body::ok(At::plus(0.0)))
        .cleanup("C/Conn", Cleanup::Ok(secs(1)))
        .cleanup("Sess", Cleanup::Ok(secs(3)));
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    let t = d.eol();
    assert_eq!(t.abandoned("C/Conn"), None, "{}", t.render());
    assert_eq!(t.cleanup_end("C/Conn"), Some(7.0), "{}", t.render());
}

/// R-3 / F-03 — a child plan's declared `Isolate` governs its own scope: an
/// inner fault that the export does not depend on skips the failed node's
/// dependents and leaves the rest of the inner run alone. The component still
/// becomes `Ready` (its export is fine) and the parent's dependent runs.
#[test]
fn review_r3_an_isolate_child_survives_a_fault_off_the_export_path() {
    let plan = isolate_child_plan();
    let script = Script::new()
        .prepare("C/A", Body::ok(At::plus(2.0)))
        .prepare("C/B", Body::fail(At::plus(0.0), "b"))
        .prepare("C/D", Body::ok(At::plus(0.0)))
        .prepare("U", Body::ok(At::plus(0.0)))
        .at(6.0, Request::Shutdown);
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    let t = d.eol();
    assert_eq!(t.ready("C/A"), Some(2.0), "A finishes\n{}", t.render());
    assert_eq!(
        t.skipped_because("C/D"),
        Some("C/B".into()),
        "{}",
        t.render()
    );
    assert!(t.start("U").is_some(), "U runs: C is Ready\n{}", t.render());
    assert_eq!(t.interrupted("C/A"), None, "A is not thrown away");
    let faulted: Vec<String> = d.report.faults.iter().map(|f| f.node.to_string()).collect();
    assert_eq!(faulted, vec!["C/B".to_string()], "one fault, the inner one");
}

/// F-03's other half: an inner fault the export *does* depend on still fails
/// the component, whatever the child's policy — the export can never arrive.
#[test]
fn review_r3b_an_isolate_child_still_fails_a_component_on_the_export_path() {
    let plan = isolate_child_export_plan();
    let script = Script::new()
        .prepare("C/A", Body::fail(At::plus(0.0), "a"))
        .prepare("U", Body::ok(At::plus(0.0)))
        .at(6.0, Request::Shutdown);
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    let t = d.eol();
    assert!(t.fail("C").is_some(), "the component fails\n{}", t.render());
    assert_eq!(t.skipped_because("U"), Some("C/A".into()), "{}", t.render());
}

/// F-04 — a blocking step is signalled when its scope settles, so
/// `cx.is_stopping()` is live in a blocking body. It is still never aborted
/// (T7): the budget abandons it. The grace of `.cooperative(g)` could never be
/// spent here, which `V-BLOCKING-CANCEL` now refuses at build.
#[test]
fn review_r_f04_a_blocking_step_is_signalled_at_settle() {
    let mut p = Plan::builder("Blocking cancel");
    let cpu = p.pool("cpu", 1);
    p.blocking_step("Verify").on(cpu).run(|_cx, ()| Ok(()));
    let plan = p
        .build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Finite)
        .expect("valid");
    let script = Script::new()
        .body("Verify", vec![Body::pending()])
        .at(2.0, Request::Cancel);
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    assert!(
        d.effects().iter().any(|e| e.contains("Signal")),
        "T5: a blocking body is told to stop, got {:?}",
        d.effects()
    );
    assert!(
        !d.effects().iter().any(|e| e.contains("Abort")),
        "T7: and never aborted, got {:?}",
        d.effects()
    );
    assert_eq!(
        d.eol().at(is_abandoned, "Verify"),
        Some(12.0),
        "the budget still abandons it"
    );
}

/// R-7 / F-08 — an `Ambiguity::Retry` whose retry succeeds is a clean run: the
/// ambiguity record is parked on the slot and cleared with the faults when the
/// node reaches `Ready`.
#[test]
fn review_r7_a_resolved_ambiguity_leaves_a_clean_run() {
    let plan = ambiguity_retry_plan();
    let script = Script::new()
        .body(
            "Registration",
            vec![Body::pending(), Body::ok(At::plus(0.0))],
        )
        .cleanup("Registration", Cleanup::Ok(secs(0)));
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    assert!(
        d.report.ambiguous.is_empty(),
        "the retry resolved it: {:?}",
        d.report.ambiguous
    );
    assert_eq!(d.report.outcome, Outcome::Ok);
    assert!(d.report.is_clean(), "{}", d.report);
}

// --------------------------------------------------------------- the plans

fn unit_res(p: &mut PlanBuilder, name: &str) -> Key<Unit> {
    p.resource(name)
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Unit)) })
        .release(|_cx, _u| async move { Ok(()) })
}

fn unit_res_needs<D: Deps>(p: &mut PlanBuilder, name: &str, deps: D) -> Key<Unit> {
    p.resource(name)
        .needs(deps)
        .acquire(|cx, _d| async move { Ok(cx.hold_value(Unit)) })
        .release(|_cx, _u| async move { Ok(()) })
}

/// R-2's child: a resource whose release takes 1 s, and a service that dies at
/// t=3 under an inner `FailFast` with a 2 s budget.
fn nested_budget_child() -> Plan<Unit> {
    let mut inner = Plan::builder("Child");
    let conn = unit_res(&mut inner, "Conn");
    inner
        .service("Svc")
        .needs(conn)
        .stop_within(secs(1))
        .initialize(|_cx, _c: Arc<Unit>| async move { Ok(()) })
        .serve(|_cx, _handle| async move { Ok(()) });
    inner
        .export(conn)
        .build(Policy::FailFast, Shutdown::within(secs(2)), Mode::Resident)
        .expect("valid")
}

/// R-2: an `Isolate` parent with 30 s of budget, a resource that depends on the
/// component and a service that depends on that.
fn nested_budget_plan() -> Plan {
    let child = nested_budget_child();
    let mut p = Plan::builder("P");
    let c = p.component("C", &child, ());
    let sess = unit_res_needs(&mut p, "Sess", c);
    p.service("Api")
        .needs(sess)
        .stop_within(secs(1))
        .initialize(|_cx, _s: Arc<Unit>| async move { Ok(()) })
        .serve(|_cx, _handle| async move { Ok(()) });
    p.build(Policy::Isolate, Shutdown::within(secs(30)), Mode::Resident)
        .expect("valid")
}

/// R-2b: the same child under a `FailFast` parent with 10 s of budget and a
/// dependent resource whose own release takes 3 s.
fn nested_budget_failfast_plan() -> Plan {
    let child = nested_budget_child();
    let mut p = Plan::builder("P");
    let c = p.component("C", &child, ());
    unit_res_needs(&mut p, "Sess", c);
    p.build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Resident)
        .expect("valid")
}

/// R-3's child: `Isolate`, `B` fails, `D` needs `B`, the export `A` is fine.
fn isolate_child() -> Plan<()> {
    let mut inner = Plan::builder("Child");
    let a = inner.step("A").run(|_cx, ()| async move { Ok(()) });
    let b = inner.step("B").run(|_cx, ()| async move { Ok(()) });
    inner
        .step("D")
        .needs(b)
        .run(|_cx, _b: Arc<()>| async move { Ok(()) });
    inner
        .export(a)
        .build(Policy::Isolate, Shutdown::within(secs(4)), Mode::Finite)
        .expect("valid")
}

fn isolate_child_plan() -> Plan {
    let child = isolate_child();
    let mut p = Plan::builder("P");
    let c = p.component("C", &child, ());
    p.step("U")
        .needs(c)
        .run(|_cx, _c: Arc<()>| async move { Ok(()) });
    p.build(Policy::Isolate, Shutdown::within(secs(10)), Mode::Resident)
        .expect("valid")
}

/// R-3b: the same shape with the fault *on* the export path.
fn isolate_child_export_plan() -> Plan {
    let mut inner = Plan::builder("Child");
    let a = inner.step("A").run(|_cx, ()| async move { Ok(()) });
    let child = inner
        .export(a)
        .build(Policy::Isolate, Shutdown::within(secs(4)), Mode::Finite)
        .expect("valid");
    let mut p = Plan::builder("P");
    let c = p.component("C", &child, ());
    p.step("U")
        .needs(c)
        .run(|_cx, _c: Arc<()>| async move { Ok(()) });
    p.build(Policy::Isolate, Shutdown::within(secs(10)), Mode::Resident)
        .expect("valid")
}

/// F-25: a parent step holds `R0` exclusively while a child node that imported
/// `R0` wants it shared.
fn imported_lock_plan() -> Plan {
    let mut p = Plan::builder("Cross");
    let r0 = unit_res(&mut p, "R0");
    p.step("S")
        .needs(r0)
        .exclusive(r0)
        .run(|_cx, _r: Arc<Unit>| async move { Ok(()) });
    let child = {
        let mut c = Plan::builder("Child");
        let r = c.import(r0);
        let e = c
            .step("E")
            .needs(r)
            .shared(r)
            .run(|_cx, _r: Arc<Unit>| async move { Ok(()) });
        c.export(e)
            .build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Finite)
            .expect("valid child")
    };
    p.component("C", &child, ());
    p.build(Policy::FailFast, Shutdown::within(secs(20)), Mode::Finite)
        .expect("valid")
}

/// R-7: an idempotent effect with `within` and `on_ambiguous(Retry)`.
fn ambiguity_retry_plan() -> Plan {
    let mut p = Plan::builder("Ambiguity");
    p.effect("Registration")
        .within(secs(2))
        .idempotent()
        .on_ambiguous(Ambiguity::Retry)
        .perform(|cx, ()| async move { Ok(cx.hold_value(Receipt)) })
        .compensate(|_cx, _r| async move { Ok(()) });
    p.build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Finite)
        .expect("valid")
}
