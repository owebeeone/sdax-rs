//! Substrate review probes (B): blocking pools, multi-thread, orphans. Throwaway.
use sdax::host::{bodies_of, BodySource, CxInner, Observer, RawKey, Runtime, Task};
use sdax::*;
use sdax_testkit::TraceRecorder;
use sdax_tokio::{FnObserver, PlanStart, TokioRuntime};
use std::any::Any;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}
fn live() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("rt")
}
fn mt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_time()
        .build()
        .expect("rt")
}
fn adapter(rt: &tokio::runtime::Runtime, obs: Arc<dyn Observer>) -> Arc<TokioRuntime> {
    Arc::new(TokioRuntime::new(rt.handle().clone()).with_observer(obs))
}
struct Unit;
fn names(v: &[NodeRecord]) -> Vec<String> {
    v.iter().map(|r| r.node.to_string()).collect()
}

/// B1: a blocking body that never checks `is_stopping` and never returns
/// until told from outside.
#[test]
fn b1_never_returning_blocking_body() {
    let go = Arc::new(AtomicBool::new(false));
    let g = go.clone();
    let mut p = Plan::builder("Stuck");
    let cpu = p.pool("cpu", 1);
    p.blocking_step("B").on(cpu).run(move |_cx, ()| {
        while !g.load(Ordering::SeqCst) {
            std::thread::sleep(ms(5));
        }
        Ok(())
    });
    let plan = p
        .build(Policy::FailFast, Shutdown::within(ms(100)), Mode::Finite)
        .expect("valid");
    let tokio_rt = live();
    let rt = adapter(&tokio_rt, Arc::new(sdax::host::NoObserver));
    let t0 = std::time::Instant::now();
    let report = tokio_rt.block_on(async {
        let running = plan.start(rt.clone());
        let h = running.handle();
        rt.spawn(Box::pin(async move {
            tokio::time::sleep(ms(20)).await;
            h.cancel();
        }));
        running.await
    });
    let tracked = rt.tracked();
    let left = tokio_rt.block_on(rt.shutdown(ms(200)));
    println!(
        "B1 outcome={:?} incomplete={:?} elapsed={:?} tracked_after_await={tracked} shutdown={left:?}",
        report.outcome,
        names(&report.incomplete),
        t0.elapsed()
    );
    // Does Runtime::drop block on the leaked thread?
    let (tx, rx) = std::sync::mpsc::channel();
    let dropper = std::thread::spawn(move || {
        drop(tokio_rt);
        let _ = tx.send(());
    });
    let blocked = rx.recv_timeout(ms(500)).is_err();
    println!("B1 Runtime::drop still blocked after 500ms: {blocked}");
    go.store(true, Ordering::SeqCst);
    dropper.join().expect("dropper");
    println!("B1 Runtime::drop returned once the body did");
}

/// A source whose cleanup is a blocking job (host API allows it).
struct BlockingCleanup {
    inner: Arc<dyn BodySource>,
    inside: Arc<AtomicBool>,
    done: Arc<AtomicBool>,
}
impl BodySource for BlockingCleanup {
    fn body(&self, node: RawKey, cx: &Arc<CxInner>) -> Option<Task> {
        self.inner.body(node, cx)
    }
    fn cleanup(&self, _node: RawKey, _cx: &Arc<CxInner>) -> Option<Task> {
        let (i, d) = (self.inside.clone(), self.done.clone());
        Some(Task::Blocking(Box::new(move || {
            i.store(true, Ordering::SeqCst);
            std::thread::sleep(ms(300));
            d.store(true, Ordering::SeqCst);
            Ok(())
        })))
    }
    fn store(&self, node: RawKey, value: Box<dyn Any + Send + Sync>) {
        self.inner.store(node, value)
    }
    fn export(&self) -> Option<Box<dyn Any + Send + Sync>> {
        self.inner.export()
    }
}

/// B2: T7b aborts a blocking cleanup's wrapper: is the thread still tracked?
#[test]
fn b2_aborting_a_blocking_cleanup_detaches_its_thread() {
    let mut p = Plan::builder("BC");
    p.resource("R")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Unit)) })
        .release(|_cx, _u| async move { Ok(()) });
    let plan = p
        .build(Policy::FailFast, Shutdown::within(ms(50)), Mode::Resident)
        .expect("valid");
    let inside = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));
    let src = Arc::new(BlockingCleanup {
        inner: bodies_of(&plan),
        inside: inside.clone(),
        done: done.clone(),
    });
    let tokio_rt = live();
    let rt = adapter(&tokio_rt, Arc::new(sdax::host::NoObserver));
    let report = tokio_rt.block_on(async {
        let running = plan.start_with(rt.clone(), sdax_tokio::RunOptions::new().bodies(src));
        let h = running.handle();
        rt.spawn(Box::pin(async move {
            tokio::time::sleep(ms(10)).await;
            h.shutdown();
        }));
        running.await
    });
    let tracked = rt.tracked();
    let left = tokio_rt.block_on(rt.shutdown(ms(50)));
    println!(
        "B2 outcome={:?} incomplete={:?} inside={} done={} tracked_after_await={tracked} shutdown(50ms)={left:?}",
        report.outcome,
        names(&report.incomplete),
        inside.load(Ordering::SeqCst),
        done.load(Ordering::SeqCst)
    );
    std::thread::sleep(ms(400));
    println!("B2 thread finished later: done={}", done.load(Ordering::SeqCst));
}

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
        .build(Policy::FailFast, Shutdown::within(ms(1000)), Mode::Finite)
        .expect("valid")
}

/// B3: `tracked()` read right after `running.await` on a multi-thread runtime.
#[test]
fn b3_tracked_right_after_await_on_multi_thread() {
    let tokio_rt = mt();
    let plan = tiny();
    let mut nonzero = 0;
    let mut max = 0;
    for _ in 0..300 {
        let rt = adapter(&tokio_rt, Arc::new(sdax::host::NoObserver));
        let report = tokio_rt.block_on(async { plan.start(rt.clone()).await });
        assert_eq!(report.outcome, Outcome::Ok);
        let t = rt.tracked();
        if t != 0 {
            nonzero += 1;
            max = max.max(t);
        }
        let left = tokio_rt.block_on(rt.shutdown(ms(500)));
        assert_eq!(left, Ok(()));
    }
    println!("B3 tracked()!=0 right after await in {nonzero}/300 runs (max {max}); shutdown() always Ok");
}

/// B6: a blocking body abandoned at the budget while its thread runs on.
#[test]
fn b6_blocking_abandoned_at_budget_thread_runs_on() {
    let finished = Arc::new(AtomicBool::new(false));
    let f = finished.clone();
    let mut p = Plan::builder("Ab");
    let cpu = p.pool("cpu", 1);
    p.blocking_step("B").on(cpu).run(move |_cx, ()| {
        std::thread::sleep(ms(300));
        f.store(true, Ordering::SeqCst);
        Ok(())
    });
    let plan = p
        .build(Policy::FailFast, Shutdown::within(ms(50)), Mode::Finite)
        .expect("valid");
    let tokio_rt = live();
    let rec = Arc::new(TraceRecorder::new());
    let rt = adapter(&tokio_rt, rec.clone());
    let report = tokio_rt.block_on(async {
        let running = plan.start(rt.clone());
        let h = running.handle();
        rt.spawn(Box::pin(async move {
            tokio::time::sleep(ms(10)).await;
            h.cancel();
        }));
        running.await
    });
    let tracked = rt.tracked();
    let left = tokio_rt.block_on(rt.shutdown(ms(1000)));
    println!(
        "B6 outcome={:?} incomplete={:?} abandoned_ev={} finished_at_await={} tracked_after_await={tracked} shutdown(1s)={left:?} finished_now={}",
        report.outcome,
        names(&report.incomplete),
        rec.contains(&TraceKind::Abandoned),
        false,
        finished.load(Ordering::SeqCst)
    );
}

/// A body that holds inside a long poll, so an abort requested meanwhile lands
/// exactly as that poll returns (multi-thread only).
struct SlowHold {
    cx: Cx<Acquire>,
    polls: u32,
    held: Option<Held<Unit>>,
}
impl Future for SlowHold {
    type Output = Result<Held<Unit>, Error>;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.polls += 1;
        match self.polls {
            1 => {
                let w = cx.waker().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(ms(5));
                    w.wake();
                });
                Poll::Pending
            }
            2 => {
                let h = self.cx.hold_value(Unit);
                self.held = Some(h);
                std::thread::sleep(ms(40));
                Poll::Pending
            }
            _ => Poll::Pending,
        }
    }
}

/// B5: hold registered in the poll during which the abort is requested.
#[test]
fn b5_hold_during_abort_on_multi_thread() {
    for bounded in [false, true] {
        let released = Arc::new(AtomicUsize::new(0));
        let rl = released.clone();
        let mut p = Plan::builder("HA");
        p.resource("R")
            .acquire(|cx, ()| SlowHold {
                cx,
                polls: 0,
                held: None,
            })
            .release(move |_cx, _u| {
                let rl = rl.clone();
                async move {
                    rl.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            });
        let shutdown = if bounded {
            Shutdown::within(ms(10))
        } else {
            Shutdown::unbounded()
        };
        let plan = p
            .build(Policy::FailFast, shutdown, Mode::Resident)
            .expect("valid");
        let tokio_rt = mt();
        let rec = Arc::new(TraceRecorder::new());
        let rt = adapter(&tokio_rt, rec.clone());
        let record = Arc::new(std::sync::Mutex::new(sdax_tokio::RunRecord::default()));
        let report = tokio_rt.block_on(async {
            let running = plan.start_with(
                rt.clone(),
                sdax_tokio::RunOptions::new().record(record.clone()),
            );
            let h = running.handle();
            rt.spawn(Box::pin(async move {
                tokio::time::sleep(ms(15)).await;
                h.cancel();
            }));
            running.await
        });
        let left = tokio_rt.block_on(rt.shutdown(ms(1000)));
        let rej = record.lock().unwrap().rejections.clone();
        println!(
            "B5 bounded={bounded}: outcome={:?} held_ev={} interrupted_held={} abandoned={} incomplete={:?} released={} rejections={rej:?} shutdown={left:?}",
            report.outcome,
            rec.contains(&TraceKind::Held),
            rec.contains(&TraceKind::Interrupted { held: true }),
            rec.contains(&TraceKind::Abandoned),
            names(&report.incomplete),
            released.load(Ordering::SeqCst)
        );
    }
}

/// B7: an observer that panics while a service is serving.
#[test]
fn b7_observer_panic_orphans_a_live_service() {
    sdax_testkit::quiet_scripted_panics();
    let stopped = Arc::new(AtomicUsize::new(0));
    let st = stopped.clone();
    let mut p = Plan::builder("OP");
    p.service("W").stop_within(ms(50)).start(move |cx, ()| {
        let st = st.clone();
        async move {
            Ok(Serving::new((), async move {
                cx.stop().await;
                st.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }))
        }
    });
    let plan = p
        .build(Policy::FailFast, Shutdown::within(ms(200)), Mode::Resident)
        .expect("valid");
    let obs: Arc<dyn Observer> = Arc::new(FnObserver(|e: &TraceEvent| {
        if matches!(e.kind, TraceKind::Settling) {
            panic!("scripted panic");
        }
    }));
    let tokio_rt = live();
    let rt = adapter(&tokio_rt, obs);
    let report = tokio_rt.block_on(async {
        let mut running = plan.start(rt.clone());
        running.ready().await.expect("steady");
        running.shutdown();
        running.await
    });
    let left = tokio_rt.block_on(rt.shutdown(ms(300)));
    println!(
        "B7 report={:?} stopped={} tracked={} shutdown(300ms)={left:?}",
        (report.outcome, report.faults.len()),
        stopped.load(Ordering::SeqCst),
        rt.tracked()
    );
    tokio_rt.shutdown_background();
}
