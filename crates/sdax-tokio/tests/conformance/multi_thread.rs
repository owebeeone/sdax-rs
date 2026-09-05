//! `R-05`: the same programs on a genuinely parallel runtime.
//!
//! `start_paused` is a current-thread facility, so a multi-threaded run cannot
//! have a virtual clock. What it can have is a **compressed** one: the engine
//! only ever reads time through [`Clock`], so one engine second is served as
//! `1/FACTOR` of a real second and a ten-second shutdown budget costs
//! milliseconds. Everything the engine measures — backoff, `within`,
//! `stop_within`, the budget — is on that clock and stays proportionate.
//!
//! **Trace equality is not asserted here, and that is deliberate.** With two
//! workers the unordered pairs the contract leaves free really do interleave
//! differently from run to run. What must hold is every invariant, the report
//! being clean or unclean as the program says, and no orphan — so that is what
//! is checked.
//!
//! **INV-8 is the one invariant not asserted here, and the reason is the
//! clock, not the engine.** A real timer fires at *or after* its deadline,
//! never before, so the engine's own measurement of "budget to End" always
//! overshoots by whatever the scheduler added — and the compression multiplies
//! that jitter by `FACTOR`, so a 7 ms hiccup reads as 1.5 engine seconds. The
//! bound is exact only on an exact clock, and that is where it is asserted:
//! under `start_paused` in `R-01`, and by `C-18`, `C-56` and `C-69` on the
//! scripted driver. Everything else the checker knows still runs here.

use crate::corpus::*;
use sdax::host::engine::Machine;
use sdax::host::{BoxFuture, Clock, Runtime, Time};
use sdax::*;
use sdax_testkit::{quiet_scripted_panics, Driven, Recorded, ScriptedBodies};
use sdax_tokio::{PlanStart, RunOptions, RunRecord, TokioRuntime};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

/// One engine second per `1/FACTOR` real seconds.
const FACTOR: u32 = 200;

/// A real clock, compressed.
#[derive(Default)]
struct ScaledClock {
    origin: OnceLock<tokio::time::Instant>,
}

impl ScaledClock {
    fn origin(&self) -> tokio::time::Instant {
        *self.origin.get_or_init(tokio::time::Instant::now)
    }
}

impl Clock for ScaledClock {
    fn now(&self) -> Time {
        let real = tokio::time::Instant::now().duration_since(self.origin());
        Time::from_nanos((real.as_nanos() as u64).saturating_mul(FACTOR as u64))
    }

    fn sleep(&self, d: Duration) -> BoxFuture<'static, ()> {
        Box::pin(tokio::time::sleep(d / FACTOR))
    }
}

/// Run one case on two worker threads and check every invariant.
fn parallel<Out: Send + Sync + 'static>(what: &str, plan: &Plan<Out>, script: &Script) {
    quiet_scripted_panics();
    let machine = Machine::new(plan).expect("the machine accepts the plan");
    let view = plan.inspect();
    let keys = view
        .nodes
        .iter()
        .filter_map(|n| {
            let p = n.path.to_string();
            machine.key_of(&p).map(|k| (p, k))
        })
        .collect();
    let src = ScriptedBodies::new(plan, script).expect("the script fits the plan");
    let tokio_rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_time()
        .build()
        .expect("runtime");
    let rt = Arc::new(
        TokioRuntime::new(tokio_rt.handle().clone()).with_clock(Arc::new(ScaledClock::default())),
    );
    let record = Arc::new(Mutex::new(RunRecord::default()));
    let opts = RunOptions::new()
        .bodies(src)
        .record(record.clone())
        .schedule(script.schedule_ref().clone());
    let requests: Vec<(Duration, Request)> = script.requests().to_vec();
    let report = tokio_rt.block_on(async move {
        let running = plan.start_with(rt.clone(), opts);
        let handle = running.handle();
        for (at, req) in requests {
            if at.is_zero() {
                ask(&handle, req);
                continue;
            }
            let h = handle.clone();
            rt.spawn(Box::pin(async move {
                tokio::time::sleep(at / FACTOR).await;
                ask(&h, req);
            }));
        }
        running.await
    });
    let rec = {
        let mut r = record.lock().expect("record poisoned");
        RunRecord {
            steps: std::mem::take(&mut r.steps),
            whys: std::mem::take(&mut r.whys),
            rejections: std::mem::take(&mut r.rejections),
        }
    };
    let driven = Driven::from_recorded(Recorded {
        spawns: Vec::new(),
        report,
        view,
        steps: rec.steps,
        whys: rec.whys,
        rejections: rec.rejections,
        stuck: false,
        keys,
    });
    let real: Vec<_> = driven
        .violations
        .iter()
        .filter(|v| v.rule != "INV-8")
        .collect();
    if !real.is_empty() || !driven.rejections.is_empty() {
        panic!(
            "{what}: {}",
            driven.problems().unwrap_or_else(|| "?".to_string())
        );
    }
}

fn ask(h: &sdax_tokio::RunHandle, req: Request) {
    match req {
        Request::Shutdown => h.shutdown(),
        Request::Cancel => h.cancel(),
    }
}

fn every_body(plan: &Plan, secs: f64) -> Script {
    let mut s = Script::new();
    for n in &plan.inspect().nodes {
        s = s.prepare(&n.path.to_string(), Body::ok(At::plus(secs)));
    }
    s
}

/// `R-05`: the whole shape of suite (c) — startup, faults, retries, services,
/// components, budgets, locks and pools — on `new_multi_thread(2)`.
///
/// The same eight programs the differential check uses, chosen so that every
/// mechanism appears at least once; the outcomes are not asserted, the
/// invariants are.
#[test]
fn r05_the_invariants_hold_on_two_worker_threads() {
    parallel(
        "i01",
        &i01(Mode::Finite),
        &every_body(&i01(Mode::Finite), 1.0),
    );
    parallel("i02", &i02(), &every_body(&i02(), 0.5));
    parallel(
        "i05",
        &i05(),
        &every_body(&i05(), 1.0).at(5.0, Request::Shutdown),
    );
    parallel(
        "i07 restart",
        &i07(true),
        &Script::new()
            .serve(
                "Exporter",
                [Serve::Err(At::tick(2.0), "partition".to_string())],
            )
            .at(9.0, Request::Shutdown),
    );
    parallel(
        "i08 isolate",
        &i08(Policy::Isolate),
        &Script::new().prepare("B", Body::fail(At::plus(1.0), "boom")),
    );
    parallel(
        "i16 budget expiry",
        &i16(Mode::Resident, Shutdown::within(Duration::from_secs(3))),
        &Script::new()
            .cleanup("PeerStore", Cleanup::IgnoreStop)
            .at(2.0, Request::Shutdown),
    );
    parallel(
        "i27 locks",
        &i27(true),
        &every_body(&i27(true), 1.0).at(9.0, Request::Shutdown),
    );
    parallel(
        "i32 component",
        &i32(),
        &every_body(&i32(), 1.0).at(8.0, Request::Shutdown),
    );
}
