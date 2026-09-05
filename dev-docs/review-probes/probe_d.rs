//! Substrate review probes (D): drop on multi-thread with orphans measured,
//! and pool contention with `within` timeouts on multi-thread. Throwaway.
use sdax::host::{Observer, Runtime};
use sdax::*;
use sdax_testkit::mc::prng::{case_seed, SplitMix64};
use sdax_testkit::TraceRecorder;
use sdax_tokio::{PlanStart, TokioRuntime};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}
struct Unit;

#[derive(Default)]
struct Led {
    held: AtomicUsize,
    released: AtomicUsize,
    serving: AtomicUsize,
    stopped: AtomicUsize,
}

/// Captures the report of a dropped run (counts only).
#[derive(Default)]
struct Cap {
    trace: TraceRecorder,
    reports: Mutex<Vec<(Outcome, usize, usize)>>,
}
impl Observer for Cap {
    fn event(&self, e: &TraceEvent) {
        self.trace.event(e)
    }
    fn report(&self, r: &Report<()>) {
        self.reports
            .lock()
            .unwrap()
            .push((r.outcome, r.faults.len(), r.incomplete.len()));
    }
}

fn plan(rng: &mut SplitMix64, led: Arc<Led>) -> Plan {
    let mut p = Plan::builder("DropMT");
    let lock = p
        .resource("Lock")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Unit)) })
        .release(|_cx, _u| async move { Ok(()) });
    let mut keys = Vec::new();
    for name in ["A", "B", "C"] {
        let d = rng.below(3);
        let (l1, l2) = (led.clone(), led.clone());
        keys.push(
            p.resource(name)
                .needs(lock)
                .exclusive(lock)
                .acquire(move |cx, _l: Arc<Unit>| {
                    let l = l1.clone();
                    async move {
                        cx.sleep(ms(d)).await;
                        l.held.fetch_add(1, Ordering::SeqCst);
                        Ok(cx.hold_value(Unit))
                    }
                })
                .release(move |cx, _u| {
                    let l = l2.clone();
                    async move {
                        cx.sleep(ms(d)).await;
                        l.released.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    }
                }),
        );
    }
    let l3 = led.clone();
    p.service("Svc")
        .needs(keys[0])
        .idempotent()
        .restart(Restart::on_error(Backoff::fixed(ms(1))))
        .stop_within(ms(10))
        .start(move |cx, _a: Arc<Unit>| {
            let l = l3.clone();
            async move {
                l.serving.fetch_add(1, Ordering::SeqCst);
                Ok(Serving::new((), async move {
                    match cx.until_stop(cx.sleep(ms(2))).await {
                        None => {
                            l.stopped.fetch_add(1, Ordering::SeqCst);
                            Ok(())
                        }
                        Some(()) => Err("tick".into()),
                    }
                }))
            }
        });
    p.build(Policy::Isolate, Shutdown::within(ms(40)), Mode::Resident)
        .expect("valid")
}

/// D1: drop `Running` at a random moment on two workers; measure orphans with
/// `shutdown()`, and that exactly one report reached the observer.
#[test]
fn d1_drop_on_multi_thread_orphans_measured() {
    let cases: u64 = std::env::var("PROBE_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(150);
    let tokio_rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_time()
        .build()
        .expect("rt");
    let mut bad = Vec::new();
    let mut hist: Vec<(String, u64)> = Vec::new();
    for i in 0..cases {
        let seed = case_seed(4242, i);
        let mut rng = SplitMix64::new(seed);
        let led = Arc::new(Led::default());
        let plan = plan(&mut rng, led.clone());
        let at = rng.below(12);
        let cap = Arc::new(Cap::default());
        let obs: Arc<dyn Observer> = cap.clone();
        let rt = Arc::new(TokioRuntime::new(tokio_rt.handle().clone()).with_observer(obs));
        tokio_rt.block_on(async {
            let mut running = plan.start(rt.clone());
            let _ = tokio::time::timeout(ms(at), running.ready()).await;
            drop(running);
        });
        let left = tokio_rt.block_on(rt.shutdown(ms(2000)));
        let reports = cap.reports.lock().unwrap().clone();
        let mut problems = Vec::new();
        if left != Ok(()) {
            problems.push(format!("orphans after drop: {left:?}"));
        }
        if reports.len() != 1 {
            problems.push(format!("reports: {reports:?}"));
        }
        let (held, released) = (
            led.held.load(Ordering::SeqCst),
            led.released.load(Ordering::SeqCst),
        );
        let incomplete = reports.first().map(|r| r.2).unwrap_or(0);
        if held != released + incomplete {
            problems.push(format!("ledger held {held} released {released} incomplete {incomplete}"));
        }
        if led.stopped.load(Ordering::SeqCst) > led.serving.load(Ordering::SeqCst) {
            problems.push("stopped > serving".to_string());
        }
        if !cap.trace.contains(&TraceKind::DroppedWhileRunning) {
            problems.push("no DroppedWhileRunning".to_string());
        }
        let key = reports
            .first()
            .map(|r| format!("{:?}", r.0))
            .unwrap_or_else(|| "none".to_string());
        match hist.iter_mut().find(|(k, _)| *k == key) {
            Some((_, n)) => *n += 1,
            None => hist.push((key, 1)),
        }
        if !problems.is_empty() {
            bad.push((seed, at, problems));
        }
    }
    println!("D1 {cases} drops on multi-thread: {hist:?}; failing: {}", bad.len());
    for (seed, at, p) in bad.iter().take(12) {
        println!("  seed {seed} at {at}ms: {p:?}");
    }
    tokio_rt.shutdown_background();
}

#[derive(Default)]
struct HighWater {
    now: AtomicUsize,
    max: AtomicUsize,
}

/// D2: pool(2), six blocking bodies with `within` and retry, cancelled
/// mid-way, on two workers: the high-water mark must never exceed 2.
#[test]
fn d2_pool_high_water_under_contention_with_timeouts() {
    let tokio_rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_time()
        .build()
        .expect("rt");
    let mut worst = 0;
    let mut problems = Vec::new();
    for i in 0..30u64 {
        let hw = Arc::new(HighWater::default());
        let mut p = Plan::builder("PoolMT");
        let cpu = p.pool("cpu", 2);
        for j in 0..6 {
            let h = hw.clone();
            p.blocking_step(&format!("V{j}"))
                .on(cpu)
                .within(ms(3))
                .retry(Retry::attempts(2))
                .run(move |_cx, ()| {
                    let n = h.now.fetch_add(1, Ordering::SeqCst) + 1;
                    h.max.fetch_max(n, Ordering::SeqCst);
                    // odd bodies overrun their deadline
                    std::thread::sleep(ms(if j % 2 == 1 { 6 } else { 1 }));
                    h.now.fetch_sub(1, Ordering::SeqCst);
                    Ok(())
                });
        }
        let plan = p
            .build(Policy::Isolate, Shutdown::within(ms(60)), Mode::Finite)
            .expect("valid");
        let rt = Arc::new(TokioRuntime::new(tokio_rt.handle().clone()));
        let record = Arc::new(Mutex::new(sdax_tokio::RunRecord::default()));
        let report = tokio_rt.block_on(async {
            let running = plan.start_with(
                rt.clone(),
                sdax_tokio::RunOptions::new().record(record.clone()),
            );
            let h = running.handle();
            let at = 2 + (i % 6);
            rt.spawn(Box::pin(async move {
                tokio::time::sleep(ms(at)).await;
                h.cancel();
            }));
            tokio::time::timeout(ms(5000), running).await
        })
        .expect("ended");
        let left = tokio_rt.block_on(rt.shutdown(ms(2000)));
        let max = hw.max.load(Ordering::SeqCst);
        worst = worst.max(max);
        let rej = record.lock().unwrap().rejections.clone();
        if max > 2 || left != Ok(()) || !rej.is_empty() {
            problems.push(format!(
                "iter {i}: max {max} shutdown {left:?} rejections {rej:?} outcome {:?}",
                report.outcome
            ));
        }
    }
    println!("D2 30 iterations: worst high-water {worst} (limit 2); problems: {problems:?}");
    tokio_rt.shutdown_background();
}
