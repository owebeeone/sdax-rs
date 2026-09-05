//! Substrate review probes (A): hangs, panics, drop order. Throwaway.
use sdax::host::{Observer, Runtime};
use sdax::*;
use sdax_testkit::TraceRecorder;
use sdax_tokio::{FnObserver, PlanStart, TokioRuntime};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
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

/// Resource + service that serves until stopped; counts releases and stops.
#[derive(Default)]
struct Marks {
    released: AtomicUsize,
    stopped: AtomicUsize,
    serving: AtomicUsize,
}
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
        .start(move |cx, _t: Arc<Unit>| {
            let m = m2.clone();
            async move {
                m.serving.fetch_add(1, Ordering::SeqCst);
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

/// A1: `Running::ready()` on a plan the machine refuses (L-IMPORTS).
#[test]
fn a1_ready_on_a_refused_plan() {
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
    let outcome = tokio_rt.block_on(async {
        let mut running = child.start(rt.clone());
        // Paused time: a hang shows as the timeout firing after auto-advance.
        tokio::time::timeout(secs(3600), running.ready()).await
    });
    println!("A1 ready() on refused plan -> {outcome:?}");
    let report = tokio_rt.block_on(async { child.start(rt.clone()).await });
    println!("A1 awaiting the refused Running -> {:?}", report.outcome);
}

/// A2: an observer that panics in `event()`.
#[test]
fn a2_observer_panics_in_event() {
    sdax_testkit::quiet_scripted_panics();
    let marks = Arc::new(Marks::default());
    let plan = res_and_service(marks.clone());
    let obs: Arc<dyn Observer> = Arc::new(FnObserver(|e: &TraceEvent| {
        if matches!(e.kind, TraceKind::Ready) {
            panic!("scripted panic");
        }
    }));
    let tokio_rt = paused();
    let rt = adapter(&tokio_rt, obs);
    let (report, ready) = tokio_rt.block_on(async {
        let mut running = plan.start(rt.clone());
        let handle = running.handle();
        let ready = tokio::time::timeout(secs(60), running.ready()).await;
        let _ = handle;
        let report = tokio::time::timeout(secs(60), running).await;
        tokio::time::sleep(secs(600)).await;
        (report, ready)
    });
    println!(
        "A2 ready={ready:?} report={:?} tracked={} serving={} stopped={} released={}",
        report.as_ref().map(|r| (r.outcome, r.faults.len())),
        rt.tracked(),
        marks.serving.load(Ordering::SeqCst),
        marks.stopped.load(Ordering::SeqCst),
        marks.released.load(Ordering::SeqCst)
    );
    let left = tokio_rt.block_on(rt.shutdown(secs(1)));
    println!("A2 rt.shutdown -> {left:?}");
}

struct PanicOnReport(TraceRecorder);
impl Observer for PanicOnReport {
    fn event(&self, e: &TraceEvent) {
        self.0.event(e)
    }
    fn report(&self, _r: &Report<()>) {
        panic!("scripted panic");
    }
}

/// A3: an observer that panics in `report()`.
#[test]
fn a3_observer_panics_in_report() {
    sdax_testkit::quiet_scripted_panics();
    let marks = Arc::new(Marks::default());
    let plan = res_and_service(marks.clone());
    let rec = Arc::new(PanicOnReport(TraceRecorder::new()));
    let tokio_rt = paused();
    let rt = adapter(&tokio_rt, rec.clone());
    let report = tokio_rt.block_on(async {
        let running = plan.start(rt.clone());
        let h = running.handle();
        rt.spawn(Box::pin(async move {
            tokio::time::sleep(secs(1)).await;
            h.shutdown();
        }));
        let r = tokio::time::timeout(secs(60), running).await;
        tokio::time::sleep(secs(60)).await;
        r
    });
    println!(
        "A3 report={:?} tracked={} released={} stopped={} end_in_trace={}",
        report.as_ref().map(|r| (r.outcome, r.faults.len())),
        rt.tracked(),
        marks.released.load(Ordering::SeqCst),
        marks.stopped.load(Ordering::SeqCst),
        rec.0.contains(&TraceKind::Settling)
    );
}

/// A4: drop `Running`, then drop the tokio runtime before the drainer runs.
#[test]
fn a4_runtime_dropped_under_the_drainer() {
    for adapter_first in [true, false] {
        let marks = Arc::new(Marks::default());
        let plan = res_and_service(marks.clone());
        let rec = Arc::new(TraceRecorder::new());
        let tokio_rt = paused();
        let rt = adapter(&tokio_rt, rec.clone());
        tokio_rt.block_on(async {
            let mut running = plan.start(rt.clone());
            running.ready().await.expect("steady");
            drop(running);
        });
        let tracked = rt.tracked();
        if adapter_first {
            drop(rt);
            drop(tokio_rt);
        } else {
            drop(tokio_rt);
            drop(rt);
        }
        println!(
            "A4 adapter_first={adapter_first}: tracked_before={tracked} released={} stopped={} reports={} dropped_ev={} rt_dropped_ev={}",
            marks.released.load(Ordering::SeqCst),
            marks.stopped.load(Ordering::SeqCst),
            rec.reports().len(),
            rec.contains(&TraceKind::DroppedWhileRunning),
            rec.contains(&TraceKind::RuntimeDroppedWithLiveRuns)
        );
    }
}

/// A5: on current_thread the drainer only progresses inside a block_on.
#[test]
fn a5_drainer_is_frozen_between_block_ons_on_current_thread() {
    let marks = Arc::new(Marks::default());
    let plan = res_and_service(marks.clone());
    let rec = Arc::new(TraceRecorder::new());
    let tokio_rt = paused();
    let rt = adapter(&tokio_rt, rec.clone());
    tokio_rt.block_on(async {
        let mut running = plan.start(rt.clone());
        running.ready().await.expect("steady");
        drop(running);
    });
    let (r1, s1, t1) = (
        marks.released.load(Ordering::SeqCst),
        marks.stopped.load(Ordering::SeqCst),
        rt.tracked(),
    );
    std::thread::sleep(Duration::from_millis(100));
    let (r2, t2) = (marks.released.load(Ordering::SeqCst), rt.tracked());
    tokio_rt.block_on(async { tokio::time::sleep(secs(60)).await });
    let (r3, t3) = (marks.released.load(Ordering::SeqCst), rt.tracked());
    println!("A5 after block_on: released={r1} stopped={s1} tracked={t1}; after 100ms wall: released={r2} tracked={t2}; after another block_on: released={r3} tracked={t3} reports={}", rec.reports().len());
}

/// A6: a blocking body that panics.
#[test]
fn a6_blocking_body_panics() {
    sdax_testkit::quiet_scripted_panics();
    let mut p = Plan::builder("BP");
    let cpu = p.pool("cpu", 1);
    let s = p.blocking_step("B").on(cpu).run(|_cx, ()| {
        panic!("scripted panic");
        #[allow(unreachable_code)]
        Ok(7u32)
    });
    let plan = p
        .export(s)
        .build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Finite)
        .expect("valid");
    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("rt");
    let rt = adapter(&tokio_rt, Arc::new(sdax::host::NoObserver));
    let report = tokio_rt.block_on(async { plan.start(rt.clone()).await });
    let labels: Vec<_> = report
        .faults
        .iter()
        .map(|f| (f.node.to_string(), f.phase, f.kind.label()))
        .collect();
    let left = tokio_rt.block_on(rt.shutdown(secs(1)));
    println!("A6 outcome={:?} faults={labels:?} shutdown={left:?}", report.outcome);
}

/// A7: a serve future that panics; A8: a panic inside `hold`'s effect future;
/// A9: a body local whose Drop panics during the abort.
#[test]
fn a7_a8_a9_panic_sites() {
    sdax_testkit::quiet_scripted_panics();
    // A7
    let mut p = Plan::builder("SP");
    p.service("S")
        .stop_within(secs(1))
        .start(|_cx, ()| async move {
            Ok(Serving::new((), async move {
                panic!("scripted panic");
                #[allow(unreachable_code)]
                Ok(())
            }))
        });
    let plan = p
        .build(Policy::FailFast, Shutdown::within(secs(5)), Mode::Resident)
        .expect("valid");
    let tokio_rt = paused();
    let rt = adapter(&tokio_rt, Arc::new(sdax::host::NoObserver));
    let report = tokio_rt.block_on(async { plan.start(rt.clone()).await });
    let labels: Vec<_> = report
        .faults
        .iter()
        .map(|f| (f.node.to_string(), f.phase, f.kind.label()))
        .collect();
    println!("A7 serve panic: outcome={:?} faults={labels:?} tracked={}", report.outcome, rt.tracked());
    // A8
    let released = Arc::new(AtomicUsize::new(0));
    let rl = released.clone();
    let mut p = Plan::builder("HP");
    p.effect("E")
        .on_ambiguous(Ambiguity::Report)
        .perform(|cx, ()| async move {
            let h = cx
                .hold(async move {
                    panic!("scripted panic");
                    #[allow(unreachable_code)]
                    Ok::<Unit, Error>(Unit)
                })
                .await?;
            Ok(h)
        })
        .compensate(move |_cx, _r| {
            let rl = rl.clone();
            async move {
                rl.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        });
    let plan = p
        .build(Policy::FailFast, Shutdown::within(secs(5)), Mode::Finite)
        .expect("valid");
    let report = tokio_rt.block_on(async { plan.start(rt.clone()).await });
    let labels: Vec<_> = report
        .faults
        .iter()
        .map(|f| (f.node.to_string(), f.phase, f.kind.label()))
        .collect();
    println!(
        "A8 hold-effect panic: outcome={:?} faults={labels:?} ambiguous={} compensated={} tracked={}",
        report.outcome,
        report.ambiguous.len(),
        released.load(Ordering::SeqCst),
        rt.tracked()
    );
    // A9
    struct PanicDrop;
    impl Drop for PanicDrop {
        fn drop(&mut self) {
            if !std::thread::panicking() {
                panic!("scripted panic");
            }
        }
    }
    let rec = Arc::new(TraceRecorder::new());
    let rt2 = adapter(&tokio_rt, rec.clone());
    let mut p = Plan::builder("DP");
    p.resource("R")
        .acquire(|cx, ()| async move {
            let _guard = PanicDrop;
            let h = cx.hold_value(Unit);
            cx.sleep(secs(100)).await;
            Ok(h)
        })
        .release(|_cx, _u| async move { Ok(()) });
    let plan = p
        .build(Policy::FailFast, Shutdown::within(secs(5)), Mode::Resident)
        .expect("valid");
    let report = tokio_rt.block_on(async {
        let running = plan.start(rt2.clone());
        let h = running.handle();
        rt2.spawn(Box::pin(async move {
            tokio::time::sleep(secs(1)).await;
            h.cancel();
        }));
        running.await
    });
    println!(
        "A9 panicking drop on abort: outcome={:?} faults={} cleanup_failures={} interrupted_held={} release_ok={} tracked={}",
        report.outcome,
        report.faults.len(),
        report.cleanup_failures.len(),
        rec.contains(&TraceKind::Interrupted { held: true }),
        rec.contains(&TraceKind::ReleaseOk),
        rt2.tracked()
    );
}

/// A10: hold twice then return Err — is the double hold reported? which value
/// does the release get? A11: drop before first poll — any report?
#[test]
fn a10_a11_double_hold_and_unpolled_drop() {
    let seen = Arc::new(Mutex::new(Vec::<u32>::new()));
    let s = seen.clone();
    let mut p = Plan::builder("DH");
    p.resource("R")
        .acquire(|cx, ()| async move {
            let _a = cx.hold_value(1u32);
            let _b = cx.hold_value(2u32);
            Err::<Held<u32>, Error>("after two holds".into())
        })
        .release(move |_cx, v: Arc<u32>| {
            let s = s.clone();
            async move {
                s.lock().unwrap().push(*v);
                Ok(())
            }
        });
    let plan = p
        .build(Policy::FailFast, Shutdown::within(secs(5)), Mode::Finite)
        .expect("valid");
    let rec = Arc::new(TraceRecorder::new());
    let tokio_rt = paused();
    let rt = adapter(&tokio_rt, rec.clone());
    let report = tokio_rt.block_on(async { plan.start(rt.clone()).await });
    let labels: Vec<_> = report.faults.iter().map(|f| f.kind.label()).collect();
    println!(
        "A10 double hold then Err: faults={labels:?} held_events={} released_values={:?}",
        rec.trace().events.iter().filter(|e| e.kind == TraceKind::Held).count(),
        seen.lock().unwrap()
    );
    // A11
    let rec2 = Arc::new(TraceRecorder::new());
    let rt2 = adapter(&tokio_rt, rec2.clone());
    let marks = Arc::new(Marks::default());
    let plan = res_and_service(marks.clone());
    let flag = Arc::new(AtomicBool::new(false));
    tokio_rt.block_on(async {
        let running = plan.start(rt2.clone());
        drop(running);
        tokio::time::sleep(secs(60)).await;
        flag.store(true, Ordering::SeqCst);
    });
    println!(
        "A11 unpolled drop: reports={} events={} tracked={}",
        rec2.reports().len(),
        rec2.len(),
        rt2.tracked()
    );
}
