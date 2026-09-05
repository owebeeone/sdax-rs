//! Suite (d) — the run driver on the real substrate.
//!
//! Rows `R-*` are `sdax-v1/B/CanonicalTests.md` § 5; `C-14` is § 4's drop row,
//! deferred to Stage 2 by the Stage 1 report. `R-01` is the whole of suite (c)
//! re-run against the adapter and lives in `tests/conformance.rs`; `R-04`,
//! `R-06` and `R-07` are in `tests/substrate.rs`.
//!
//! Every runtime here is built by hand: the adapter takes no `macros` feature,
//! so there is no `#[tokio::test]` anywhere in this workspace. Bodies are real
//! — this file is where the plan's own code runs, not a script's.

use sdax::host::{bodies_of, BodySource, Observer, Runtime};
use sdax::*;
use sdax_testkit::TraceRecorder;
use sdax_tokio::{PlanStart, TokioRuntime};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn paused() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .expect("runtime")
}

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn adapter(rt: &tokio::runtime::Runtime, obs: Arc<dyn Observer>) -> Arc<TokioRuntime> {
    Arc::new(TokioRuntime::new(rt.handle().clone()).with_observer(obs))
}

struct Unit;

/// A resource, a step that needs it, and an exported value.
fn tiny() -> Plan<u32> {
    let mut p = Plan::builder("Tiny");
    let r = p
        .resource("R")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Unit)) })
        .release(|_cx, _u| async move { Ok(()) });
    let s = p
        .step("S")
        .needs(r)
        .run(|_cx, _r: Arc<Unit>| async move { Ok(7u32) });
    p.export(s)
        .build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Finite)
        .expect("valid")
}

/// The stated Stage 1 blocker: the erased bodies are reachable from another
/// crate, as host API, so a driver can run them.
#[test]
fn r00_the_plans_bodies_are_reachable_as_host_api() {
    let plan = tiny();
    let src: Arc<dyn BodySource> = bodies_of(&plan);
    assert!(src.export().is_none(), "nothing has run yet");
}

/// The happy path end to end: real tasks, a real clock, real bodies.
#[test]
fn r00_a_plan_runs_to_ok_on_the_adapter() {
    let tokio_rt = paused();
    let rt = adapter(&tokio_rt, Arc::new(sdax::host::NoObserver));
    let plan = tiny();
    let report = tokio_rt.block_on(async { plan.start(rt.clone()).await });
    assert_eq!(report.outcome, Outcome::Ok);
    assert_eq!(report.output.as_deref(), Some(&7u32));
    assert!(report.is_clean(), "{:?}", report.faults);
    assert_eq!(rt.tracked(), 0, "no orphaned tasks");
}

/// C-11 on the adapter: the handle is lazy, so a cancel before the first poll
/// ends the run with no body ever spawned.
#[test]
fn c11_cancel_before_the_first_poll_spawns_nothing() {
    let tokio_rt = paused();
    let rec = Arc::new(TraceRecorder::new());
    let rt = adapter(&tokio_rt, rec.clone());
    let plan = tiny();
    let report = tokio_rt.block_on(async {
        let running = plan.start(rt.clone());
        running.cancel();
        running.await
    });
    assert_eq!(report.outcome, Outcome::Cancelled);
    assert!(
        !rec.contains(&TraceKind::Start(Phase::Prepare)),
        "no body was spawned: {:?}",
        rec.trace().events
    );
}

// --------------------------------------------------------------- C-14

/// A transport resource and a service that serves until it is asked to stop.
fn dropped_plan(marks: Arc<Marks>) -> Plan {
    let m1 = marks.clone();
    let m2 = marks.clone();
    let mut p = Plan::builder("Dropped");
    let transport = p
        .resource("Transport")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Unit)) })
        .release(move |_cx, _u| {
            let m = m1.clone();
            async move {
                m.released.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        });
    p.service("Worker")
        .needs(transport)
        .stop_within(secs(2))
        .start(move |cx, _t: Arc<Unit>| {
            let m = m2.clone();
            async move {
                Ok(Serving::new((), async move {
                    cx.stop().await;
                    m.stopped.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }))
            }
        });
    p.build(Policy::FailFast, Shutdown::within(secs(5)), Mode::Resident)
        .expect("valid")
}

#[derive(Default)]
struct Marks {
    released: AtomicUsize,
    stopped: AtomicUsize,
    second_half: AtomicUsize,
}

/// `C-14`: dropping a live `Running` cancels the run, and one engine-owned
/// drainer joins the bodies and runs the release graph inside the budget. The
/// report reaches the observer even though nobody is awaiting it.
#[test]
fn c14_dropping_running_drains_and_reports_to_the_observer() {
    let tokio_rt = paused();
    let rec = Arc::new(TraceRecorder::new());
    let rt = adapter(&tokio_rt, rec.clone());
    let marks = Arc::new(Marks::default());
    let plan = dropped_plan(marks.clone());
    tokio_rt.block_on(async {
        let mut running = plan.start(rt.clone());
        running.ready().await.expect("reaches steady state");
        drop(running);
        // Paused time: this advances only while the drainer is idle, so it is
        // a way of letting the drainer finish, not a wait.
        tokio::time::sleep(secs(60)).await;
    });
    assert!(rec.contains(&TraceKind::DroppedWhileRunning));
    assert_eq!(
        marks.stopped.load(Ordering::SeqCst),
        1,
        "the service stopped"
    );
    assert_eq!(marks.released.load(Ordering::SeqCst), 1, "the release ran");
    let reports = rec.reports();
    assert_eq!(reports.len(), 1, "exactly one report");
    assert_eq!(reports[0].outcome, Outcome::Cancelled);
    assert_eq!(reports[0].incomplete, 0, "nothing was abandoned");
    assert_eq!(rt.tracked(), 0, "the drainer finished and left nothing");
}

// --------------------------------------------------------------- R-02

/// `R-02`: an abort lands between polls, so the engine joins before the node
/// counts as settled — and no release starts before that join.
#[test]
fn r02_the_release_never_precedes_the_join_of_the_aborted_body() {
    let tokio_rt = paused();
    let rec = Arc::new(TraceRecorder::new());
    let rt = adapter(&tokio_rt, rec.clone());
    let marks = Arc::new(Marks::default());
    let (m1, m2) = (marks.clone(), marks.clone());
    let mut p = Plan::builder("Abort");
    p.resource("Slow")
        .acquire(move |cx, ()| {
            let m = m1.clone();
            async move {
                let held = cx.hold_value(Unit);
                cx.sleep(secs(10)).await;
                m.second_half.fetch_add(1, Ordering::SeqCst);
                Ok(held)
            }
        })
        .release(move |_cx, _u| {
            let m = m2.clone();
            async move {
                m.released.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        });
    let plan = p
        .build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Resident)
        .expect("valid");
    let report = tokio_rt.block_on(async {
        let running = plan.start(rt.clone());
        let handle = running.handle();
        rt.spawn(Box::pin(async move {
            tokio::time::sleep(secs(1)).await;
            handle.cancel();
        }));
        running.await
    });
    assert_eq!(report.outcome, Outcome::Cancelled);
    assert_eq!(
        marks.second_half.load(Ordering::SeqCst),
        0,
        "the abort took effect: the body's continuation never ran"
    );
    assert_eq!(marks.released.load(Ordering::SeqCst), 1, "the release ran");
    let events = rec.trace().events;
    let pos = |k: &TraceKind| events.iter().position(|e| &e.kind == k);
    let interrupted = pos(&TraceKind::Interrupted { held: true }).expect("interrupted");
    let release = pos(&TraceKind::ReleaseStart).expect("release start");
    assert!(
        interrupted < release,
        "the join precedes the release (T5): {events:?}"
    );
}

// --------------------------------------------------------------- R-03

/// `R-03`: the runtime's own shutdown answers honestly, and a runtime dropped
/// with tasks of ours still running says so rather than going quiet (INV-15).
#[test]
fn r03_runtime_shutdown_waits_and_reports_what_is_left() {
    let tokio_rt = paused();
    let rt = adapter(&tokio_rt, Arc::new(sdax::host::NoObserver));
    let plan = tiny();
    let left = tokio_rt.block_on(async {
        let report = plan.start(rt.clone()).await;
        assert_eq!(report.outcome, Outcome::Ok);
        rt.shutdown(secs(1)).await
    });
    assert_eq!(left, Ok(()), "a finished run leaves nothing behind");
}

#[test]
fn r03_a_runtime_shutdown_over_a_task_that_will_not_end_says_how_many() {
    let tokio_rt = paused();
    let rt = adapter(&tokio_rt, Arc::new(sdax::host::NoObserver));
    let left = tokio_rt.block_on(async {
        let _task = rt.spawn(Box::pin(std::future::pending::<()>()));
        rt.shutdown(secs(1)).await
    });
    assert_eq!(
        left,
        Err(1),
        "one task is still out there, and it is counted"
    );
}

#[test]
fn r03_a_dropped_runtime_with_live_runs_is_reported() {
    let tokio_rt = paused();
    let rec = Arc::new(TraceRecorder::new());
    tokio_rt.block_on(async {
        let rt = adapter(&tokio_rt, rec.clone());
        let _task = rt.spawn(Box::pin(std::future::pending::<()>()));
        assert_eq!(rt.tracked(), 1);
        drop(rt);
    });
    assert!(
        rec.contains(&TraceKind::RuntimeDroppedWithLiveRuns),
        "{:?}",
        rec.trace().events
    );
}

#[test]
fn r03_a_dropped_runtime_with_nothing_running_is_silent() {
    let tokio_rt = paused();
    let rec = Arc::new(TraceRecorder::new());
    tokio_rt.block_on(async {
        let rt = adapter(&tokio_rt, rec.clone());
        let report = tiny().start(rt.clone()).await;
        assert_eq!(report.outcome, Outcome::Ok);
        drop(rt);
    });
    assert!(!rec.contains(&TraceKind::RuntimeDroppedWithLiveRuns));
}

/// The bug `R-05` found, pinned under paused time.
///
/// A start body that returns `Serving` *after* the engine signalled it is
/// `Interrupted`, not ready (T5). The driver used to spawn the serve future on
/// the body's `Ok` alone, which left a task nobody owned (INV-15) and fed the
/// machine a `ServeEnded` for a service that was not serving (D1). The serve
/// future is now armed only once the machine says the node became `Ready`.
#[test]
fn r05_regression_a_signalled_start_body_never_starts_its_serve_future() {
    let served = Arc::new(AtomicUsize::new(0));
    let s = served.clone();
    let mut p = Plan::builder("Late start");
    p.service("Late").stop_within(secs(5)).start(move |cx, ()| {
        let s = s.clone();
        async move {
            // Returns only once the engine has asked it to stop.
            cx.stop().await;
            Ok(Serving::new((), async move {
                s.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }))
        }
    });
    let plan = p
        .build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Resident)
        .expect("valid");
    let tokio_rt = paused();
    let rt = adapter(&tokio_rt, Arc::new(sdax::host::NoObserver));
    let record = Arc::new(std::sync::Mutex::new(sdax_tokio::RunRecord::default()));
    let report = tokio_rt.block_on(async {
        let running = plan.start_with(
            rt.clone(),
            sdax_tokio::RunOptions::new().record(record.clone()),
        );
        let handle = running.handle();
        rt.spawn(Box::pin(async move {
            tokio::time::sleep(secs(2)).await;
            handle.shutdown();
        }));
        running.await
    });
    assert_eq!(report.outcome, Outcome::Ok);
    assert_eq!(
        served.load(Ordering::SeqCst),
        0,
        "the serve future of an interrupted start never runs"
    );
    let rejections = record.lock().expect("record").rejections.clone();
    assert!(rejections.is_empty(), "{rejections:?}");
    assert_eq!(rt.tracked(), 0, "no orphaned serve task");
}
