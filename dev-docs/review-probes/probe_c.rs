//! Substrate review probes (C): multi-thread stress with real bodies, timer
//! ties, and the B1 timing question. Throwaway.
use sdax::host::engine::Machine;
use sdax::host::{Observer, Runtime};
use sdax::*;
use sdax_testkit::mc::prng::{case_seed, SplitMix64};
use sdax_testkit::{Driven, Recorded, TraceRecorder};
use sdax_tokio::{PlanStart, RunOptions, RunRecord, TokioRuntime};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}
fn adapter(rt: &tokio::runtime::Runtime, obs: Arc<dyn Observer>) -> Arc<TokioRuntime> {
    Arc::new(TokioRuntime::new(rt.handle().clone()).with_observer(obs))
}
struct Unit;

thread_local! {
    static KINDS: std::cell::RefCell<Vec<(String, u64)>> = std::cell::RefCell::new(Vec::new());
}

#[derive(Default)]
struct Ledger {
    held: AtomicUsize,
    released: AtomicUsize,
    serving: AtomicUsize,
    stopped: AtomicUsize,
    blk_started: AtomicUsize,
    blk_done: AtomicUsize,
}

/// Exclusive contention on one lock, a restarting service under shutdown, a
/// component, and a blocking step — all with real bodies on a real clock.
fn contended(rng: &mut SplitMix64, led: Arc<Ledger>) -> Plan {
    let policy = if rng.chance(0.5) {
        Policy::FailFast
    } else {
        Policy::Isolate
    };
    let mut inner = Plan::builder("Net");
    let l = led.clone();
    let x = inner
        .resource("X")
        .acquire(move |cx, ()| {
            let l = l.clone();
            async move {
                cx.sleep(ms(1)).await;
                l.held.fetch_add(1, Ordering::SeqCst);
                Ok(cx.hold_value(Unit))
            }
        })
        .release({
            let l = led.clone();
            move |_cx, _u| {
                let l = l.clone();
                async move {
                    l.released.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            }
        });
    let y = inner
        .step("Y")
        .needs(x)
        .run(|cx, _x: Arc<Unit>| async move {
            cx.sleep(ms(1)).await;
            Ok(3u32)
        });
    let net = inner
        .export(y)
        .build(Policy::FailFast, Shutdown::within(ms(30)), Mode::Finite)
        .expect("valid");

    let mut p = Plan::builder("Contended");
    let cpu = p.pool("cpu", 1);
    let lock = p
        .resource("Lock")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Unit)) })
        .release(|_cx, _u| async move { Ok(()) });
    let mut users = Vec::new();
    for name in ["A", "B", "C"] {
        let d = rng.below(3);
        let l1 = led.clone();
        let l2 = led.clone();
        let k = p
            .resource(name)
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
            });
        users.push(k);
    }
    let (l3, l4) = (led.clone(), led.clone());
    p.service("Svc")
        .needs(users[0])
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
    let (l5, l6) = (led.clone(), led.clone());
    p.blocking_step("Blk")
        .needs(users[1])
        .on(cpu)
        .run(move |cx, _b: Arc<Unit>| {
            l5.blocking_started();
            std::thread::sleep(ms(1));
            if cx.is_stopping() {
                l4.blk_done.fetch_add(1, Ordering::SeqCst);
                return Err("stopping".into());
            }
            l6.blk_done.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });
    let comp = p.component("Net", &net);
    p.step("D")
        .needs((comp, users[2]))
        .run(|_cx, _d: (Arc<u32>, Arc<Unit>)| async move { Ok(()) });
    p.build(policy, Shutdown::within(ms(40)), Mode::Resident)
        .expect("valid")
}

impl Ledger {
    fn blocking_started(&self) {
        self.blk_started.fetch_add(1, Ordering::SeqCst);
    }
}

fn one(tokio_rt: &tokio::runtime::Runtime, seed: u64) -> (String, Vec<String>) {
    let mut rng = SplitMix64::new(seed);
    let led = Arc::new(Ledger::default());
    let plan = contended(&mut rng, led.clone());
    let at = rng.below(14);
    let cancel = rng.chance(0.5);
    let view = plan.inspect();
    let machine = Machine::new(&plan).expect("static");
    let keys = view
        .nodes
        .iter()
        .filter_map(|n| {
            let p = n.path.to_string();
            machine.key_of(&p).map(|k| (p, k))
        })
        .collect();
    let rec = Arc::new(TraceRecorder::new());
    let rt = adapter(tokio_rt, rec.clone());
    let record = Arc::new(Mutex::new(RunRecord::default()));
    let mut report = tokio_rt.block_on(async {
        let running = plan.start_with(rt.clone(), RunOptions::new().record(record.clone()));
        let h = running.handle();
        rt.spawn(Box::pin(async move {
            tokio::time::sleep(ms(at)).await;
            if cancel {
                h.cancel()
            } else {
                h.shutdown()
            }
        }));
        tokio::time::timeout(ms(5000), running).await
    })
    .expect("the run ended within 5 s");
    let left = tokio_rt.block_on(rt.shutdown(ms(2000)));
    let mut problems = Vec::new();
    if left != Ok(()) {
        problems.push(format!("orphans after shutdown: {left:?}"));
    }
    let rej = std::mem::take(&mut record.lock().unwrap().rejections);
    if !rej.is_empty() {
        problems.push(format!("rejections: {rej:?}"));
    }
    let (held, released) = (
        led.held.load(Ordering::SeqCst),
        led.released.load(Ordering::SeqCst),
    );
    if held != released + report.incomplete.len() {
        problems.push(format!(
            "ledger: held {held} released {released} incomplete {:?}",
            report.incomplete.iter().map(|r| r.node.to_string()).collect::<Vec<_>>()
        ));
    }
    let (serving, stopped) = (
        led.serving.load(Ordering::SeqCst),
        led.stopped.load(Ordering::SeqCst),
    );
    if stopped > serving {
        problems.push(format!("service: serving {serving} stopped {stopped}"));
    }
    report.trace = Some(rec.trace());
    let steps = std::mem::take(&mut record.lock().unwrap().steps);
    let mut kinds: Vec<String> = rec
        .trace()
        .events
        .iter()
        .map(|e| match &e.kind {
            TraceKind::Fail(p, l) => format!("Fail({p:?},{l:?})"),
            TraceKind::Skipped { .. } => "Skipped".to_string(),
            k => format!("{k:?}"),
        })
        .collect();
    kinds.sort();
    kinds.dedup();
    let outcome = format!("{:?}", report.outcome);
    KINDS.with(|k| {
        let mut k = k.borrow_mut();
        for kind in kinds {
            match k.iter_mut().find(|(n, _)| *n == kind) {
                Some((_, c)) => *c += 1,
                None => k.push((kind, 1)),
            }
        }
    });
    let driven = Driven::from_recorded(Recorded {
        report,
        view,
        steps,
        whys: Vec::new(),
        rejections: Vec::new(),
        stuck: false,
        keys,
    });
    for v in driven.violations.iter().filter(|v| v.rule != "INV-8") {
        problems.push(format!("{}: {}", v.rule, v.detail));
    }
    (outcome, problems)
}

/// C1: the shapes most likely to race, on two workers, with repetition.
#[test]
fn c1_multi_thread_stress_with_real_bodies() {
    let cases: u64 = std::env::var("PROBE_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(150);
    let tokio_rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_time()
        .build()
        .expect("rt");
    let mut hist: Vec<(String, u64)> = Vec::new();
    let mut bad = Vec::new();
    for i in 0..cases {
        let seed = case_seed(777, i);
        let (outcome, problems) = one(&tokio_rt, seed);
        match hist.iter_mut().find(|(k, _)| *k == outcome) {
            Some((_, n)) => *n += 1,
            None => hist.push((outcome, 1)),
        }
        if !problems.is_empty() {
            bad.push((seed, problems));
        }
    }
    println!("C1 {cases} cases: {hist:?}; failing cases: {}", bad.len());
    KINDS.with(|k| {
        let mut k = k.borrow().clone();
        k.sort();
        println!("C1 cases containing each trace kind: {k:?}");
    });
    for (seed, p) in bad.iter().take(12) {
        println!("  seed {seed}: {p:?}");
    }
    tokio_rt.shutdown_background();
}

/// C2: a `within` deadline and the body's Ok at the same virtual instant.
#[test]
fn c2_within_tie_on_paused_time() {
    let mut outcomes: Vec<(String, u64)> = Vec::new();
    let mut rejections = 0;
    for _ in 0..50 {
        let mut p = Plan::builder("Tie");
        p.step("S").within(ms(1000)).run(|cx, ()| async move {
            cx.sleep(ms(1000)).await;
            Ok(())
        });
        let plan = p
            .build(Policy::Isolate, Shutdown::within(ms(5000)), Mode::Finite)
            .expect("valid");
        let tokio_rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .start_paused(true)
            .build()
            .expect("rt");
        let rt = adapter(&tokio_rt, Arc::new(sdax::host::NoObserver));
        let record = Arc::new(Mutex::new(RunRecord::default()));
        let report = tokio_rt.block_on(async {
            plan.start_with(rt.clone(), RunOptions::new().record(record.clone()))
                .await
        });
        rejections += record.lock().unwrap().rejections.len();
        let key = format!(
            "{:?}/{:?}",
            report.outcome,
            report.faults.iter().map(|f| f.kind.label()).collect::<Vec<_>>()
        );
        match outcomes.iter_mut().find(|(k, _)| *k == key) {
            Some((_, n)) => *n += 1,
            None => outcomes.push((key, 1)),
        }
    }
    println!("C2 within tie: {outcomes:?} rejections={rejections}");
}

/// C3: B1's timing — where do the 335 ms go?
#[test]
fn c3_budget_timing_with_a_stuck_blocking_body() {
    let go = Arc::new(std::sync::atomic::AtomicBool::new(false));
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
    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("rt");
    let rec = Arc::new(TraceRecorder::new());
    let rt = adapter(&tokio_rt, rec.clone());
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
    let wall = t0.elapsed();
    let evs: Vec<String> = rec
        .trace()
        .events
        .iter()
        .map(|e| format!("{:?}@{:?}", e.kind, e.at))
        .collect();
    println!("C3 wall={wall:?} outcome={:?} events={evs:?}", report.outcome);
    go.store(true, Ordering::SeqCst);
    tokio_rt.shutdown_timeout(ms(500));
}
