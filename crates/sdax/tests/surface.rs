//! The public surface, pinned.
//!
//! The crate root and `prelude` are the **author** API; `sdax::host` is what a
//! runtime adapter, a run driver or the engine needs and an author does not.
//! This file names every item at its intended path, so a move or a rename
//! stops it compiling. The mirror image — a host item that must *not* resolve
//! at the root any more — is witness S-01 in `sdax::compile_fail`, because
//! only a program that fails to compile can witness an absence.
//!
//! Imports are deliberately unused in places: the witness is that the path
//! resolves, not that the item is exercised. What the file does exercise, it
//! exercises through the author API alone.

#![allow(unused_imports)]

// ---------------------------------------------------------------- the root

use sdax::{
    release, Acquire, Ambiguity, At, AttrChange, Backoff, Blocking, Body, CancelMode, Child,
    Cleanup, Cx, Deps, Edge, Effect, Effects, Ending, Error, Fault, FaultKind, FaultLabel, Finding,
    Held, Hold, Invalid, Key, Kind, Mode, NeedsCompensate, NeedsRelease, NoAmbiguity, NoPool, Node,
    NodePath, NodeRecord, NodeView, Outcome, Phase, Plan, PlanBuilder, PlanDiff, PlanView, Policy,
    Pool, PoolView, Reason, RecordOrder, Release, ReleaseOrder, ReleaseStyle, Report, Request,
    Resource, Restart, Retry, Rule, Run, Schedule, Script, Serve, Service, Serving, Shutdown,
    SpawnError, Start, Step, Stop, Template, Timeout, Trace, TraceEvent, TraceKind, TryStep, Why,
};

// ---------------------------------------------------------------- the prelude

mod prelude_only {
    //! Everything an author needs, and nothing an adapter needs: this module
    //! builds and inspects a plan with `use sdax::prelude::*` as its only
    //! import.
    use sdax::prelude::*;
    use std::sync::Arc;

    pub struct Db;
    pub struct Receipt;

    pub fn build() -> Result<Plan, Invalid> {
        let mut p = Plan::builder("Surface");
        let db = p
            .resource("Db")
            .within(Duration::from_secs(3))
            .acquire(|cx, ()| async move { Ok(cx.hold_value(Db)) })
            .release(release::by_drop());
        p.effect("Audit")
            .needs(db)
            .on_ambiguous(Ambiguity::Report)
            .perform(|cx, _d: Arc<Db>| async move { Ok(cx.hold_value(Receipt)) })
            .compensate(|_cx, _r| async move { Ok(()) });
        p.build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(10)),
            Mode::Finite,
        )
    }

    /// The prelude carries the report and view vocabulary too, so a caller
    /// reads a run's answer without reaching past it.
    pub fn reads_a_report(r: Report) -> Outcome {
        let _: Vec<&Fault> = r.panics();
        let _: Option<&Trace> = r.trace.as_ref();
        r.outcome
    }

    pub fn reads_a_view(v: &PlanView) -> Option<Why> {
        let _: Vec<Vec<NodePath>> = v.layers();
        let _: ReleaseOrder = v.release_order();
        let _: Effects = v.effects();
        let _: PlanDiff = v.diff(v);
        let _: Vec<Edge> = v.edges.clone();
        v.why("Db")
    }

    /// `Cx`, its four phases, `Held`, `Serving` and `Child`: everything a body
    /// is handed, named from the prelude alone.
    pub fn seam_shapes(
        _a: &Cx<Acquire>,
        _r: &Cx<Run>,
        _s: &Cx<Start>,
        _rel: &Cx<Release>,
        _h: &Held<Db>,
        _sv: &Serving<()>,
        _c: &Child,
    ) {
    }

    /// The three errors an author can see.
    pub fn error_shapes(_e: Error, _t: Timeout, _s: SpawnError) {}

    /// What a declaration hands back, and what `build` hands back when it
    /// refuses.
    pub fn plan_shapes(
        _k: Key<Db>,
        _t: Template<u8>,
        _p: Pool,
        _f: &Finding,
        _r: Rule,
        _i: &Invalid,
    ) {
    }
}

// ---------------------------------------------------------------- the host

use sdax::host::engine::{Effect as EngineEffect, Event, JoinedLabel, Machine, TimerId};
use sdax::host::{
    bodies_of, Bodies, BodySource, BoxFuture, ChildControl, Clock, CxInner, InstanceId, Joined,
    NoObserver, Observer, RawKey, Runtime, Scope, StopSignal, Task, TaskHandle, Time, SEMANTICS,
};

/// The host traits are reachable and usable as bounds and as `dyn`.
fn takes_a_runtime<R: Runtime>(rt: &R) -> Time {
    let _: &dyn Observer = rt.observer();
    rt.clock().now()
}

fn takes_a_clock(c: &dyn Clock) -> BoxFuture<'static, ()> {
    c.sleep(std::time::Duration::from_secs(1))
}

/// A clock that never moves. Real, not a stub: the witnesses below read it.
struct Frozen;

impl Clock for Frozen {
    fn now(&self) -> Time {
        Time::ZERO
    }
    fn sleep(&self, _d: std::time::Duration) -> BoxFuture<'static, ()> {
        Box::pin(std::future::ready(()))
    }
}

fn takes_a_scope(
    s: &dyn Scope,
    ctl: &dyn ChildControl,
    id: InstanceId,
) -> BoxFuture<'static, Result<(), Error>> {
    let child = s.spawn_instance(
        RawKey { plan: 1, idx: 0 },
        RawKey { plan: 1, idx: 1 },
        Box::new(()),
    );
    drop(child);
    ctl.stop(id);
    ctl.ready(id)
}

// ---------------------------------------------------------------- witnesses

#[test]
fn the_author_api_builds_and_inspects_a_plan_through_the_prelude_alone() {
    let plan = prelude_only::build().expect("valid");
    let view = plan.inspect();
    assert_eq!(view.nodes.len(), 2);
    assert!(prelude_only::reads_a_view(&view).is_some());
    assert_eq!(
        prelude_only::reads_a_report(Report::empty(Outcome::Ok)),
        Outcome::Ok
    );
}

#[test]
fn the_host_surface_carries_the_semantics_tag_and_the_engine_vocabulary() {
    assert_eq!(SEMANTICS, "sdax/1");
    assert_eq!(prelude_only::build().expect("valid").semantics(), SEMANTICS);

    let node = RawKey { plan: 1, idx: 0 };
    assert!(format!("{:?}", Event::Started(node)).contains("Started"));
    assert!(format!("{:?}", EngineEffect::Abort(node)).contains("Abort"));
    assert_eq!(TimerId(1), TimerId(1));
    assert!(matches!(JoinedLabel::Done, JoinedLabel::Done));
    assert!(matches!(Joined::Done, Joined::Done));
}

#[test]
fn the_driver_hand_offs_are_reachable_from_another_crate() {
    // The Stage 2 run driver lives in `sdax-tokio`, so `CxInner` and its
    // hand-offs must be usable from outside this crate — under `host`, never
    // at the root.
    let inner = CxInner::new(RawKey { plan: 1, idx: 0 }, std::sync::Arc::new(Frozen));
    let cx: Cx<Acquire> = Cx::new(inner.clone());
    assert_eq!(cx.node(), RawKey { plan: 1, idx: 0 });
    assert_eq!(inner.hold_count(), 0);
    assert!(inner.take_held().is_none());
    assert!(inner.take_serve().is_none());

    let held = cx.hold_value(7u8);
    assert_eq!(*held, 7);
    assert_eq!(inner.hold_count(), 1);
    assert!(inner.take_held().is_some());

    inner.put_output(Box::new(std::sync::Arc::new(1u8)));
    assert!(inner.take_held().is_some());

    let stop: &std::sync::Arc<StopSignal> = inner.stop_signal();
    assert!(!stop.is_stopping());
    stop.request();
    assert!(cx.is_stopping());

    drop(takes_a_clock(&Frozen));
    assert_eq!(takes_a_runtime(&NoRuntime(Frozen)), Time::ZERO);
    let _ = takes_a_scope;
    let _ = prelude_only::seam_shapes;
    let _ = prelude_only::error_shapes;
    let _ = prelude_only::plan_shapes;
    let _: &dyn Observer = &NoObserver;
}

/// A runtime that satisfies the contract without a substrate: it runs nothing,
/// and says so by handing back a task that is already done.
struct NoRuntime(Frozen);
struct NoTask;

impl TaskHandle for NoTask {
    fn abort(&self) {}
    fn join(self) -> BoxFuture<'static, Joined> {
        Box::pin(std::future::ready(Joined::Done))
    }
}

impl Runtime for NoRuntime {
    type Task = NoTask;
    fn spawn(&self, _fut: BoxFuture<'static, ()>) -> NoTask {
        NoTask
    }
    fn spawn_blocking(&self, _f: Box<dyn FnOnce() + Send>) -> NoTask {
        NoTask
    }
    fn clock(&self) -> &dyn Clock {
        &self.0
    }
    fn observer(&self) -> &dyn Observer {
        &NoObserver
    }
}

#[test]
fn unresolved_imports_names_the_import_nodes_rather_than_raw_keys() {
    let mut root = Plan::builder("Root");
    let db = root
        .resource("Db")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(prelude_only::Db)) })
        .release(|_cx, _d| async move { Ok(()) });

    let mut child = Plan::builder("Child");
    let imported = child.import(db);
    child
        .step("Uses")
        .needs(imported)
        .run(|_cx, _d: std::sync::Arc<prelude_only::Db>| async move { Ok(()) });
    let child = child
        .build(
            Policy::Isolate,
            Shutdown::within(std::time::Duration::from_secs(1)),
            Mode::Finite,
        )
        .expect("valid");

    let unresolved: Vec<NodePath> = child.unresolved_imports();
    assert_eq!(unresolved, vec![NodePath::root("import#0")]);
    assert_eq!(unresolved[0].to_string(), "import#0");

    root.component("Child", &child);
    let root = root
        .build(
            Policy::FailFast,
            Shutdown::within(std::time::Duration::from_secs(10)),
            Mode::Finite,
        )
        .expect("valid");
    assert!(
        root.unresolved_imports().is_empty(),
        "a root plan imports nothing"
    );
}

#[test]
fn the_script_vocabulary_is_author_api_at_the_root() {
    // `Plan::simulate` is author API (contract § 9), so the values it takes
    // are too. Stage 1 left them unpinned; this is the pin.
    let script = Script::new()
        .prepare("Db", Body::ok(At::plus(1.0)))
        .body("Audit", [Body::fail(At::tick(2.0), "no"), Body::pending()])
        .serve("Db", [Serve::StopsAfter(std::time::Duration::ZERO)])
        .cleanup("Db", Cleanup::Ok(std::time::Duration::ZERO))
        .at(3.0, Request::Shutdown)
        .schedule(Schedule::order(["Db"]));
    assert_eq!(script.named_nodes(), vec!["Db", "Audit", "Db", "Db"]);
    assert_eq!(script.requests().len(), 1);
    assert!(matches!(script.body_of("Db"), Some([_])));
    assert!(script.serve_of("Audit").is_none());
    assert!(matches!(script.cleanup_of("Db"), Some(Cleanup::Ok(_))));
    assert!(matches!(Ending::Pending, Ending::Pending));

    let plan = prelude_only::build().expect("valid");
    let trace = plan.simulate(&Script::new()).expect("simulates");
    assert!(!trace.events.is_empty());
}

#[test]
fn the_run_drivers_hand_offs_are_reachable_from_another_crate() {
    // Stage 2: the driver lives in `sdax-tokio` and needs the plan's erased
    // bodies, the node table and the deadline the machine would give a body.
    let plan = prelude_only::build().expect("valid");
    let src: std::sync::Arc<dyn BodySource> = bodies_of(&plan);
    assert!(src.export().is_none());

    let machine = Machine::new(&plan).expect("a static plan");
    let nodes = machine.nodes();
    assert_eq!(nodes.len(), 2);
    let (key, path, kind) = nodes[0].clone();
    assert_eq!(path.to_string(), "Db");
    assert_eq!(kind, Kind::Resource);
    assert_eq!(machine.kind_of(key), Some(Kind::Resource));
    // `Db` declares `within(3s)`, and the machine's clock has not moved.
    assert_eq!(
        machine.deadline_for(key),
        Some(Time::ZERO + std::time::Duration::from_secs(3))
    );

    let inner = CxInner::new(key, std::sync::Arc::new(Frozen));
    assert!(matches!(src.body(key, &inner), Some(Task::Async(_))));
    // Nothing has been stored, so the release body has no value to discharge.
    assert!(src.cleanup(key, &inner).is_none());
}

/// `Bodies` names a plan's erased code and the bodies of its components.
/// Never called: the witness is that the path and the signature resolve.
fn _names_the_bodies_type(b: &Bodies) -> (u64, usize) {
    (b.plan(), b.children().len())
}
