//! `R-05`: the same programs on a genuinely parallel runtime.
//!
//! `start_paused` is a current-thread facility, so a multi-threaded run cannot
//! have a virtual clock. What it can have is a **compressed** one: the engine
//! only ever reads time through [`Clock`], so one engine second is served as
//! `1/factor` of a real second and a ten-second shutdown budget costs
//! milliseconds. Everything the engine measures — backoff, `within`,
//! `stop_within`, the budget — is on that clock and stays proportionate.
//!
//! **Trace equality is not asserted here, and that is deliberate.** With two
//! workers the unordered pairs the contract leaves free really do interleave
//! differently from run to run. What must hold is every invariant, the report
//! being clean or unclean as the program says, and no orphan — which is
//! measured on the substrate by [`TokioRuntime::shutdown`], not read off the
//! trace, because a trace cannot see a detached thread.
//!
//! **INV-8 and the clock.** A real timer fires at *or after* its deadline,
//! never before, so the engine's own measurement of "budget to `End`" always
//! overshoots by whatever the scheduler added — and the compression multiplies
//! that jitter by the factor. Measured on this substrate, a bare 20 ms
//! `tokio::time::sleep` on two workers returns 1–11 ms late, so at ×200 the
//! slop alone is 0.2–2.2 engine seconds *per timer* and no assertion about a
//! 3-second budget can survive it. INV-8 is therefore **absent** from the
//! ×`FACTOR` pass — not tolerated, absent — and asserted instead at
//! ×[`INV8_FACTOR`] with [`INV8_SLACK`] of engine time, where the same slop is
//! a tenth of the allowance. Exactly, it is asserted under `start_paused` in
//! `R-01` and by `C-18`, `C-56` and `C-69` on the scripted driver.

use crate::corpus::*;
use sdax::host::engine::Machine;
use sdax::host::{BoxFuture, Clock, Runtime, Time};
use sdax::*;
use sdax_testkit::{quiet_scripted_panics, Driven, Recorded, ScriptedBodies};
use sdax_tokio::{PlanStart, RunOptions, RunRecord, TokioRuntime};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

/// One engine second per `1/FACTOR` real seconds, for the pass that is about
/// interleaving rather than duration.
const FACTOR: u32 = 200;

/// The compression at which INV-8 carries an assertion.
///
/// Ten times slower than [`FACTOR`], so the substrate's 1–11 ms of timer slop
/// is 0.02–0.22 engine seconds instead of 0.2–2.2, and [`INV8_SLACK`] is an
/// allowance rather than a licence.
const INV8_FACTOR: u32 = 20;

/// What INV-8's bound forgives at [`INV8_FACTOR`]: half an engine second,
/// which is 25 ms of real time — a little over two of the worst per-timer
/// slops measured here (11 ms), and a sixth of the smallest budget in play.
///
/// Measured, not guessed: with the allowance set to zero the same run reports
/// 40 ms of engine overshoot on a 3-second budget, so this is ×12 the observed
/// error and a driver that waited even 3.5 engine seconds would fail.
const INV8_SLACK: Duration = Duration::from_millis(500);

/// How many times each program is run.
///
/// One execution of a race witness is not a witness: the one multi-thread
/// defect Stage 2 found (`OD-SERVE-ARM`) appeared about one run in six.
/// `SDAX_R05_REPEATS` overrides it, which is how a longer soak is run without
/// a second test.
const REPEATS: usize = 3;

/// Real time a program is allowed before it counts as stuck. Nothing here
/// takes a tenth of it; it exists so a multi-thread hang is a failed
/// assertion and not a stalled suite.
const LIVENESS: Duration = Duration::from_secs(30);

/// A real clock, compressed.
struct ScaledClock {
    origin: OnceLock<tokio::time::Instant>,
    factor: u32,
}

impl ScaledClock {
    fn new(factor: u32) -> Self {
        ScaledClock {
            origin: OnceLock::new(),
            factor,
        }
    }

    fn origin(&self) -> tokio::time::Instant {
        *self.origin.get_or_init(tokio::time::Instant::now)
    }
}

impl Clock for ScaledClock {
    fn now(&self) -> Time {
        let real = tokio::time::Instant::now().duration_since(self.origin());
        Time::from_nanos((real.as_nanos() as u64).saturating_mul(self.factor as u64))
    }

    fn sleep(&self, d: Duration) -> BoxFuture<'static, ()> {
        Box::pin(tokio::time::sleep(d / self.factor))
    }
}

fn repeats() -> usize {
    std::env::var("SDAX_R05_REPEATS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(REPEATS)
}

/// Run one case on two worker threads, [`repeats`] times, and check every
/// invariant but INV-8 — see the module docs.
fn parallel<Out: Send + Sync + 'static>(what: &str, plan: &Plan<Out>, script: &Script) {
    for i in 0..repeats() {
        once(&format!("{what} #{i}"), plan, script, FACTOR, None);
    }
}

/// Run one case at [`INV8_FACTOR`] and check **every** invariant, INV-8
/// included, with [`INV8_SLACK`].
fn bounded<Out: Send + Sync + 'static>(what: &str, plan: &Plan<Out>, script: &Script) {
    for i in 0..repeats() {
        once(
            &format!("{what} #{i}"),
            plan,
            script,
            INV8_FACTOR,
            Some(INV8_SLACK),
        );
    }
}

/// One execution: run it, then check the trace, the report, the rejections and
/// the substrate.
fn once<Out: Send + Sync + 'static>(
    what: &str,
    plan: &Plan<Out>,
    script: &Script,
    factor: u32,
    slack: Option<Duration>,
) {
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
        TokioRuntime::new(tokio_rt.handle().clone()).with_clock(Arc::new(ScaledClock::new(factor))),
    );
    let record = Arc::new(Mutex::new(RunRecord::default()));
    let opts = RunOptions::new()
        .bodies(src)
        .record(record.clone())
        .schedule(script.schedule_ref().clone());
    let requests: Vec<(Duration, Request)> = script.requests().to_vec();
    let mine = rt.clone();
    let out = tokio_rt.block_on(async move {
        let running = plan.start_with(rt.clone(), opts);
        let handle = running.handle();
        for (at, req) in requests {
            if at.is_zero() {
                ask(&handle, req);
                continue;
            }
            let h = handle.clone();
            rt.spawn(Box::pin(async move {
                tokio::time::sleep(at / factor).await;
                ask(&h, req);
            }));
        }
        tokio::time::timeout(LIVENESS, running).await
    });
    let (report, stuck) = match out {
        Ok(r) => (r, false),
        Err(_) => (Report::empty(Outcome::Cancelled), true),
    };
    // INV-15 on the substrate: every task and every pool thread this run
    // spawned has finished. A trace cannot see a detached one, so this is the
    // only place the claim is actually checked on two workers.
    let orphans = tokio_rt.block_on(mine.shutdown(Duration::from_secs(5)));
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
        stuck,
        clock_slack: slack.unwrap_or_default(),
        keys,
    });
    let real: Vec<_> = driven
        .violations
        .iter()
        .filter(|v| slack.is_some() || v.rule != "INV-8")
        .collect();
    if !real.is_empty() || !driven.rejections.is_empty() || driven.stuck {
        panic!(
            "{what}: {}",
            driven.problems().unwrap_or_else(|| "?".to_string())
        );
    }
    assert_eq!(
        orphans,
        Ok(()),
        "{what}: INV-15, an orphan on the substrate"
    );
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
/// invariants are. Each runs [`REPEATS`] times, because the interleavings
/// differ run to run and one execution of a race witness is not a witness.
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

/// `R-05` — S-11: INV-8's bound, asserted on two worker threads.
///
/// The two programs whose settling actually costs something: one whose budget
/// **expires** with a cleanup that ignores the stop (`i16`), and one that ends
/// on a fault under `Isolate` (`i08`). At ×[`INV8_FACTOR`] the substrate's own
/// timer slop is a fifth of [`INV8_SLACK`], so what this asserts is the
/// engine's waiting and not the scheduler's punctuality.
#[test]
fn r05_the_shutdown_bound_holds_on_two_worker_threads_with_measured_slack() {
    bounded(
        "i16 budget expiry",
        &i16(Mode::Resident, Shutdown::within(Duration::from_secs(3))),
        &Script::new()
            .cleanup("PeerStore", Cleanup::IgnoreStop)
            .at(2.0, Request::Shutdown),
    );
    bounded(
        "i08 isolate",
        &i08(Policy::Isolate),
        &Script::new().prepare("B", Body::fail(At::plus(1.0), "boom")),
    );
}
