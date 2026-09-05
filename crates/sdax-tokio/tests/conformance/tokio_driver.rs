//! `R-01`: suite (c), re-run against the tokio adapter.
//!
//! Same plans, same scripts, same expectations, same invariant checker — only
//! the thing performing the effects changes. The bodies come from
//! [`ScriptedBodies`], so the script still decides every outcome; the timing,
//! the tasks, the timers, the aborts and the joins are real.
//!
//! Time is paused (`start_paused(true)`), so a run costs no wall clock and the
//! engine clock is exact (LBT-008).

use sdax::host::sim::ScriptError;
use sdax::host::{engine::Machine, Runtime};
use sdax::{Plan, Report, Request, Script};
use sdax_testkit::{quiet_scripted_panics, Driven, Recorded, ScriptedBodies};
use sdax_tokio::{PlanStart, RunOptions, RunRecord, TokioRuntime};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// A virtual hour. With paused time this only elapses when the run has nothing
/// left to do, so it is a liveness guard rather than a deadline: a run that
/// cannot terminate is reported instead of hanging the suite.
const LIVENESS: Duration = Duration::from_secs(3_600);

/// Suite (c) on the run driver.
pub struct TokioDriver;

impl TokioDriver {
    /// Run `plan` under `script` on a paused tokio runtime and check it the
    /// same way [`sdax_testkit::ScriptedDriver`] checks its own runs.
    pub fn run<Out: Send + Sync + 'static>(
        plan: &Plan<Out>,
        script: &Script,
    ) -> Result<Driven<Out>, ScriptError> {
        quiet_scripted_panics();
        let machine = Machine::new(plan).map_err(ScriptError::Engine)?;
        // A template's inner nodes are declared but exist only per instance,
        // so the script is checked against the *declarations*, exactly as the
        // stepping simulator checks it.
        let declared: Vec<String> = Machine::declarations(plan)
            .iter()
            .map(|(_, p, _)| p.to_string())
            .collect();
        for name in script.named_nodes() {
            if !declared.iter().any(|p| p == name) {
                return Err(ScriptError::UnknownNode(name.to_string()));
            }
        }
        let view = plan.inspect();
        let keys = view
            .nodes
            .iter()
            .filter_map(|n| {
                let p = n.path.to_string();
                machine.key_of(&p).map(|k| (p, k))
            })
            .collect();
        let src = ScriptedBodies::new(plan, script).map_err(ScriptError::Engine)?;
        let bodies = src.clone();
        let tokio_rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .start_paused(true)
            .build()
            .expect("runtime");
        let rt = Arc::new(TokioRuntime::new(tokio_rt.handle().clone()));
        let record = Arc::new(Mutex::new(RunRecord::default()));
        let opts = RunOptions::new()
            .bodies(src)
            .record(record.clone())
            .schedule(script.schedule_ref().clone())
            .observe_states();
        let requests: Vec<(Duration, Request)> = script.requests().to_vec();
        let out = tokio_rt.block_on(async move {
            let running = plan.start_with(rt.clone(), opts);
            let handle = running.handle();
            for (at, req) in requests {
                if at.is_zero() {
                    // A request at the origin precedes the first poll, so
                    // `@0 cancel` is a cancel before anything started (C-11).
                    ask(&handle, req);
                    continue;
                }
                let h = handle.clone();
                rt.spawn(Box::pin(async move {
                    tokio::time::sleep(at).await;
                    ask(&h, req);
                }));
            }
            tokio::time::timeout(LIVENESS, running).await
        });
        let rec = {
            let mut r = record.lock().expect("record poisoned");
            RunRecord {
                steps: std::mem::take(&mut r.steps),
                whys: std::mem::take(&mut r.whys),
                rejections: std::mem::take(&mut r.rejections),
            }
        };
        let (report, stuck) = match out {
            Ok(r) => (r, false),
            Err(_) => (Report::empty(sdax::Outcome::Cancelled), true),
        };
        Ok(Driven::from_recorded(Recorded {
            report,
            view,
            steps: rec.steps,
            whys: rec.whys,
            rejections: rec.rejections,
            spawns: bodies.spawns(),
            stuck,
            keys,
        }))
    }
}

fn ask(h: &sdax_tokio::RunHandle, req: Request) {
    match req {
        Request::Shutdown => h.shutdown(),
        Request::Cancel => h.cancel(),
    }
}
