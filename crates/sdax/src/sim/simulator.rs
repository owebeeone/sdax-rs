//! The stepping simulator: executes the machine's effects per a [`Script`] on
//! a virtual clock and feeds the events back, one queue item at a time.
//!
//! Ordering within one instant is FIFO in scheduling order, so an outcome
//! already due when an `Abort` arrives is delivered — the body had finished
//! before the abort could land, as it can on a real runtime — while anything
//! due later is dropped by the abort. No sleeping, no runtime, no bodies.

use super::instances::{Awaiting, SpawnOutcome};
use super::script::{At, Body, Cleanup, Script, Serve, SpawnSpec};
use crate::contracts::Time;
use crate::cx::InstanceId;
use crate::host::engine::{Event, Machine};
use crate::key::RawKey;
use crate::plan::{Kind, Plan};
use crate::report::{Report, Trace};
use std::collections::VecDeque;

/// A script that does not fit the plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptError {
    /// The machine refused the plan.
    Engine(crate::host::engine::EngineError),
    /// The script names a node the plan does not have.
    UnknownNode(String),
}

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScriptError::Engine(e) => write!(f, "{e}"),
            ScriptError::UnknownNode(n) => {
                write!(f, "the script names {n:?}, which the plan does not declare")
            }
        }
    }
}
impl std::error::Error for ScriptError {}

/// One step of a simulation: the event fed at that time, and what the
/// machine answered, rendered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimStep {
    /// When.
    pub at: Time,
    /// The event, as `Debug` renders it.
    pub event: String,
    /// The effects, as `Debug` renders them.
    pub effects: Vec<String>,
}

/// A body error with the scripted message.
#[derive(Debug)]
pub(super) struct Scripted(pub String);
impl std::fmt::Display for Scripted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Scripted {}

#[derive(Debug)]
pub(super) enum Item {
    Begin,
    Event(Event),
    /// A body event for a node, dropped by an abort if still due later.
    Body(RawKey, Event),
    /// A scripted `cx.spawn` from this node's body: which directive.
    Spawn(RawKey, usize),
}

pub(super) struct Queued {
    pub at: Time,
    pub seq: u64,
    pub item: Item,
}

pub(super) struct NodeScript {
    pub key: RawKey,
    pub kind: Kind,
    /// The path the script names this node by. Every instance of a template
    /// shares it, so one script line covers them all.
    pub path: String,
    pub bodies: Vec<Body>,
    pub serves: Vec<Serve>,
    pub cleanup: Cleanup,
    pub spawns: Vec<SpawnSpec>,
    /// Attempts spawned so far.
    pub attempts: usize,
    /// Serving episodes so far.
    pub episodes: usize,
    pub held: bool,
    pub stopped: bool,
    /// The instances this body created, in directive order.
    pub children: Vec<InstanceId>,
    /// Those whose readiness it awaits before returning (INV-17).
    pub awaited: Vec<InstanceId>,
}

/// The pure machine, driven by a script on a virtual clock.
pub struct Simulator {
    pub(super) machine: Machine,
    pub(super) nodes: Vec<NodeScript>,
    pub(super) queue: Vec<Queued>,
    pub(super) publications: VecDeque<Event>,
    pub(super) seq: u64,
    pub(super) now: Time,
    pub(super) trace: Trace,
    pub(super) steps: Vec<SimStep>,
    pub(super) rejections: Vec<String>,
    pub(super) ended: bool,
    pub(super) script: Script,
    /// Every template of the plan tree, by the path the view shows it at.
    pub(super) templates: Vec<(String, RawKey)>,
    pub(super) next_id: u64,
    pub(super) spawns: Vec<SpawnOutcome>,
    pub(super) parked: Vec<(RawKey, Awaiting)>,
}

impl Simulator {
    /// A simulation of `plan` under `script`, not yet started.
    ///
    /// Supplies no per-run input, so a plan that declares one is refused
    /// (`EngineError::TemplateAsScope`). [`Simulator::with_input`] is the
    /// entry point for a run that supplies one.
    pub fn new<Out, In>(plan: &Plan<Out, In>, script: &Script) -> Result<Simulator, ScriptError> {
        Simulator::over(
            Machine::new(plan).map_err(ScriptError::Engine)?,
            plan,
            script,
        )
    }

    /// A simulation of a plan whose per-run input the caller supplies.
    ///
    /// The value itself is not taken: a simulated body reads no slot, so all
    /// that matters here is that the input *has* one — which is what decides
    /// whether the plan may run at all.
    pub fn with_input<Out, In>(
        plan: &Plan<Out, In>,
        script: &Script,
    ) -> Result<Simulator, ScriptError> {
        Simulator::over(
            Machine::with_input(plan).map_err(ScriptError::Engine)?,
            plan,
            script,
        )
    }

    fn over<Out, In>(
        machine: Machine,
        plan: &Plan<Out, In>,
        script: &Script,
    ) -> Result<Simulator, ScriptError> {
        let machine = machine.with_schedule(&script.schedule);
        let declared = Machine::declarations(plan);
        let templates: Vec<(String, RawKey)> = declared
            .iter()
            .filter(|(_, _, k)| *k == Kind::Template)
            .map(|(key, path, _)| (path.to_string(), *key))
            .collect();
        let named: Vec<String> = declared.iter().map(|(_, p, _)| p.to_string()).collect();
        for name in script.named_nodes() {
            if !named.iter().any(|p| p == name) {
                return Err(ScriptError::UnknownNode(name.to_string()));
            }
        }
        let mut sim = Simulator {
            machine,
            nodes: Vec::new(),
            queue: Vec::new(),
            publications: VecDeque::new(),
            seq: 0,
            now: Time::ZERO,
            trace: Trace::default(),
            steps: Vec::new(),
            rejections: Vec::new(),
            ended: false,
            script: script.clone(),
            templates,
            next_id: 0,
            spawns: Vec::new(),
            parked: Vec::new(),
        };
        for (key, path, kind) in sim.machine.nodes() {
            let ns = sim.node_script(key, kind, path.to_string());
            sim.nodes.push(ns);
        }
        // Requests at the origin precede the first poll (`@0 cancel`).
        for (t, r) in &script.requests {
            let ev = match r {
                super::script::Request::Shutdown => Event::ShutdownRequested,
                super::script::Request::Cancel => Event::CancelRequested,
            };
            sim.push(Time::ZERO + *t, Item::Event(ev));
        }
        sim.push(Time::ZERO, Item::Begin);
        Ok(sim)
    }

    /// The script lines for one node, by the path it is named at.
    pub(super) fn node_script(&self, key: RawKey, kind: Kind, path: String) -> NodeScript {
        NodeScript {
            key,
            kind,
            bodies: self
                .script
                .body_of(&path)
                .map(|b| b.to_vec())
                .unwrap_or_else(|| vec![Body::default()]),
            serves: self
                .script
                .serve_of(&path)
                .map(|s| s.to_vec())
                .unwrap_or_else(|| vec![Serve::default()]),
            cleanup: self.script.cleanup_of(&path).cloned().unwrap_or_default(),
            spawns: self
                .script
                .spawns_of(&path)
                .map(|s| s.to_vec())
                .unwrap_or_default(),
            path,
            attempts: 0,
            episodes: 0,
            held: false,
            stopped: false,
            children: Vec::new(),
            awaited: Vec::new(),
        }
    }

    pub(super) fn push(&mut self, at: Time, item: Item) {
        self.seq += 1;
        self.queue.push(Queued {
            at,
            seq: self.seq,
            item,
        });
    }

    pub(super) fn resolve(&self, at: At, start: Time) -> Time {
        match at {
            At::Tick(d) => (Time::ZERO + d).max(start),
            At::After(d) => start + d,
        }
    }

    pub(super) fn node_mut(&mut self, key: RawKey) -> &mut NodeScript {
        self.nodes
            .iter_mut()
            .find(|n| n.key == key)
            .expect("effects name known nodes")
    }

    /// Feed the next queue item. `None` once the run has ended or nothing is
    /// left to deliver (a stuck run: an unbounded budget waiting on a body
    /// that never completes).
    pub fn step(&mut self) -> Option<&SimStep> {
        if self.ended || self.queue.is_empty() {
            return None;
        }
        let pos = (0..self.queue.len())
            .min_by_key(|&i| (self.queue[i].at, self.queue[i].seq))
            .expect("non-empty");
        let q = self.queue.remove(pos);
        self.now = self.now.max(q.at);
        self.machine.advance(self.now);
        let (event, effects) = match q.item {
            Item::Begin => ("begin".to_string(), self.machine.begin()),
            Item::Spawn(node, which) => match self.do_spawn(node, which) {
                Some(ev) => {
                    let described = format!("{ev:?}");
                    (described, self.machine.step(ev))
                }
                // A refused `cx.spawn` is the body's answer, not the run's:
                // nothing is fed to the machine and the outcome is recorded.
                None => ("spawn refused".to_string(), Vec::new()),
            },
            Item::Body(node, ev) => {
                // INV-17: an initializer that awaits `Child::ready()` returns
                // only once the instances it awaits have answered.
                if self.hold_for_ready(node, &ev) {
                    ("awaiting an instance".to_string(), Vec::new())
                } else {
                    let described = format!("{ev:?}");
                    (described, self.machine.step(ev))
                }
            }
            Item::Event(ev) => {
                let described = format!("{ev:?}");
                (described, self.machine.step(ev))
            }
        };
        self.perform_batch(event, effects);
        while let Some(ack) = self.publications.pop_front() {
            let described = format!("{ack:?}");
            let effects = self.machine.step(ack);
            self.perform_batch(described, effects);
        }
        self.release_awaits();
        self.steps.last()
    }

    fn perform_batch(&mut self, event: String, effects: Vec<crate::host::engine::Effect>) {
        let rendered: Vec<String> = effects.iter().map(|e| format!("{e:?}")).collect();
        for e in effects {
            self.perform(e);
        }
        self.steps.push(SimStep {
            at: self.now,
            event,
            effects: rendered,
        });
    }

    /// Run to the end, or until nothing is left to deliver.
    pub fn run(&mut self) {
        while self.step().is_some() {}
    }

    /// The trace so far.
    pub fn trace(&self) -> &Trace {
        &self.trace
    }

    /// Every step so far.
    pub fn steps(&self) -> &[SimStep] {
        &self.steps
    }

    /// The machine, for a harness that reads its state.
    pub fn machine(&self) -> &Machine {
        &self.machine
    }

    /// The virtual clock.
    pub fn now(&self) -> Time {
        self.now
    }

    /// Whether `End` was emitted.
    pub fn ended(&self) -> bool {
        self.ended
    }

    /// Whether the run stopped short of `End` with nothing left to deliver.
    pub fn stuck(&self) -> bool {
        !self.ended && self.queue.is_empty()
    }

    /// Every rejected event, rendered.
    pub fn rejections(&self) -> &[String] {
        &self.rejections
    }

    /// Every scripted `cx.spawn` and what it answered, in order.
    pub fn spawns(&self) -> &[SpawnOutcome] {
        &self.spawns
    }

    /// The report, once, after `End`, with the trace attached.
    pub fn take_report<Out>(&mut self) -> Option<Report<Out>> {
        let r = self.machine.take_report()?;
        Some(Report {
            outcome: r.outcome,
            output: None,
            faults: r.faults,
            cleanup_failures: r.cleanup_failures,
            incomplete: r.incomplete,
            ambiguous: r.ambiguous,
            trace: Some(self.trace.clone()),
        })
    }

    /// Items still queued after `End`, fed to the machine so a harness can
    /// check they are refused rather than acted on (D1).
    pub fn drain_after_end(&mut self) -> Vec<Vec<crate::host::engine::Effect>> {
        let mut out = Vec::new();
        while let Some(q) = self.queue.pop() {
            if let Item::Event(ev) | Item::Body(_, ev) = q.item {
                out.push(self.machine.step(ev));
            }
        }
        out
    }
}

impl<Out, In> Plan<Out, In> {
    /// The trace the pure machine produces for a scripted schedule, with no
    /// body run and no effect: the counterfactual answered pre-ship with the
    /// code that drives production (contract § 9, adoption A3).
    ///
    /// Refuses a plan the machine cannot run and a script that names an
    /// unknown node. A plan that declares a per-run input is one the machine
    /// cannot run without a value for it, so that plan wants
    /// [`simulate_with_input`](Self::simulate_with_input).
    pub fn simulate(&self, script: &Script) -> Result<Trace, ScriptError> {
        let mut sim = Simulator::new(self, script)?;
        sim.run();
        Ok(sim.trace().clone())
    }

    /// [`simulate`](Self::simulate) for a plan started with a per-run input —
    /// the counterfactual for what `start(rt, input)` will do.
    ///
    /// The value is dropped: no body runs in a simulation, so nothing reads a
    /// slot. What it changes is that the input *is* supplied, which is what
    /// decides whether the plan may run at all.
    pub fn simulate_with_input(&self, input: In, script: &Script) -> Result<Trace, ScriptError> {
        drop(input);
        let mut sim = Simulator::with_input(self, script)?;
        sim.run();
        Ok(sim.trace().clone())
    }
}

#[cfg(test)]
mod publication_tests {
    use super::*;
    use crate::{Mode, Policy, Shutdown};

    #[test]
    fn publications_are_drained_before_the_next_scheduled_event() {
        let mut builder = Plan::builder("Publication");
        let first = builder.join("first", ());
        let second = builder.join("second", ());
        let third = builder.join("third", first);
        let plan = builder
            .build(Policy::FailFast, Shutdown::unbounded(), Mode::Finite)
            .unwrap();
        let mut sim = Simulator::new(&plan, &Script::default()).unwrap();
        sim.step().unwrap();
        let acknowledgements: Vec<_> = sim
            .steps()
            .iter()
            .filter(|step| step.event.starts_with("ReadyPublished"))
            .collect();
        assert_eq!(acknowledgements.len(), 3);
        for (step, key) in acknowledgements
            .iter()
            .zip([first.raw(), second.raw(), third.raw()])
        {
            assert!(step
                .event
                .starts_with(&format!("ReadyPublished {{ node: {key:?},")));
        }
        assert!(sim.rejections().is_empty());
        sim.run();
        assert!(sim.ended());
    }
}
