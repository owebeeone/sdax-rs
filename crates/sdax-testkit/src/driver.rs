//! The scripted driver: the core's [`Simulator`] with recording and checking
//! around it. It is the same code path as `Plan::simulate` — not a second
//! driver — plus, after every step, the independent trace checker.

use crate::eol::Eol;
use crate::invariants::{
    check_trace, check_trace_prefix, check_trace_with_slack, check_whys, Violation,
};
use sdax::host::sim::{ScriptError, SimStep, Simulator, SpawnOutcome};
use sdax::host::{RawKey, Time};
use sdax::{Plan, PlanView, Reason, Report, Script, Trace};
use std::time::Duration;

/// One `why` answer, recorded at a time: when, which node, and what it was
/// waiting for.
pub type WhyAt = (Time, String, Vec<(String, Reason)>);

/// The most steps a run may take before the driver calls it non-terminating.
const LIVENESS_STEPS: usize = 1_000;

/// Runs a plan under a script on the pure machine.
pub struct ScriptedDriver;

/// One finished (or stuck) scripted run, with everything observed.
pub struct Driven<Out = ()> {
    /// The report, in F4 order, with the trace attached.
    pub report: Report<Out>,
    /// The trace, in observation order.
    pub trace: Trace,
    /// The plan as the checker saw it.
    pub view: PlanView,
    /// Every event fed and every effect returned, rendered.
    pub steps: Vec<SimStep>,
    /// Everything the checker found, after any step or at the end.
    pub violations: Vec<Violation>,
    /// Every event the machine refused.
    pub rejections: Vec<String>,
    /// Every `cx.spawn` a body made and what it answered, in order.
    pub spawns: Vec<SpawnOutcome>,
    /// The run stopped short of `End` with nothing left to deliver.
    pub stuck: bool,
    /// Where the first violation was seen: how many steps had been fed, and
    /// how long the trace was. A failure print truncates there rather than
    /// dumping the whole run.
    pub first_violation: Option<(usize, usize)>,
    /// `why` answers recorded after every step, for `why_at`.
    whys: Vec<WhyAt>,
    keys: Vec<(String, RawKey)>,
}

impl ScriptedDriver {
    /// Run to the end (or until stuck), checking the trace after every step.
    ///
    /// Supplies no per-run input, so a plan that declares one is refused —
    /// [`run_with_input`](Self::run_with_input) is the entry point for those.
    pub fn run<Out, In>(plan: &Plan<Out, In>, script: &Script) -> Result<Driven<Out>, ScriptError> {
        Ok(ScriptedDriver::drive(
            Simulator::new(plan, script)?,
            plan.inspect(),
        ))
    }

    /// [`run`](Self::run) for a plan started with a per-run input.
    ///
    /// The value is dropped: a scripted body reads no slot. What it proves is
    /// the shape — the input node is not a node of the run, and a node that
    /// `needs` it is satisfied from the first step.
    pub fn run_with_input<Out, In>(
        plan: &Plan<Out, In>,
        input: In,
        script: &Script,
    ) -> Result<Driven<Out>, ScriptError> {
        drop(input);
        Ok(ScriptedDriver::drive(
            Simulator::with_input(plan, script)?,
            plan.inspect(),
        ))
    }

    fn drive<Out>(mut sim: Simulator, view: PlanView) -> Driven<Out> {
        let mut violations = Vec::new();
        let mut whys = Vec::new();
        let mut first_violation = None;
        while sim.step().is_some() {
            // Checked before the prefix check, which is quadratic in the
            // trace: a machine that will not settle must be reported, not
            // ground over. No honest run comes near this many steps.
            if sim.steps().len() > LIVENESS_STEPS {
                violations.push(Violation {
                    rule: "LIVENESS",
                    detail: format!("more than {LIVENESS_STEPS} steps without End"),
                });
                break;
            }
            let prefix = check_trace_prefix(sim.trace(), &view);
            for v in prefix {
                if !violations.contains(&v) {
                    violations.push(v);
                }
            }
            if first_violation.is_none() && !violations.is_empty() {
                first_violation = Some((sim.steps().len(), sim.trace().events.len()));
            }
            let now = sim.now();
            let m = sim.machine();
            for n in &view.nodes {
                let path = n.path.to_string();
                if let Some(sdax::host::engine::NodeState::Waiting { on }) = m.state_of(&path) {
                    let on: Vec<(String, Reason)> =
                        on.into_iter().map(|(p, r)| (p.to_string(), r)).collect();
                    whys.push((now, path, on));
                }
            }
        }
        let stuck = sim.stuck();
        let rejections = sim.rejections().to_vec();
        let spawns = sim.spawns().to_vec();
        let trace = sim.trace().clone();
        let mut report: Report<Out> = match sim.take_report() {
            Some(r) => r,
            None => {
                let mut r = Report::empty(sdax::Outcome::Cancelled);
                r.trace = Some(trace.clone());
                r
            }
        };
        if !stuck && report.trace.is_none() {
            report.trace = Some(trace.clone());
        }
        for v in check_whys(&whys) {
            if !violations.contains(&v) {
                violations.push(v);
            }
        }
        if !stuck {
            for v in check_trace(&trace, &view, &report) {
                if !violations.contains(&v) {
                    violations.push(v);
                }
            }
            // D1: whatever the script still had queued after End is refused,
            // never acted on.
            for fx in sim.drain_after_end() {
                let refused = fx
                    .iter()
                    .all(|e| matches!(e, sdax::host::engine::Effect::Reject(_)));
                if !refused {
                    violations.push(Violation {
                        rule: "D1",
                        detail: format!("an event after End was acted on: {fx:?}"),
                    });
                }
            }
        }
        // Every node of the run, instances included: a template's inner node
        // has one key per instance, and a harness that reads an effect back
        // has to be able to name all of them.
        let keys = sim
            .machine()
            .nodes()
            .into_iter()
            .map(|(k, p, _)| (p.to_string(), k))
            .collect();
        Driven {
            report,
            trace,
            view,
            steps: sim.steps().to_vec(),
            violations,
            rejections,
            spawns,
            stuck,
            first_violation,
            whys,
            keys,
        }
    }
}

/// What a driver other than [`ScriptedDriver`] recorded of one run.
///
/// The fields the scripted driver fills from its simulator, named so that
/// another driver — the tokio run driver in `sdax-tokio` — can hand the same
/// material to the same checker. That is what makes suite (c) re-runnable
/// against a real runtime without a second copy of it (LBT-009).
pub struct Recorded<Out> {
    /// The report, with the trace attached.
    pub report: Report<Out>,
    /// The plan as the checker should see it.
    pub view: PlanView,
    /// Every event fed and every effect returned, rendered.
    pub steps: Vec<SimStep>,
    /// A `why` answer per waiting node after every step.
    pub whys: Vec<WhyAt>,
    /// Every event the machine refused.
    pub rejections: Vec<String>,
    /// Every `cx.spawn` a body made and what it answered, in order.
    pub spawns: Vec<SpawnOutcome>,
    /// The run stopped short of `End`.
    pub stuck: bool,
    /// How much engine time INV-8's bound forgives, for a run measured on a
    /// clock that is not exact.
    ///
    /// [`Duration::ZERO`] on a virtual or scripted clock, which is every run
    /// this crate drives itself. A real clock, compressed or not, fires a
    /// timer at or after its deadline and never before, so a driver measured
    /// on one overshoots by the scheduler's own slop; this is that slop, in
    /// the engine's units, and nothing else is relaxed by it.
    pub clock_slack: Duration,
    /// The key behind each node path.
    pub keys: Vec<(String, RawKey)>,
}

impl<Out> Driven<Out> {
    /// Check a run another driver produced, exactly as [`ScriptedDriver`]
    /// checks its own.
    ///
    /// The prefix check runs over every prefix of the finished trace rather
    /// than after each step, because the trace is what a real driver hands
    /// back; that is a superset of the per-step checks, not a weaker one.
    pub fn from_recorded(r: Recorded<Out>) -> Driven<Out> {
        let trace = r.report.trace.clone().unwrap_or_default();
        let mut violations = Vec::new();
        let mut first_violation = None;
        for len in 1..=trace.events.len() {
            let prefix = Trace {
                events: trace.events[..len].to_vec(),
            };
            for v in check_trace_prefix(&prefix, &r.view) {
                if !violations.contains(&v) {
                    violations.push(v);
                    if first_violation.is_none() {
                        first_violation = Some((len, len));
                    }
                }
            }
        }
        for v in check_whys(&r.whys) {
            if !violations.contains(&v) {
                violations.push(v);
            }
        }
        if !r.stuck {
            for v in check_trace_with_slack(&trace, &r.view, &r.report, r.clock_slack) {
                if !violations.contains(&v) {
                    violations.push(v);
                }
            }
        }
        Driven {
            report: r.report,
            trace,
            view: r.view,
            steps: r.steps,
            violations,
            rejections: r.rejections,
            spawns: r.spawns,
            stuck: r.stuck,
            first_violation,
            whys: r.whys,
            keys: r.keys,
        }
    }

    /// The trace, queried.
    pub fn eol(&self) -> Eol<'_> {
        Eol(&self.trace)
    }

    /// Every effect the machine returned, rendered, in order.
    pub fn effects(&self) -> Vec<String> {
        self.steps.iter().flat_map(|s| s.effects.clone()).collect()
    }

    /// The key behind a path. A template's inner node has one per live
    /// instance, and this is the first of them.
    pub fn key(&self, path: &str) -> RawKey {
        self.keys
            .iter()
            .find(|(p, _)| p == path)
            .map(|(_, k)| *k)
            .unwrap_or_else(|| panic!("no node {path}"))
    }

    /// Every run key behind a path: one per copy. A node the run never
    /// instantiated has none, which is an answer and not a failure.
    pub fn keys_of(&self, path: &str) -> Vec<RawKey> {
        self.keys
            .iter()
            .filter(|(p, _)| p == path)
            .map(|(_, k)| *k)
            .collect()
    }

    /// The last recorded `why` for a node at or before `secs`.
    pub fn why_at(&self, node: &str, secs: f64) -> Vec<(String, Reason)> {
        let t = Time::from_nanos((secs * 1e9) as u64);
        self.whys
            .iter()
            .rfind(|(at, n, _)| *at <= t && n == node)
            .map(|(_, _, on)| on.clone())
            .unwrap_or_default()
    }

    /// How many recorded `why` answers named a reason the predicate accepts.
    /// The machine records one after every step for every waiting node, so a
    /// non-zero count says the run really reached that kind of contention.
    pub fn waited(&self, want: fn(&Reason) -> bool) -> usize {
        self.whys
            .iter()
            .filter(|(_, _, on)| on.iter().any(|(_, r)| want(r)))
            .count()
    }

    /// Everything wrong with this run, rendered with the trace.
    pub fn problems(&self) -> Option<String> {
        if self.violations.is_empty() && self.rejections.is_empty() && !self.stuck {
            return None;
        }
        let mut s = String::new();
        for v in &self.violations {
            s.push_str(&format!("{}: {}\n", v.rule, v.detail));
        }
        for r in &self.rejections {
            s.push_str(&format!("rejected: {r}\n"));
        }
        if self.stuck {
            s.push_str("stuck: the run stopped short of End\n");
        }
        s.push_str("trace:\n");
        s.push_str(&self.eol().render());
        s.push_str("steps:\n");
        for st in &self.steps {
            s.push_str(&format!("  t={} {} -> {:?}\n", st.at, st.event, st.effects));
        }
        Some(s)
    }

    /// Assert the run ended, every invariant held, and nothing was refused.
    pub fn check(&self) {
        if let Some(p) = self.problems() {
            panic!("{p}");
        }
    }
}
