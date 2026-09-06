//! Suite (d), the substrate half: what only a real runtime can answer.
//!
//! `R-04` (a declared pool's bound, measured), `R-06` (panics) and `R-07`
//! (many concurrent runs of one plan). Split from `tests/driver.rs` to keep
//! both files under the house limit; the two are one suite.

use sdax::host::{Observer, Runtime};
use sdax::*;
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

/// Real time, one worker: what a blocking pool needs, because a pool thread
/// cannot advance a paused clock and the auto-advance would fire the shutdown
/// budget while it worked.
fn live() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
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

// --------------------------------------------------------------- R-04

/// How many blocking bodies were inside the pool at once, and the most there
/// ever were.
#[derive(Default)]
struct HighWater {
    now: AtomicUsize,
    max: AtomicUsize,
    ever: AtomicUsize,
}

impl HighWater {
    fn enter(&self) {
        let n = self.now.fetch_add(1, Ordering::SeqCst) + 1;
        self.max.fetch_max(n, Ordering::SeqCst);
        self.ever.fetch_add(1, Ordering::SeqCst);
    }
    fn exit(&self) {
        self.now.fetch_sub(1, Ordering::SeqCst);
    }
}

/// `R-04`: a declared pool bounds the blocking steps that may run at once, and
/// the bound is *measured* by the bodies themselves, not inferred from a trace.
#[test]
fn r04_a_declared_pool_bounds_the_blocking_steps_and_the_bound_is_measured() {
    let hw = Arc::new(HighWater::default());
    let mut p = Plan::builder("Bounded blocking");
    let cpu = p.pool("cpu", 2);
    let snapshot = p
        .resource("Snapshot")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Unit)) })
        .release(|_cx, _u| async move { Ok(()) });
    for i in 0..4 {
        let h = hw.clone();
        p.blocking_step(&format!("Verify{i}"))
            .needs(snapshot)
            .on(cpu)
            .run(move |_cx, _s: Arc<Unit>| {
                h.enter();
                // Stay in the pool until it is as full as it is allowed to
                // get, so that the high-water mark is a real overlap and not
                // an artefact of four bodies that never met. The last body has
                // nobody left to wait for, and a short deadline means a broken
                // bound is a failed assertion rather than a hung suite.
                let deadline = std::time::Instant::now() + Duration::from_millis(200);
                while h.now.load(Ordering::SeqCst) < 2
                    && h.ever.load(Ordering::SeqCst) < 4
                    && std::time::Instant::now() < deadline
                {
                    std::thread::yield_now();
                }
                h.exit();
                Ok(())
            });
    }
    let plan = p
        .build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Finite)
        .expect("valid");
    // Real time: a pool thread cannot advance a paused clock, and the
    // auto-advance would fire the shutdown budget while it worked.
    let tokio_rt = live();
    let rt = adapter(&tokio_rt, Arc::new(sdax::host::NoObserver));
    let report = tokio_rt.block_on(async { plan.start(rt.clone()).await });
    assert_eq!(report.outcome, Outcome::Ok);
    assert!(report.is_clean(), "{:?}", report.faults);
    assert_eq!(
        hw.max.load(Ordering::SeqCst),
        2,
        "the pool's limit is the high-water mark, and it is reached"
    );
    assert_eq!(hw.now.load(Ordering::SeqCst), 0);
    assert_eq!(rt.tracked(), 0);
}

/// `R-05` — F-05: a blocking step whose `within` expires does not
/// over-subscribe its pool.
///
/// The thread cannot be aborted (T7), so the deadline used to fail the attempt,
/// free the pool grant and start the next attempt beside the thread that was
/// still running: two bodies in a pool of one, INV-12 ("attempts never
/// overlap") and INV-15 ("every task the engine spawned has been joined or is
/// listed as abandoned") both false. The bound is measured by the bodies.
#[test]
fn r05_a_blocking_within_does_not_oversubscribe_its_pool() {
    let hw = Arc::new(HighWater::default());
    let mut p = Plan::builder("Blocking timeout");
    let cpu = p.pool("cpu", 1);
    let h = hw.clone();
    p.blocking_step("B")
        .within(Duration::from_millis(50))
        .retry(Retry::attempts(2))
        .on(cpu)
        .run(move |_cx, ()| {
            h.enter();
            // Attempt 1 runs well past its own deadline; attempt 2 is quick.
            if h.ever.load(Ordering::SeqCst) == 1 {
                std::thread::sleep(Duration::from_millis(300));
            }
            h.exit();
            Ok(())
        });
    let plan = p
        .build(Policy::Isolate, Shutdown::within(secs(10)), Mode::Finite)
        .expect("valid");
    // Real time: a pool thread cannot advance a paused clock.
    let tokio_rt = live();
    let rt = adapter(&tokio_rt, Arc::new(sdax::host::NoObserver));
    let report = tokio_rt.block_on(async { plan.start(rt.clone()).await });
    assert_eq!(
        hw.max.load(Ordering::SeqCst),
        1,
        "cpu(1): the timed-out thread keeps its grant until it returns"
    );
    assert_eq!(hw.now.load(Ordering::SeqCst), 0);
    assert_eq!(rt.tracked(), 0, "INV-15: every thread was joined");
    assert_eq!(report.outcome, Outcome::Ok, "{report:?}");
}

// --------------------------------------------------------------- R-06

/// The payload the testkit's panic hook swallows, so a suite that raises
/// dozens of deliberate panics does not bury its own output.
const SCRIPTED: &str = "scripted panic";

/// `R-06`: a panic in a body is a fault and a panic in a release is a cleanup
/// failure; neither is re-raised, and the runtime is still usable afterwards.
#[test]
fn r06_panics_in_a_body_and_in_a_release_are_recorded_never_re_raised() {
    sdax_testkit::quiet_scripted_panics();
    let mut p = Plan::builder("Panics");
    let r = p
        .resource("R")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Unit)) })
        .release(|_cx, _u| async move { panic!("{SCRIPTED}") });
    p.step("S").needs(r).run(|_cx, _r: Arc<Unit>| async move {
        panic!("{SCRIPTED}");
        #[allow(unreachable_code)]
        Ok(())
    });
    let plan = p
        .build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Finite)
        .expect("valid");
    let tokio_rt = paused();
    let rt = adapter(&tokio_rt, Arc::new(sdax::host::NoObserver));
    let report = tokio_rt.block_on(async { plan.start(rt.clone()).await });
    assert_eq!(report.outcome, Outcome::Failed);
    let labels: Vec<_> = report
        .faults
        .iter()
        .map(|f| (f.node.to_string(), f.kind.label()))
        .collect();
    assert_eq!(labels, vec![("S".to_string(), FaultLabel::Panic)]);
    let cleanup: Vec<_> = report
        .cleanup_failures
        .iter()
        .map(|f| (f.node.to_string(), f.kind.label()))
        .collect();
    assert_eq!(cleanup, vec![("R".to_string(), FaultLabel::Panic)]);
    assert_eq!(rt.tracked(), 0);
    // The runtime survived both: a second run on it is unaffected.
    let again = tokio_rt.block_on(async { tiny().start(rt.clone()).await });
    assert_eq!(again.outcome, Outcome::Ok);
}

// --------------------------------------------------------------- R-07

/// `R-07`: many concurrent runs of one plan value share no slots and no pools
/// (INV-13).
///
/// The pool is declared with a limit of **one**, so if the four runs shared it
/// the high-water mark below would be one. It is four.
#[test]
fn r07_concurrent_runs_of_one_plan_share_no_slots_and_no_pools() {
    const RUNS: usize = 4;
    let hw = Arc::new(HighWater::default());
    let ids = Arc::new(AtomicUsize::new(0));
    let mut p = Plan::builder("Isolation");
    let counter = ids.clone();
    let id = p
        .resource("Id")
        .acquire(move |cx, ()| {
            let c = counter.clone();
            async move { Ok(cx.hold_value(c.fetch_add(1, Ordering::SeqCst))) }
        })
        .release(|_cx, _i| async move { Ok(()) });
    let slot = p.pool("slot", 1);
    let h = hw.clone();
    let used = p
        .blocking_step("Use")
        .needs(id)
        .on(slot)
        .run(move |_cx, i: Arc<usize>| {
            h.enter();
            let deadline = std::time::Instant::now() + Duration::from_millis(500);
            while h.now.load(Ordering::SeqCst) < RUNS && std::time::Instant::now() < deadline {
                std::thread::yield_now();
            }
            h.exit();
            Ok(*i)
        });
    let plan = p
        .export(used)
        .build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Finite)
        .expect("valid");

    let tokio_rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_time()
        .build()
        .expect("runtime");
    let rt = adapter(&tokio_rt, Arc::new(sdax::host::NoObserver));
    let outputs = tokio_rt.block_on(async {
        let mut tasks = Vec::new();
        for _ in 0..RUNS {
            let (plan, mine) = (plan.clone(), rt.clone());
            let (tx, rx) = tokio::sync::oneshot::channel();
            rt.spawn(Box::pin(async move {
                let _ = tx.send(plan.start(mine.clone()).await);
            }));
            tasks.push(rx);
        }
        let mut out = Vec::new();
        for rx in tasks {
            out.push(rx.await.expect("a run finished"));
        }
        out
    });
    for r in &outputs {
        assert_eq!(r.outcome, Outcome::Ok);
        assert!(r.is_clean(), "{:?}", r.faults);
    }
    let mut seen: Vec<usize> = outputs
        .iter()
        .map(|r| *r.output.as_deref().expect("an export"))
        .collect();
    seen.sort_unstable();
    assert_eq!(
        seen,
        (0..RUNS).collect::<Vec<_>>(),
        "each run has its own slot"
    );
    assert_eq!(
        hw.max.load(Ordering::SeqCst),
        RUNS,
        "a pool of one per run, not one pool for all of them"
    );
    // Not `tracked() == 0`. On two workers a `Running` resolves before the
    // driver's own tidy-up is reaped — its result crosses a oneshot, and the
    // aborted budget timer and the driver task itself are still on the
    // tracker: measured non-zero in 246 of 300 runs of a two-node plan (max
    // 2). The assertion used to pass by luck. `shutdown` is the documented
    // orphan check and is a bound rather than a snapshot, so that is what is
    // asserted; the run's own end is what `running.await` already proved.
    assert_eq!(
        tokio_rt.block_on(rt.shutdown(secs(5))),
        Ok(()),
        "INV-15: everything the four runs spawned has finished"
    );
}
