//! Suite (d), the observer boundary: `S-02`, `S-05` and `S-12` of
//! `dev-docs/Review-Stage2-Substrate.md`.
//!
//! Contract § 10 says an [`Observer`] "must not block and must not panic", and
//! a panicking one is a user-obligation violation. What made it a defect of
//! *this* crate is the consequence: the callback ran on the driver task with
//! no boundary, so a panic there killed the run driver — every live body
//! orphaned for ever, no report, `ready()` waiting on a latch nobody would
//! ever set, and the awaiter handed an empty `Cancelled`. That is INV-15's
//! failure mode, reached silently. The driver now catches it, records it and
//! keeps driving.

use sdax::host::Observer;
use sdax::*;
use sdax_testkit::TraceRecorder;
use sdax_tokio::{PlanStart, TokioRuntime};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn paused() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .expect("runtime")
}

fn adapter(rt: &tokio::runtime::Runtime, obs: Arc<dyn Observer>) -> Arc<TokioRuntime> {
    Arc::new(
        TokioRuntime::current_thread_no_background_drain(rt.handle().clone()).with_observer(obs),
    )
}

struct Unit;

/// What the plan below actually did, counted by the bodies themselves rather
/// than read off a trace the panicking observer never received.
#[derive(Default)]
struct Marks {
    released: AtomicUsize,
    stopped: AtomicUsize,
}

/// A resource and a service that serves until `cx.stop()`. If the driver dies,
/// the service is never signalled and `stopped` stays zero for ever.
fn res_and_service(m: Arc<Marks>) -> Plan {
    let (m1, m2) = (m.clone(), m.clone());
    let mut p = Plan::builder("RS");
    let r = p
        .resource("R")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Unit)) })
        .release(move |_cx, _u| {
            let m = m1.clone();
            async move {
                m.released.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        });
    p.service("W")
        .needs(r)
        .stop_within(secs(2))
        .initialize(|_cx, _t: Arc<Unit>| async move { Ok(()) })
        .serve(move |cx, _handle| {
            let m = m2.clone();
            async move {
                cx.stop().await;
                m.stopped.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        });
    p.build(Policy::FailFast, Shutdown::within(secs(5)), Mode::Resident)
        .expect("valid")
}

/// An observer that panics on one kind of event and records the rest.
struct PanicOn {
    on: TraceKind,
    rec: TraceRecorder,
    report_panics: bool,
}

impl Observer for PanicOn {
    fn event(&self, e: &TraceEvent) {
        if e.kind == self.on {
            panic!("scripted panic");
        }
        self.rec.event(e);
    }
    fn report(&self, r: &Report<()>) {
        if self.report_panics {
            panic!("scripted panic");
        }
        self.rec.report(r);
    }
}

// --------------------------------------------------------------- S-02

/// `S-02`: an `Observer::event` that panics does not take the run with it.
///
/// The panic is contained where it happens, recorded as
/// [`TraceKind::ObserverPanicked`], and the driver goes on: the service is
/// stopped, the resource released, the report delivered, and nothing of the
/// engine's is left running.
#[test]
fn s02_an_observer_panic_in_event_does_not_kill_the_driver() {
    sdax_testkit::quiet_scripted_panics();
    let marks = Arc::new(Marks::default());
    let plan = res_and_service(marks.clone());
    let obs = Arc::new(PanicOn {
        on: TraceKind::Settling,
        rec: TraceRecorder::new(),
        report_panics: false,
    });
    let tokio_rt = paused();
    let rt = adapter(&tokio_rt, obs.clone());
    let (ready, report) = tokio_rt.block_on(async {
        let mut running = plan.start(rt.clone(), ());
        let ready = tokio::time::timeout(secs(3600), running.ready()).await;
        running.shutdown();
        let report = tokio::time::timeout(secs(3600), running).await;
        (ready, report)
    });
    assert_eq!(ready, Ok(Ok(())), "ready() must not wait on a dead driver");
    let report = report.expect("the run must end, not hang");
    assert_eq!(report.outcome, Outcome::Ok, "{report:?}");
    assert_eq!(
        marks.stopped.load(Ordering::SeqCst),
        1,
        "the service was signalled and stopped"
    );
    assert_eq!(marks.released.load(Ordering::SeqCst), 1, "R was released");
    assert_eq!(rt.tracked(), 0, "INV-15: nothing of the engine's is left");
    let trace = report.trace.as_ref().expect("a trace");
    assert!(
        trace
            .events
            .iter()
            .any(|e| e.kind == TraceKind::ObserverPanicked),
        "the panic is recorded, not swallowed: {:?}",
        trace.events.iter().map(|e| &e.kind).collect::<Vec<_>>()
    );
    assert!(
        matches!(
            trace.events.last().map(|e| &e.kind),
            Some(TraceKind::End(_))
        ),
        "T8: End is still the last event"
    );
    assert_eq!(obs.rec.reports().len(), 1, "the report still reaches it");
}

// --------------------------------------------------------------- S-05

/// `S-05`: an `Observer::report` that panics does not corrupt the awaiter's
/// report.
///
/// `report()` used to run before `done.send`, so a panic there dropped the
/// sender and the awaiter of a run that ended `Ok` was told `Cancelled` with
/// nothing in it — the one case where the report is the only record.
#[test]
fn s05_an_observer_panic_in_report_still_delivers_the_report() {
    sdax_testkit::quiet_scripted_panics();
    let marks = Arc::new(Marks::default());
    let plan = res_and_service(marks.clone());
    let obs = Arc::new(PanicOn {
        on: TraceKind::RuntimeDroppedWithLiveRuns, // never emitted here
        rec: TraceRecorder::new(),
        report_panics: true,
    });
    let tokio_rt = paused();
    let rt = adapter(&tokio_rt, obs.clone());
    let report = tokio_rt.block_on(async {
        let mut running = plan.start(rt.clone(), ());
        running.ready().await.expect("steady");
        running.shutdown();
        tokio::time::timeout(secs(3600), running).await
    });
    let report = report.expect("the run must end");
    assert_eq!(
        report.outcome,
        Outcome::Ok,
        "the awaiter gets the run's own outcome: {report:?}"
    );
    assert_eq!(marks.released.load(Ordering::SeqCst), 1);
    assert_eq!(marks.stopped.load(Ordering::SeqCst), 1);
    assert_eq!(rt.tracked(), 0);
    let trace = report.trace.as_ref().expect("a trace");
    assert!(
        trace
            .events
            .iter()
            .any(|e| e.kind == TraceKind::ObserverPanicked),
        "the awaiter's copy says the observer panicked"
    );
    assert!(
        matches!(
            trace.events.last().map(|e| &e.kind),
            Some(TraceKind::End(_))
        ),
        "T8: End is still the last event"
    );
}
