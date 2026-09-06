//! `S-01` — the research report's spike, on the adapter.
//!
//! A transport with an injected failure, storage, routing that needs both, and
//! a heartbeat service; many runs with random cancellation and random panics.
//! After every run, four things must hold, and each is checked by something
//! that could actually fail:
//!
//! 1. **no orphans** — `TokioRuntime::tracked()` is zero, and the invariant
//!    checker's `orphans: none` rule agrees from the trace;
//! 2. **no held slot without a release** — the bodies count their own holds and
//!    discharges, and INV-3/INV-4 check the same thing from the trace;
//! 3. **every fault in the report** — an injected failure or panic that was
//!    reached appears in `faults`, and INV-9 checks the general case;
//! 4. **bounded shutdown** — INV-8, exact because the clock is paused.
//!
//! The bodies here are the plan's own: nothing is scripted, so this is the
//! whole path — `Deps::fetch`, `cx.hold`, `Serving`, the release graph.

use sdax::host::engine::Machine;
use sdax::host::Observer;
use sdax::*;
use sdax_testkit::mc::prng::{case_seed, SplitMix64};
use sdax_testkit::{quiet_scripted_panics, Driven, Recorded, TraceRecorder};
use sdax_tokio::{PlanStart, RunOptions, RunRecord, TokioRuntime};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const SCRIPTED: &str = "scripted panic";
const DEFAULT_CASES: u64 = 300;
const SUITE_SEED: u64 = 20_260_906;

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

/// What the bodies of one run did, counted by the bodies themselves.
#[derive(Default)]
struct Ledger {
    held: AtomicUsize,
    discharged: AtomicUsize,
    serving: AtomicUsize,
    stopped: AtomicUsize,
    /// The transport body reached its injected failure and returned `Err`.
    failed: AtomicUsize,
}

/// How one case's bodies are told to misbehave.
#[derive(Debug, Clone, Copy)]
struct Injected {
    transport_fails: bool,
    routing_panics: bool,
    storage_release_fails: bool,
    transport_delay: u64,
    storage_delay: u64,
    routing_delay: u64,
}

struct Handle;
struct Conn;
struct Store;

/// The mesh: transport, storage, routing over both, and a heartbeat.
fn mesh(inj: Injected, led: Arc<Ledger>) -> Plan {
    let mut p = Plan::builder("Mesh");
    let (l1, l2, l3, l4, l5) = (
        led.clone(),
        led.clone(),
        led.clone(),
        led.clone(),
        led.clone(),
    );
    let transport = p
        .resource("Transport")
        .acquire(move |cx, ()| {
            let l = l1.clone();
            async move {
                cx.sleep(secs(inj.transport_delay)).await;
                if inj.transport_fails {
                    l.failed.fetch_add(1, Ordering::SeqCst);
                    return Err("the transport could not be opened".into());
                }
                l.held.fetch_add(1, Ordering::SeqCst);
                Ok(cx.hold_value(Conn))
            }
        })
        .release(move |_cx, _c| {
            let l = l2.clone();
            async move {
                l.discharged.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        });
    let storage = p
        .resource("Storage")
        .acquire(move |cx, ()| {
            let l = l3.clone();
            async move {
                cx.sleep(secs(inj.storage_delay)).await;
                l.held.fetch_add(1, Ordering::SeqCst);
                Ok(cx.hold_value(Store))
            }
        })
        .release(move |_cx, _s| {
            let l = l4.clone();
            async move {
                l.discharged.fetch_add(1, Ordering::SeqCst);
                if inj.storage_release_fails {
                    return Err("the storage would not close".into());
                }
                Ok(())
            }
        });
    let routing = p.step("Routing").needs((transport, storage)).run(
        move |cx, _d: (Arc<Conn>, Arc<Store>)| async move {
            cx.sleep(secs(inj.routing_delay)).await;
            if inj.routing_panics {
                panic!("{SCRIPTED}");
            }
            Ok(())
        },
    );
    p.service("Heartbeat")
        .needs(routing)
        .stop_within(secs(2))
        .start(move |cx, _r: Arc<()>| {
            let l = l5.clone();
            async move {
                l.serving.fetch_add(1, Ordering::SeqCst);
                Ok(Serving::new(Handle, async move {
                    cx.stop().await;
                    l.stopped.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }))
            }
        });
    p.build(Policy::FailFast, Shutdown::within(secs(5)), Mode::Resident)
        .expect("valid")
}

/// An observer that keeps the trace *and* a copy of the report.
///
/// A dropped `Running` has nobody awaiting its report, so the observer is the
/// only place it appears — and a `Report` is not `Clone`, because a
/// `FaultKind` carries a boxed error or panic payload. This rebuilds one whose
/// node, order, phase and fault *label* are the original's, which is what the
/// invariant checker reads; the payloads themselves are re-boxed and are not
/// claimed to be the originals.
#[derive(Default)]
struct Captured {
    trace: TraceRecorder,
    report: Mutex<Option<Report<()>>>,
}

#[derive(Debug)]
struct Rebuilt(String);
impl std::fmt::Display for Rebuilt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Rebuilt {}

fn rebuild(kind: &FaultKind) -> FaultKind {
    match kind {
        FaultKind::Error(e) => FaultKind::Error(Box::new(Rebuilt(e.to_string()))),
        FaultKind::Panic(_) => FaultKind::Panic(Box::new(SCRIPTED)),
        FaultKind::Timeout => FaultKind::Timeout,
        FaultKind::NeverReady => FaultKind::NeverReady,
        FaultKind::DoubleHold => FaultKind::DoubleHold,
    }
}

fn rebuild_all(v: &[Fault]) -> Vec<Fault> {
    v.iter()
        .map(|f| Fault {
            node: f.node.clone(),
            order: f.order.clone(),
            phase: f.phase,
            kind: rebuild(&f.kind),
        })
        .collect()
}

impl Observer for Captured {
    fn event(&self, e: &TraceEvent) {
        self.trace.event(e);
    }

    fn report(&self, r: &Report<()>) {
        self.trace.report(r);
        *self.report.lock().expect("captured poisoned") = Some(Report {
            outcome: r.outcome,
            output: None,
            faults: rebuild_all(&r.faults),
            cleanup_failures: rebuild_all(&r.cleanup_failures),
            incomplete: r.incomplete.clone(),
            ambiguous: r.ambiguous.clone(),
            trace: None,
        });
    }
}

/// What the case's random request is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ask {
    Shutdown(u64),
    Cancel(u64),
    Drop(u64),
}

fn one_case(seed: u64) -> String {
    let mut rng = SplitMix64::new(seed);
    let inj = Injected {
        transport_fails: rng.chance(0.2),
        routing_panics: rng.chance(0.2),
        storage_release_fails: rng.chance(0.15),
        transport_delay: rng.below(3),
        storage_delay: rng.below(3),
        routing_delay: rng.below(3),
    };
    let at = rng.below(7);
    let ask = match rng.below(3) {
        0 => Ask::Shutdown(at),
        1 => Ask::Cancel(at),
        _ => Ask::Drop(at),
    };
    let led = Arc::new(Ledger::default());
    let plan = mesh(inj, led.clone());
    let view = plan.inspect();
    let machine = Machine::new(&plan).expect("a static plan");
    let keys = view
        .nodes
        .iter()
        .filter_map(|n| {
            let p = n.path.to_string();
            machine.key_of(&p).map(|k| (p, k))
        })
        .collect();

    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .expect("runtime");
    let rec = Arc::new(Captured::default());
    let obs: Arc<dyn Observer> = rec.clone();
    let rt = Arc::new(
        TokioRuntime::current_thread_no_background_drain(tokio_rt.handle().clone())
            .with_observer(obs),
    );
    let record = Arc::new(Mutex::new(RunRecord::default()));
    let report = tokio_rt.block_on(async {
        let mut running = plan.start_with(rt.clone(), (), RunOptions::new().record(record.clone()));
        let handle = running.handle();
        match ask {
            Ask::Drop(at) => {
                drop(handle);
                // A `Running` nobody has polled has started nothing, so there
                // would be nothing to drain: launch it, let it get somewhere,
                // and then drop it live.
                let _ = running.ready().await;
                tokio::time::sleep(secs(at)).await;
                // The drop guard: nobody awaits the report, so the drainer
                // finishes the release graph and hands it to the observer.
                drop(running);
                tokio::time::sleep(secs(120)).await;
                None
            }
            Ask::Shutdown(at) | Ask::Cancel(at) => {
                // The request is made from this future rather than a task of
                // its own: a spawned one would still be sleeping when the run
                // ends early, and `tracked()` — the orphan check below — would
                // be counting the harness instead of the engine.
                match tokio::time::timeout(secs(at), &mut running).await {
                    Ok(report) => Some(Ok(report)),
                    Err(_) => {
                        match ask {
                            Ask::Shutdown(_) => handle.shutdown(),
                            _ => handle.cancel(),
                        }
                        Some(tokio::time::timeout(secs(3_600), &mut running).await)
                    }
                }
            }
        }
    });

    let where_ = format!("seed {seed}, {inj:?}, {ask:?}");
    // 1. no orphans, from the substrate itself.
    assert_eq!(rt.tracked(), 0, "{where_}: a task outlived the run");
    let summary = rec.trace.reports();
    assert_eq!(summary.len(), 1, "{where_}: exactly one report");
    let summary = summary[0];

    let report = match report {
        Some(Ok(r)) => r,
        Some(Err(_)) => panic!("{where_}: the run never reached End"),
        // Dropped: the report went to the observer, and the trace with it.
        None => {
            let mut r = rec
                .report
                .lock()
                .expect("captured poisoned")
                .take()
                .expect("the drainer reported");
            r.trace = Some(rec.trace.trace());
            r
        }
    };
    let rejections = {
        let mut g = record.lock().expect("record poisoned");
        std::mem::take(&mut g.rejections)
    };
    assert!(rejections.is_empty(), "{where_}: {rejections:?}");

    // 2. no held slot without a release, counted by the bodies.
    let held = led.held.load(Ordering::SeqCst);
    let discharged = led.discharged.load(Ordering::SeqCst);
    assert_eq!(
        held,
        discharged + summary.incomplete,
        "{where_}: {held} held, {discharged} discharged, {} abandoned",
        summary.incomplete
    );
    assert_eq!(
        led.stopped.load(Ordering::SeqCst),
        led.serving.load(Ordering::SeqCst),
        "{where_}: a service started serving and was never stopped"
    );

    // 3. every fault reached is in the report. The body itself says whether
    //    it got as far as failing, so this is a body-side fact checked against
    //    a report-side one, not a restatement of the trace.
    if led.failed.load(Ordering::SeqCst) > 0 {
        let named = report
            .faults
            .iter()
            .any(|f| f.node.to_string() == "Transport")
            || report
                .incomplete
                .iter()
                .any(|r| r.node.to_string() == "Transport");
        assert!(
            named,
            "{where_}: the transport body returned Err and the report does not say so"
        );
    }

    // 4. every invariant, including the bounded shutdown (INV-8, exact under
    //    paused time) and `orphans: none` from the trace.
    let rec2 = {
        let mut g = record.lock().expect("record poisoned");
        RunRecord {
            steps: std::mem::take(&mut g.steps),
            whys: std::mem::take(&mut g.whys),
            rejections: Vec::new(),
        }
    };
    let driven = Driven::from_recorded(Recorded {
        spawns: Vec::new(),
        report,
        view,
        steps: rec2.steps,
        whys: rec2.whys,
        rejections: Vec::new(),
        stuck: false,
        clock_slack: std::time::Duration::ZERO,
        keys,
    });
    if let Some(p) = driven.problems() {
        panic!("{where_}\n{p}");
    }
    format!("{:?}", summary.outcome)
}

fn walk(cases: u64, suite: u64) {
    quiet_scripted_panics();
    let mut histogram: Vec<(String, u64)> = Vec::new();
    for i in 0..cases {
        let outcome = one_case(case_seed(suite, i));
        match histogram.iter_mut().find(|(k, _)| *k == outcome) {
            Some((_, n)) => *n += 1,
            None => histogram.push((outcome, 1)),
        }
    }
    histogram.sort();
    println!("S-01 over {cases} cases: {histogram:?}");
    for corner in ["Ok", "Failed", "Cancelled"] {
        let n = histogram
            .iter()
            .find(|(k, _)| k == corner)
            .map(|(_, n)| *n)
            .unwrap_or(0);
        assert!(n >= 5, "S-01 reached {corner} only {n} times in {cases}");
    }
}

/// `S-01`: the spike, walked with random cancellation, drops and panics.
#[test]
fn s01_the_spike_holds_under_random_cancellation_and_panics() {
    let cases = std::env::var("SDAX_S01_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_CASES);
    let suite = std::env::var("SDAX_S01_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(SUITE_SEED);
    walk(cases, suite);
}
