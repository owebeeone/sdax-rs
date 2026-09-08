//! Suite (b) machine step tests: P-16 (T1 grants are atomic and `why` names
//! the missing one) and P-17 (T4/T5 under `FailFast`: abort, join, then the
//! release graph). These drive `Machine::step` by hand, one event at a time,
//! with no script and no clock but the machine's own.

use crate::host::engine::{Effect, Event, Machine, NodeState, RunState};
use crate::host::RawKey;
use crate::*;
use std::sync::Arc;
use std::time::Duration;

struct Db;
struct A;
struct B;

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn p16_plan(contender: bool) -> Plan {
    let mut p = Plan::builder("P16");
    let cpu = p.pool("cpu", 1);
    let db = p
        .resource("Db")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Db)) })
        .release(|_cx, _d| async move { Ok(()) });
    let a = p
        .resource("A")
        .needs(db)
        .acquire(|cx, _d: Arc<Db>| async move { Ok(cx.hold_value(A)) })
        .release(|_cx, _a| async move { Ok(()) });
    let b = p
        .resource("B")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(B)) })
        .release(|_cx, _b| async move { Ok(()) });
    p.step("Hog")
        .limit(cpu)
        .run(|_cx, ()| async move { Ok(()) });
    if contender {
        p.step("M")
            .needs(db)
            .exclusive(db)
            .run(|_cx, _d: Arc<Db>| async move { Ok(()) });
    }
    p.step("N")
        .needs((db, a, b))
        .exclusive(db)
        .limit(cpu)
        .run(|_cx, _d: (Arc<Db>, Arc<A>, Arc<B>)| async move { Ok(()) });
    p.build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Finite)
        .expect("valid")
}

fn p17_plan() -> Plan {
    let mut p = Plan::builder("P17");
    let base = p
        .resource("Base")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Db)) })
        .release(|_cx, _d| async move { Ok(()) });
    let mut keys = Vec::new();
    for name in ["A", "B", "C"] {
        keys.push(
            p.resource(name)
                .needs(base)
                .acquire(|cx, _d: Arc<Db>| async move { Ok(cx.hold_value(A)) })
                .release(|_cx, _a| async move { Ok(()) }),
        );
    }
    p.step("Down")
        .needs((keys[0], keys[1], keys[2]))
        .run(|_cx, _d: (Arc<A>, Arc<A>, Arc<A>)| async move { Ok(()) });
    p.build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Finite)
        .expect("valid")
}

fn key(m: &Machine, path: &str) -> RawKey {
    m.key_of(path).unwrap_or_else(|| panic!("no node {path}"))
}

fn spawned(m: &Machine, fx: &[Effect]) -> Vec<String> {
    fx.iter()
        .filter_map(|e| match e {
            Effect::Spawn { node, .. } | Effect::SpawnBlocking { node, .. } => {
                Some(m.path_of(*node).expect("known").to_string())
            }
            _ => None,
        })
        .collect()
}

fn released(m: &Machine, fx: &[Effect]) -> Vec<String> {
    fx.iter()
        .filter_map(|e| match e {
            Effect::Release(node) | Effect::Compensate(node) | Effect::StopService(node) => {
                Some(m.path_of(*node).expect("known").to_string())
            }
            _ => None,
        })
        .collect()
}

fn aborted(m: &Machine, fx: &[Effect]) -> Vec<String> {
    fx.iter()
        .filter_map(|e| match e {
            Effect::Abort(node) => Some(m.path_of(*node).expect("known").to_string()),
            _ => None,
        })
        .collect()
}

fn skipped(fx: &[Effect]) -> Vec<String> {
    fx.iter()
        .filter_map(|e| match e {
            Effect::Emit(ev) => match (&ev.kind, &ev.node) {
                (TraceKind::Skipped { .. }, Some(node)) => Some(node.to_string()),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

fn ended(fx: &[Effect]) -> Option<Outcome> {
    fx.iter().find_map(|e| match e {
        Effect::End(o) => Some(*o),
        _ => None,
    })
}

fn ok(m: &mut Machine, path: &str) -> Vec<Effect> {
    let k = key(m, path);
    let mut fx = m.step(Event::Held(k));
    fx.extend(m.step(Event::NodeOk(k)));
    fx
}

fn err(m: &mut Machine, path: &str) -> Vec<Effect> {
    let k = key(m, path);
    m.step(Event::NodeErr(k, FaultKind::Error("boom".into())))
}

// ---------------------------------------------------------------- P-16

#[test]
fn p16_a_node_is_spawned_only_when_its_needs_are_ready_and_every_grant_is_taken() {
    let plan = p16_plan(false);
    let mut m = Machine::new(&plan).expect("static plan");
    let fx = m.begin();
    assert_eq!(
        spawned(&m, &fx),
        ["Db", "B", "Hog"],
        "roots only; A needs Db"
    );
    assert_eq!(m.run_state(), RunState::Admitting);

    let fx = ok(&mut m, "Db");
    assert_eq!(spawned(&m, &fx), ["A"], "N still needs A and B");
    let fx = ok(&mut m, "B");
    assert!(spawned(&m, &fx).is_empty(), "A is not ready yet: {fx:?}");
    let fx = ok(&mut m, "A");
    assert!(
        spawned(&m, &fx).is_empty(),
        "needs are ready but the pool is full: {fx:?}"
    );
    let why = m.why("N").expect("N exists");
    assert!(
        why.waits_on
            .iter()
            .any(|(p, r)| *p == *"cpu" && *r == Reason::Pool),
        "why must name the pool: {why:?}"
    );
    assert!(
        matches!(m.state_of("N"), Some(NodeState::Waiting { .. })),
        "{:?}",
        m.state_of("N")
    );

    // The pool frees in this step, and the grant is taken in the same step.
    let k = key(&m, "Hog");
    let fx = m.step(Event::NodeOk(k));
    assert_eq!(spawned(&m, &fx), ["N"]);
    assert!(matches!(
        m.state_of("N"),
        Some(NodeState::Running { attempt: 1, .. })
    ));

    // Finite: Steady leads straight to the release graph.
    let k = key(&m, "N");
    let fx = m.step(Event::NodeOk(k));
    assert_eq!(
        released(&m, &fx),
        ["A", "B"],
        "Db waits for A's cleanup to end"
    );
    let k = key(&m, "A");
    let fx = m.step(Event::NodeOk(k));
    assert_eq!(
        released(&m, &fx),
        ["Db"],
        "Db's dependents are A and N only"
    );
    let k = key(&m, "Db");
    let fx = m.step(Event::NodeOk(k));
    assert!(ended(&fx).is_none(), "B's release is still running: {fx:?}");
    let k = key(&m, "B");
    let fx = m.step(Event::NodeOk(k));
    assert_eq!(ended(&fx), Some(Outcome::Ok));
    let report = m.take_report().expect("ended");
    assert!(report.is_clean(), "{report}");
}

#[test]
fn p16_an_exclusive_lock_held_by_a_sibling_is_named_by_why() {
    let plan = p16_plan(true);
    let mut m = Machine::new(&plan).expect("static plan");
    m.begin();
    let fx = ok(&mut m, "Db");
    assert_eq!(spawned(&m, &fx), ["A", "M"], "M takes Db exclusively");
    ok(&mut m, "B");
    ok(&mut m, "A");
    let k = key(&m, "Hog");
    let fx = m.step(Event::NodeOk(k));
    assert!(spawned(&m, &fx).is_empty(), "M holds Db: {fx:?}");
    let why = m.why("N").expect("N");
    assert!(
        why.waits_on
            .iter()
            .any(|(p, r)| *p == *"M" && *r == Reason::Exclusive),
        "{why:?}"
    );
    let k = key(&m, "M");
    let fx = m.step(Event::NodeOk(k));
    assert_eq!(spawned(&m, &fx), ["N"], "both grants become available");
}

// ---------------------------------------------------------------- P-17

#[test]
fn p17_fail_fast_aborts_in_flight_siblings_joins_them_then_releases_in_graph_order() {
    let plan = p17_plan();
    let mut m = Machine::new(&plan).expect("static plan");
    m.begin();
    let fx = ok(&mut m, "Base");
    assert_eq!(spawned(&m, &fx), ["A", "B", "C"]);
    for n in ["A", "B", "C"] {
        let k = key(&m, n);
        assert!(m.step(Event::Started(k)).is_empty());
    }

    let fx = err(&mut m, "B");
    assert_eq!(aborted(&m, &fx), ["A", "C"]);
    assert_eq!(skipped(&fx), ["Down"]);
    assert!(released(&m, &fx).is_empty(), "no release before the joins");
    assert_eq!(m.run_state(), RunState::Settling);

    let k = key(&m, "A");
    let fx = m.step(Event::NodeCancelled {
        node: k,
        held: true,
    });
    assert!(released(&m, &fx).is_empty(), "C is still in flight: {fx:?}");

    let k = key(&m, "C");
    let fx = m.step(Event::NodeCancelled {
        node: k,
        held: false,
    });
    assert_eq!(m.run_state(), RunState::Cleanup);
    assert_eq!(released(&m, &fx), ["A"], "A held; Base waits for A");
    assert!(matches!(
        m.state_of("C"),
        Some(NodeState::Interrupted { held: false })
    ));
    assert!(matches!(
        m.state_of("B"),
        Some(NodeState::Failed { held: false })
    ));

    let k = key(&m, "A");
    let fx = m.step(Event::NodeOk(k));
    assert_eq!(released(&m, &fx), ["Base"]);
    let k = key(&m, "Base");
    let fx = m.step(Event::NodeOk(k));
    assert_eq!(ended(&fx), Some(Outcome::Failed));

    let report = m.take_report().expect("ended");
    let faults: Vec<String> = report.faults.iter().map(|f| f.node.to_string()).collect();
    assert_eq!(faults, ["B"], "C is interrupted, never a second fault");
    assert!(report.incomplete.is_empty() && report.ambiguous.is_empty());

    // D1 totality: a late event after End is refused, never a panic.
    let k = key(&m, "C");
    let fx = m.step(Event::NodeOk(k));
    assert!(matches!(fx.as_slice(), [Effect::Reject(_)]), "{fx:?}");
}

// -------------------------------------------- review remediation (R-4, R-8)

/// `R-4` / F-07 — `OD-PANIC-CANCELLED` on the **drop** path.
///
/// The decision says a body the engine had already cancelled that panics is
/// `Interrupted` and is not a fault. The escape was keyed on `signalled`, so it
/// held only for a cooperative cancel; a drop-mode `Abort` — the default for
/// resources, steps and effects — left `signalled` false and the panic the
/// driver reported on the join became a fault and `Failed`.
#[test]
fn r4_a_panic_joined_after_a_drop_abort_is_not_a_fault() {
    let plan = p17_plan();
    let mut m = Machine::new(&plan).expect("static plan");
    m.begin();
    ok(&mut m, "Base");
    for n in ["A", "B", "C"] {
        let k = key(&m, n);
        m.step(Event::Started(k));
    }
    let fx = err(&mut m, "B");
    assert_eq!(aborted(&m, &fx), ["A", "C"], "drop-mode aborts");

    let k = key(&m, "A");
    m.step(Event::TaskJoined {
        node: k,
        joined: crate::host::engine::JoinedLabel::Panicked,
    });
    assert!(
        matches!(m.state_of("A"), Some(NodeState::Interrupted { .. })),
        "{:?}",
        m.state_of("A")
    );

    let k = key(&m, "C");
    m.step(Event::NodeCancelled {
        node: k,
        held: false,
    });
    let k = key(&m, "Base");
    let fx = m.step(Event::NodeOk(k));
    assert_eq!(ended(&fx), Some(Outcome::Failed));
    let report = m.take_report().expect("ended");
    let faults: Vec<String> = report.faults.iter().map(|f| f.node.to_string()).collect();
    assert_eq!(faults, ["B"], "the panic on the way out is not a fault");
}

/// `R-8` / F-11 — a `NodeCancelled` for a body the engine never aborted.
///
/// INV-7 forbids the engine to cancel a cleanup body, so the event can only
/// mean the *driver* dropped one. It was accepted silently and the node stayed
/// `Releasing` until the budget — for ever under `Shutdown::unbounded()`. D1
/// says an event that makes no sense here is a `Reject`.
#[test]
fn r8_a_cancelled_cleanup_body_is_refused() {
    let plan = p17_plan();
    let mut m = Machine::new(&plan).expect("static plan");
    m.begin();
    ok(&mut m, "Base");
    for n in ["A", "B", "C"] {
        let k = key(&m, n);
        m.step(Event::Started(k));
        m.step(Event::Held(k));
        m.step(Event::NodeOk(k));
    }
    let k = key(&m, "Down");
    m.step(Event::Started(k));
    let fx = m.step(Event::NodeOk(k));
    assert_eq!(released(&m, &fx), ["A", "B", "C"], "{fx:?}");

    let k = key(&m, "A");
    let fx = m.step(Event::NodeCancelled {
        node: k,
        held: true,
    });
    assert!(matches!(fx.as_slice(), [Effect::Reject(_)]), "{fx:?}");
    assert!(matches!(m.state_of("A"), Some(NodeState::Releasing)));
}

/// F-10 — a body outcome for a kind that has no body is refused.
///
/// A component's attempt is its inner graph coming up and a join's is nothing
/// at all: the engine spawns neither, so an outcome for one can only be a
/// driver's mistake. Taken, `NodeErr(component)` gave the component a fault
/// vector the exit helpers assume is always empty, and `NodeOk(component)` made
/// it `Ready` with its inner scope still admitting.
#[test]
fn f10_a_body_outcome_for_a_component_or_a_join_is_refused() {
    let mut p = Plan::builder("Kinds");
    let child = {
        let mut c = Plan::builder("Child");
        c.step("Inner").run(|_cx, ()| async move { Ok(()) });
        c.build(Policy::FailFast, Shutdown::within(secs(4)), Mode::Finite)
            .expect("valid child")
    };
    let a = p.step("A").run(|_cx, ()| async move { Ok(()) });
    let b = p.step("B").run(|_cx, ()| async move { Ok(()) });
    p.join("J", (a, b));
    p.component("C", &child, ());
    let plan = p
        .build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Resident)
        .expect("valid");
    let mut m = Machine::new(&plan).expect("static plan");
    m.begin();
    for path in ["C", "J"] {
        let k = key(&m, path);
        let fx = m.step(Event::NodeOk(k));
        assert!(
            matches!(fx.as_slice(), [Effect::Reject(_)]),
            "{path}: {fx:?}"
        );
        let fx = m.step(Event::NodeErr(k, crate::report::FaultKind::Timeout));
        assert!(
            matches!(fx.as_slice(), [Effect::Reject(_)]),
            "{path}: {fx:?}"
        );
        let fx = m.step(Event::Held(k));
        assert!(
            matches!(fx.as_slice(), [Effect::Reject(_)]),
            "{path}: {fx:?}"
        );
    }
    // And a `Held` for a kind that carries no obligation.
    let k = key(&m, "A");
    let fx = m.step(Event::Held(k));
    assert!(matches!(fx.as_slice(), [Effect::Reject(_)]), "{fx:?}");
}

/// The Stage 1 refusal is gone: `I-30`'s plan runs, and the machine's table
/// carries the template node itself. Its *instances* are appended when a body
/// spawns one (`tests::instances`).
#[test]
fn a_plan_with_a_template_runs_and_the_template_is_a_node() {
    let plan = super::corpus::i30().expect("valid");
    let m = Machine::new(&plan).expect("templates run from Stage 3");
    assert_eq!(m.kind_of(key(&m, "Link")), Some(crate::Kind::Template));
    assert!(
        m.instances().is_empty(),
        "no instance exists before a spawn"
    );
}

/// A body event for a template is refused like one for a component or a join:
/// a template has no body at all (`Stage1-TDD-Log` row 13, extended).
#[test]
fn a_body_event_for_a_template_is_refused() {
    let plan = super::corpus::i30().expect("valid");
    let mut m = Machine::new(&plan).expect("runs");
    m.begin();
    let k = key(&m, "Link");
    for ev in [Event::Started(k), Event::NodeOk(k), Event::Held(k)] {
        let fx = m.step(ev);
        assert!(matches!(fx.as_slice(), [Effect::Reject(_)]), "{fx:?}");
    }
}
