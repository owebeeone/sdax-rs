//! The stepping simulator: executes the machine's effects per a [`Script`]
//! on a virtual clock and feeds the events back, one queue item at a time.
//!
//! Ordering within one instant is FIFO in scheduling order, so an outcome
//! already due when an `Abort` arrives is delivered — the body had finished
//! before the abort could land, as it can on a real runtime — while anything
//! due later is dropped by the abort. No sleeping, no runtime, no bodies.

use super::script::{At, Body, Cleanup, Ending, Request, Script, Serve};
use crate::contracts::Time;
use crate::host::engine::{Effect, Event, Machine};
use crate::key::RawKey;
use crate::plan::{Kind, Plan};
use crate::report::{FaultKind, Report, Trace, TraceEvent};
use crate::view::NodePath;

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
struct Scripted(String);
impl std::fmt::Display for Scripted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Scripted {}

#[derive(Debug)]
enum Item {
    Begin,
    Event(Event),
    /// A body event for a node, dropped by an abort if still due later.
    Body(RawKey, Event),
}

struct Queued {
    at: Time,
    seq: u64,
    item: Item,
}

struct NodeScript {
    key: RawKey,
    kind: Kind,
    bodies: Vec<Body>,
    serves: Vec<Serve>,
    cleanup: Cleanup,
    /// Attempts spawned so far.
    attempts: usize,
    /// Serving episodes so far.
    episodes: usize,
    held: bool,
    stopped: bool,
}

/// The pure machine, driven by a script on a virtual clock.
pub struct Simulator {
    machine: Machine,
    nodes: Vec<NodeScript>,
    queue: Vec<Queued>,
    seq: u64,
    now: Time,
    trace: Trace,
    steps: Vec<SimStep>,
    rejections: Vec<String>,
    ended: bool,
}

impl Simulator {
    /// A simulation of `plan` under `script`, not yet started.
    pub fn new<Out>(plan: &Plan<Out>, script: &Script) -> Result<Simulator, ScriptError> {
        let machine = Machine::new(plan)
            .map_err(ScriptError::Engine)?
            .with_schedule(&script.schedule);
        let view = plan.inspect();
        let mut nodes = Vec::new();
        for n in &view.nodes {
            let path = n.path.to_string();
            let key = machine
                .key_of(&path)
                .expect("every viewed node is in the table");
            let bodies = script
                .bodies
                .iter()
                .find(|(p, _)| *p == path)
                .map(|(_, b)| b.clone())
                .unwrap_or_else(|| vec![Body::default()]);
            let serves = script
                .serves
                .iter()
                .find(|(p, _)| *p == path)
                .map(|(_, s)| s.clone())
                .unwrap_or_else(|| vec![Serve::default()]);
            let cleanup = script
                .cleanups
                .iter()
                .find(|(p, _)| *p == path)
                .map(|(_, c)| c.clone())
                .unwrap_or_default();
            nodes.push(NodeScript {
                key,
                kind: n.kind,
                bodies,
                serves,
                cleanup,
                attempts: 0,
                episodes: 0,
                held: false,
                stopped: false,
            });
        }
        for name in script.named_nodes() {
            if machine.key_of(name).is_none() {
                return Err(ScriptError::UnknownNode(name.to_string()));
            }
        }
        let mut sim = Simulator {
            machine,
            nodes,
            queue: Vec::new(),
            seq: 0,
            now: Time::ZERO,
            trace: Trace::default(),
            steps: Vec::new(),
            rejections: Vec::new(),
            ended: false,
        };
        // Requests at the origin precede the first poll (`@0 cancel`).
        for (t, r) in &script.requests {
            let ev = match r {
                Request::Shutdown => Event::ShutdownRequested,
                Request::Cancel => Event::CancelRequested,
            };
            sim.push(Time::ZERO + *t, Item::Event(ev));
        }
        sim.push(Time::ZERO, Item::Begin);
        Ok(sim)
    }

    fn push(&mut self, at: Time, item: Item) {
        self.seq += 1;
        self.queue.push(Queued {
            at,
            seq: self.seq,
            item,
        });
    }

    fn resolve(&self, at: At, start: Time) -> Time {
        match at {
            At::Tick(d) => (Time::ZERO + d).max(start),
            At::After(d) => start + d,
        }
    }

    fn node_mut(&mut self, key: RawKey) -> &mut NodeScript {
        self.nodes
            .iter_mut()
            .find(|n| n.key == key)
            .expect("effects name known nodes")
    }

    /// Perform one effect.
    fn perform(&mut self, effect: Effect) {
        let now = self.now;
        match effect {
            Effect::Spawn { node, .. } | Effect::SpawnBlocking { node, .. } => {
                // A run driver holds one task handle per node: when the
                // machine gives up on an attempt and starts the next, the old
                // handle's result no longer maps to anything and the driver
                // drops it. The simulator stands in for one, so it drops the
                // superseded attempt's queued events here. A blocking body is
                // the case that needs it — it cannot be aborted (T7), so
                // `on_within_timer` fails the attempt and says the thread's
                // later outcome is ignored — and nothing else did, so the
                // stale outcome ended the attempt that had just started, which
                // then reached `Ready` before its own `Started` arrived.
                // A cleanup outcome is never pending here: INV-12 finishes a
                // held attempt's release before attempt k+1 starts. A serve
                // episode is not an attempt and is left alone.
                self.queue.retain(|q| match &q.item {
                    Item::Body(k, ev) if *k == node => !matches!(
                        ev,
                        Event::Started(_)
                            | Event::Held(_)
                            | Event::NodeOk(_)
                            | Event::NodeErr(..)
                            | Event::NodeCancelled { .. }
                    ),
                    _ => true,
                });
                let ns = self.node_mut(node);
                let body = ns.bodies[ns.attempts.min(ns.bodies.len() - 1)].clone();
                ns.attempts += 1;
                ns.held = false;
                ns.stopped = false;
                let kind = ns.kind;
                self.push(now, Item::Body(node, Event::Started(node)));
                let holds = matches!(kind, Kind::Resource | Kind::Effect);
                let end_at = match &body.ending {
                    Ending::Ok(at) | Ending::Fail(at, _) | Ending::Panic(at) => {
                        Some(self.resolve(*at, now))
                    }
                    Ending::Pending => None,
                };
                if holds {
                    let held_at = match (body.held, &body.ending) {
                        (Some(at), _) => Some(self.resolve(at, now)),
                        (None, Ending::Ok(_)) => end_at,
                        _ => None,
                    };
                    if let Some(t) = held_at {
                        // A body registers its value *inside* itself, so a
                        // hold can never be later than the body's own ending.
                        // A script that says otherwise holds at the ending
                        // instant instead, and is delivered first (it was
                        // queued first, so its sequence number is lower).
                        // Without this the machine is fed a `Held` for a body
                        // that has already returned, and rejects it.
                        let t = match end_at {
                            Some(e) => t.min(e),
                            None => t,
                        };
                        self.push(t, Item::Body(node, Event::Held(node)));
                    }
                }
                if let Some(t) = end_at {
                    let ev = match body.ending {
                        Ending::Ok(_) => Event::NodeOk(node),
                        // A try-step's `Err` is its value, not a fault
                        // (OD-5): the body the engine sees returns `Ok`.
                        Ending::Fail(_, _) if kind == Kind::TryStep => Event::NodeOk(node),
                        Ending::Fail(_, msg) => {
                            Event::NodeErr(node, FaultKind::Error(Box::new(Scripted(msg))))
                        }
                        Ending::Panic(_) => {
                            Event::NodeErr(node, FaultKind::Panic(Box::new("scripted panic")))
                        }
                        Ending::Pending => unreachable!(),
                    };
                    self.push(t, Item::Body(node, ev));
                }
            }
            Effect::Abort(node) => {
                // Outcomes already due stand; later ones are dropped, and the
                // join reports the cancellation.
                let due_now = self.queue.iter().any(|q| {
                    q.at <= now && matches!(&q.item, Item::Body(k, ev) if *k == node && matches!(ev, Event::NodeOk(_) | Event::NodeErr(..)))
                });
                self.queue
                    .retain(|q| !(q.at > now && matches!(&q.item, Item::Body(k, _) if *k == node)));
                if !due_now {
                    let held = self.node_mut(node).held;
                    self.push(now, Item::Body(node, Event::NodeCancelled { node, held }));
                }
            }
            Effect::Signal(node) | Effect::StopService(node) => {
                let ns = self.node_mut(node);
                if ns.kind == Kind::Service && ns.episodes > 0 && !ns.stopped {
                    ns.stopped = true;
                    let serve = ns.serves[(ns.episodes - 1).min(ns.serves.len() - 1)].clone();
                    if let Serve::StopsAfter(d) = serve {
                        self.push(
                            now + d,
                            Item::Body(node, Event::ServeEnded { node, fault: None }),
                        );
                    }
                }
            }
            Effect::Release(node) | Effect::Compensate(node) => {
                let cleanup = self.node_mut(node).cleanup.clone();
                match cleanup {
                    Cleanup::Ok(d) => self.push(now + d, Item::Body(node, Event::NodeOk(node))),
                    Cleanup::Fail(d, msg) => self.push(
                        now + d,
                        Item::Body(
                            node,
                            Event::NodeErr(node, FaultKind::Error(Box::new(Scripted(msg)))),
                        ),
                    ),
                    Cleanup::Panic(d) => self.push(
                        now + d,
                        Item::Body(
                            node,
                            Event::NodeErr(node, FaultKind::Panic(Box::new("scripted panic"))),
                        ),
                    ),
                    Cleanup::IgnoreStop => {}
                }
            }
            Effect::Timer { id, at } => self.push(at, Item::Event(Event::Timer(id))),
            Effect::CancelTimer(id) => self
                .queue
                .retain(|q| !matches!(&q.item, Item::Event(Event::Timer(t)) if *t == id)),
            Effect::Emit(ev) => self.observe(*ev),
            Effect::End(_) => self.ended = true,
            Effect::Reject(r) => self.rejections.push(format!("{} — {}", r.reason, r.event)),
            Effect::SpawnInstance { .. } => self.rejections.push("SpawnInstance is Stage 3".into()),
        }
    }

    fn observe(&mut self, ev: TraceEvent) {
        // A service's readiness starts its serving episode.
        if let (crate::report::TraceKind::Ready, Some(path)) = (&ev.kind, &ev.node) {
            self.serve_begins(path.clone());
        }
        if let (crate::report::TraceKind::Held, Some(path)) = (&ev.kind, &ev.node) {
            let key = self.machine.key_of(&path.to_string()).expect("known");
            self.node_mut(key).held = true;
        }
        self.trace.events.push(ev);
    }

    fn serve_begins(&mut self, path: NodePath) {
        let key = self.machine.key_of(&path.to_string()).expect("known");
        let now = self.now;
        let ns = self.node_mut(key);
        if ns.kind != Kind::Service {
            return;
        }
        let serve = ns.serves[ns.episodes.min(ns.serves.len() - 1)].clone();
        ns.episodes += 1;
        ns.stopped = false;
        match serve {
            Serve::Ok(at) => {
                let t = self.resolve(at, now);
                self.push(
                    t,
                    Item::Body(
                        key,
                        Event::ServeEnded {
                            node: key,
                            fault: None,
                        },
                    ),
                );
            }
            Serve::Err(at, msg) => {
                let t = self.resolve(at, now);
                self.push(
                    t,
                    Item::Body(
                        key,
                        Event::ServeEnded {
                            node: key,
                            fault: Some(FaultKind::Error(Box::new(Scripted(msg)))),
                        },
                    ),
                );
            }
            Serve::IgnoreStop | Serve::StopsAfter(_) => {}
        }
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
            Item::Event(ev) | Item::Body(_, ev) => {
                let described = format!("{ev:?}");
                (described, self.machine.step(ev))
            }
        };
        let rendered: Vec<String> = effects.iter().map(|e| format!("{e:?}")).collect();
        for e in effects {
            self.perform(e);
        }
        self.steps.push(SimStep {
            at: self.now,
            event,
            effects: rendered,
        });
        self.steps.last()
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
    pub fn drain_after_end(&mut self) -> Vec<Vec<Effect>> {
        let mut out = Vec::new();
        while let Some(q) = self.queue.pop() {
            if let Item::Event(ev) | Item::Body(_, ev) = q.item {
                out.push(self.machine.step(ev));
            }
        }
        out
    }
}

impl<Out> Plan<Out> {
    /// The trace the pure machine produces for a scripted schedule, with no
    /// body run and no effect: the counterfactual answered pre-ship with the
    /// code that drives production (contract § 9, adoption A3).
    ///
    /// Refuses a plan the machine cannot run (templates are Stage 3) and a
    /// script that names an unknown node.
    pub fn simulate(&self, script: &Script) -> Result<Trace, ScriptError> {
        let mut sim = Simulator::new(self, script)?;
        sim.run();
        Ok(sim.trace().clone())
    }
}
