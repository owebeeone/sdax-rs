//! Suite (d), the substrate review's edges: `S-01`, `S-03`, `S-04` and `S-06`
//! of `dev-docs/Review-Stage2-Substrate.md`.
//!
//! Each is a case the review reproduced on a real runtime and this crate got
//! wrong: a `ready()` that never returns, a run lost when the runtime is torn
//! down under its drainer, a blocking cleanup whose thread escapes the
//! tracker, and a double hold nobody reports. The observer's own edges
//! (`S-02`, `S-05`, `S-12`) are in `tests/review_observer.rs`.

use sdax::host::{bodies_of, BodySource, CxInner, InstanceId, Observer, RawKey, Runtime, Task};
use sdax::*;
use sdax_testkit::TraceRecorder;
use sdax_tokio::{PlanStart, RunOptions, RunRecord, TokioRuntime};
use std::any::Any;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// Paused time, one worker: a hang shows up as a virtual hour elapsing with
/// nothing to do, which costs no wall clock at all.
fn paused() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .expect("runtime")
}

/// Real time, one worker: what a blocking pool needs.
fn live() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("runtime")
}

fn adapter(rt: &tokio::runtime::Runtime, obs: Arc<dyn Observer>) -> Arc<TokioRuntime> {
    Arc::new(
        TokioRuntime::current_thread_no_background_drain(rt.handle().clone()).with_observer(obs),
    )
}

struct Unit;

/// A resource whose release is counted, and a service that serves until it is
/// told to stop. Enough to have something live when the runtime goes away.
fn res_and_service(released: Arc<Mutex<usize>>) -> Plan {
    let mut p = Plan::builder("RS");
    let r = p
        .resource("R")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Unit)) })
        .release(move |_cx, _u| {
            let n = released.clone();
            async move {
                *n.lock().expect("count") += 1;
                Ok(())
            }
        });
    p.service("W")
        .needs(r)
        .stop_within(secs(2))
        .start(|cx, _t: Arc<Unit>| async move {
            Ok(Serving::new((), async move {
                cx.stop().await;
                Ok(())
            }))
        });
    p.build(Policy::FailFast, Shutdown::within(secs(5)), Mode::Resident)
        .expect("valid")
}

// --------------------------------------------------------------- S-01

/// `S-01`: `ready()` on a plan the machine refuses answers with the refusal.
///
/// `start` is total — awaiting a refused `Running` gives a `Failed` report —
/// but `ready()` used to await a readiness signal nothing would ever request,
/// so the one call documented to make a refused start safe hung for ever. The
/// bound is a virtual hour: under paused time it elapses the moment nothing is
/// left to do, so a hang is a failed assertion and not a stalled suite.
#[test]
fn s01_ready_on_a_refused_plan_answers_with_the_refusal() {
    let mut root = Plan::builder("Root");
    let db = root
        .resource("Db")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Unit)) })
        .release(|_cx, _d| async move { Ok(()) });
    let mut child = Plan::builder("Child");
    let imported = child.import(db);
    child
        .step("Uses")
        .needs(imported)
        .run(|_cx, _d: Arc<Unit>| async move { Ok(()) });
    let child = child
        .build(Policy::Isolate, Shutdown::within(secs(1)), Mode::Finite)
        .expect("valid");

    let tokio_rt = paused();
    let rt = adapter(&tokio_rt, Arc::new(sdax::host::NoObserver));
    let (from_ready, from_handle) = tokio_rt.block_on(async {
        let mut running = child.start(rt.clone(), ());
        let handle = running.handle();
        let a = tokio::time::timeout(secs(3600), running.ready()).await;
        let b = tokio::time::timeout(secs(3600), handle.ready()).await;
        (a, b)
    });
    assert_eq!(
        from_ready,
        Ok(Err(Outcome::Failed)),
        "ready() on a refused plan must answer, not hang"
    );
    assert_eq!(
        from_handle,
        Ok(Err(Outcome::Failed)),
        "RunHandle::ready() sees the same latch"
    );
    // And the report is still the documented one.
    let report = tokio_rt.block_on(async { child.start(rt.clone(), ()).await });
    assert_eq!(report.outcome, Outcome::Failed);
    assert_eq!(report.faults.len(), 1, "{:?}", report.faults);
}

// --------------------------------------------------------------- S-03

/// `S-03`: the tokio runtime torn down under a pending drainer is loud, in
/// **both** drop orders.
///
/// `Running` is dropped inside a `block_on` that returns at once, so the
/// drainer's `Msg::Dropped` is queued and never processed. Whichever of the
/// adapter and the runtime is dropped first, the run is over and nobody will
/// speak for it again — so the driver says so on its way out: one
/// `RuntimeDroppedWithLiveRuns` event and one report.
#[test]
fn s03_a_runtime_dropped_under_the_drainer_is_reported_in_both_orders() {
    for adapter_first in [true, false] {
        let released = Arc::new(Mutex::new(0usize));
        let plan = res_and_service(released.clone());
        let rec = Arc::new(TraceRecorder::new());
        let tokio_rt = paused();
        let rt = adapter(&tokio_rt, rec.clone());
        tokio_rt.block_on(async {
            let mut running = plan.start(rt.clone(), ());
            running.ready().await.expect("steady");
            drop(running);
        });
        if adapter_first {
            drop(rt);
            drop(tokio_rt);
        } else {
            drop(tokio_rt);
            drop(rt);
        }
        assert!(
            rec.contains(&TraceKind::RuntimeDroppedWithLiveRuns),
            "adapter_first={adapter_first}: the loss must be recorded"
        );
        assert_eq!(
            rec.reports().len(),
            1,
            "adapter_first={adapter_first}: exactly one report reaches the observer"
        );
        assert_eq!(
            rec.reports()[0].outcome,
            Outcome::Cancelled,
            "adapter_first={adapter_first}"
        );
    }
}

// --------------------------------------------------------------- S-04

/// A source whose cleanup is a blocking job. The plan's own bodies never
/// produce one; the host API allows it, so the driver has to survive it.
struct BlockingCleanup {
    inner: Arc<dyn BodySource>,
    done: Arc<AtomicBool>,
}

impl BodySource for BlockingCleanup {
    fn body(&self, node: RawKey, instance: Option<InstanceId>, cx: &Arc<CxInner>) -> Option<Task> {
        self.inner.body(node, instance, cx)
    }
    fn cleanup(
        &self,
        _node: RawKey,
        _instance: Option<InstanceId>,
        _cx: &Arc<CxInner>,
    ) -> Option<Task> {
        let d = self.done.clone();
        Some(Task::Blocking(Box::new(move || {
            std::thread::sleep(ms(300));
            d.store(true, Ordering::SeqCst);
            Ok(())
        })))
    }
    fn store(&self, node: RawKey, instance: Option<InstanceId>, v: Box<dyn Any + Send + Sync>) {
        self.inner.store(node, instance, v)
    }
    fn export(&self) -> Option<Box<dyn Any + Send + Sync>> {
        self.inner.export()
    }
    fn open_instance(
        &self,
        template: RawKey,
        parent: Option<InstanceId>,
        id: InstanceId,
        input: Box<dyn Any + Send + Sync>,
    ) {
        self.inner.open_instance(template, parent, id, input)
    }
    fn close_instance(&self, id: InstanceId) {
        self.inner.close_instance(id)
    }
}

/// `S-04`: a blocking cleanup abandoned at the budget stays on the tracker
/// until its thread returns, and announces nothing the machine must refuse.
///
/// A blocking task cannot be aborted. The driver used to abort the *wrapper*
/// that awaits it, which detached the thread: `tracked()` read zero and
/// `shutdown()` said `Ok` while the thread was still working — the one claim
/// INV-15 rests on, false. And every blocking cleanup announced `Started`,
/// which the machine rightly refuses for a node whose release it just opened.
#[test]
fn s04_a_blocking_cleanup_abandoned_at_the_budget_stays_tracked() {
    let mut p = Plan::builder("BC");
    p.resource("R")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Unit)) })
        .release(|_cx, _u| async move { Ok(()) });
    let plan = p
        .build(Policy::FailFast, Shutdown::within(ms(50)), Mode::Resident)
        .expect("valid");
    let done = Arc::new(AtomicBool::new(false));
    let src = Arc::new(BlockingCleanup {
        inner: bodies_of::<(), ()>(&plan),
        done: done.clone(),
    });
    let record = Arc::new(Mutex::new(RunRecord::default()));
    let tokio_rt = live();
    let rt = adapter(&tokio_rt, Arc::new(sdax::host::NoObserver));
    let report = tokio_rt.block_on(async {
        let running = plan.start_with(
            rt.clone(),
            (),
            RunOptions::new().bodies(src).record(record.clone()),
        );
        let h = running.handle();
        rt.spawn(Box::pin(async move {
            tokio::time::sleep(ms(10)).await;
            h.shutdown();
        }));
        running.await
    });
    // A shutdown a `Resident` plan answers is a clean end; the abandoned
    // release is `incomplete`, which is the honest half of T7b.
    assert_eq!(report.outcome, Outcome::Ok, "{report:?}");
    assert_eq!(report.incomplete.len(), 1, "{:?}", report.incomplete);
    let rejections = record.lock().expect("record").rejections.clone();
    assert!(
        rejections.is_empty(),
        "a cleanup body announces no Started: {rejections:?}"
    );
    assert!(
        !done.load(Ordering::SeqCst),
        "the thread is still working when the run ends"
    );
    // Two: the tracked task awaiting the pool job, and the joiner the abort
    // spawned to wait on it. Both are the engine's, and both end when the
    // thread does.
    assert!(
        rt.tracked() >= 1,
        "INV-15: the thread is still ours, so it is still counted"
    );
    let early = tokio_rt.block_on(rt.shutdown(ms(50)));
    assert!(
        early.is_err(),
        "shutdown() must not answer Ok while the thread works: {early:?}"
    );
    let left = tokio_rt.block_on(rt.shutdown(secs(2)));
    assert_eq!(left, Ok(()), "and it waits for it");
    assert!(done.load(Ordering::SeqCst), "the thread finished first");
}

// --------------------------------------------------------------- S-06

/// `S-06`: a body that holds twice and then returns `Err` is a `DoubleHold`.
///
/// Only the `Ok` path used to check `hold_count() > 1`, so a double hold
/// followed by an error was reported as a plain `Error` and the first value's
/// obligation went with the body's locals — no release, no record, and INV-9
/// none the wiser. The seam's one-value rule is broken whatever the body then
/// returned, and that is what the report says.
#[test]
fn s06_a_double_hold_followed_by_an_error_is_reported_as_a_double_hold() {
    let seen = Arc::new(Mutex::new(Vec::<u32>::new()));
    let s = seen.clone();
    let mut p = Plan::builder("DH");
    p.resource("R")
        .acquire(|cx, ()| async move {
            let _first = cx.hold_value(1u32);
            let _second = cx.hold_value(2u32);
            Err::<Held<u32>, Error>("after two holds".into())
        })
        .release(move |_cx, v: Arc<u32>| {
            let s = s.clone();
            async move {
                s.lock().expect("seen").push(*v);
                Ok(())
            }
        });
    let plan = p
        .build(Policy::FailFast, Shutdown::within(secs(5)), Mode::Finite)
        .expect("valid");
    let tokio_rt = paused();
    let rt = adapter(&tokio_rt, Arc::new(sdax::host::NoObserver));
    let report = tokio_rt.block_on(async { plan.start(rt.clone(), ()).await });
    let labels: Vec<_> = report.faults.iter().map(|f| f.kind.label()).collect();
    assert_eq!(
        labels,
        vec![FaultLabel::DoubleHold],
        "the seam's one-value rule is what was broken"
    );
    // INV-3 still holds: the value that *was* banked is discharged.
    assert_eq!(*seen.lock().expect("seen"), vec![2u32]);
    assert_eq!(rt.tracked(), 0);
}
