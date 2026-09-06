//! The machine's public face: construction, the clock hand-off, `begin`,
//! `step`, and what a host may read back.

use super::state::{Machine, NodeState, Purpose, St};
use super::table::{RootInput, Table};
use super::{Effect, Event, Rejected, RunState};
use crate::contracts::Time;
use crate::key::RawKey;
use crate::plan::{Kind, Plan};
use crate::report::Report;
use crate::sim::Schedule;
use crate::view::{NodePath, Reason, Why};

impl Machine {
    /// A machine for one run of a plan the caller supplies **no** input for:
    /// resources, steps, services, effects, joins, components and templates.
    ///
    /// Refuses a plan that declares a per-run input (nothing would ever fill
    /// its slot, so the nodes that need it could never start), a root plan with
    /// unresolved imports (`L-IMPORTS`), and one child plan used as two
    /// components. Components declaring input are refused throughout the
    /// declaration tree, including inside templates, before any effect.
    /// [`Machine::with_input`] is the constructor for a run that
    /// does supply one.
    pub fn new<Out, In>(plan: &Plan<Out, In>) -> Result<Machine, super::EngineError> {
        Table::build(plan.ir(), RootInput::Absent).map(Machine::from_table)
    }

    /// A machine for one run of a plan whose per-run input the caller supplies:
    /// `start(rt, input)` at the root, `cx.spawn(&template, input)` for an
    /// instance.
    ///
    /// The caller owes the run one thing the machine cannot check: the value
    /// must be in the run's slot for the input node *before* the first body is
    /// built (`bodies_of_with_input`, `BodySource::open_instance`). The input
    /// node is then not a node of the run at all, and a need on it is satisfied
    /// from the first step.
    ///
    /// A plan that declares no input is accepted here too — there is simply
    /// nothing to seed. Supplying root input does not supply a component's
    /// declared input; those are refused before any effects, including inside
    /// template declarations.
    pub fn with_input<Out, In>(plan: &Plan<Out, In>) -> Result<Machine, super::EngineError> {
        Table::build(plan.ir(), RootInput::Supplied).map(Machine::from_table)
    }

    /// Every node the declaration tree holds, a template's inner nodes
    /// included, as `(declaration key, path, kind)`.
    ///
    /// [`nodes`](Self::nodes) answers for the *run*, which has a template's
    /// nodes only once an instance exists. A harness that supplies bodies
    /// needs them before that.
    pub fn declarations<Out, In>(plan: &Plan<Out, In>) -> Vec<(RawKey, NodePath, Kind)> {
        Table::declarations(plan.ir())
    }

    /// Prefer these nodes when several become eligible in one step (T1's FIFO
    /// among waiters is then this order among the simultaneously ready).
    pub fn with_schedule(mut self, schedule: &Schedule) -> Machine {
        if let Schedule::Order(names) = schedule {
            for (rank, name) in names.iter().enumerate() {
                if let Some(i) = self.t.index_of_path(name) {
                    self.rank[i] = rank as u32;
                }
            }
            // Kept so an instance's nodes, which appear at run time, are
            // ranked by the same preference (INV-14).
            self.schedule = names.clone();
        }
        self
    }

    /// The ship boundary: `Planned → Admitting`, and the first starts.
    pub fn begin(&mut self) -> Vec<Effect> {
        if self.scopes[0].st == RunState::Planned {
            self.scopes[0].st = RunState::Admitting;
            self.admit(0);
            self.check_steady(0);
            self.try_cleanup(0);
        }
        std::mem::take(&mut self.fx)
    }

    /// What the machine believes the host's clock reads.
    pub fn now(&self) -> Time {
        self.now
    }

    /// Tell the machine the host's clock reading before an event observed at
    /// that time. Never moves backwards.
    pub fn advance(&mut self, now: Time) {
        if now > self.now {
            self.now = now;
        }
    }

    /// Feed one event; perform every effect returned, in order.
    ///
    /// Total: an event that makes no sense in the current state yields
    /// [`Effect::Reject`] and nothing else, never a panic (D1).
    pub fn step(&mut self, event: Event) -> Vec<Effect> {
        let described = format!("{event:?}");
        let result = match event {
            Event::ReadyPublished { node, result } => self
                .node(node)
                .and_then(|n| self.on_ready_published(n, result)),
            Event::Started(k) => self.body_node(k).and_then(|n| self.on_started(n)),
            Event::Held(k) => self.body_node(k).and_then(|n| {
                if !self.t.nodes[n].kind.can_hold() {
                    return Err("Held for a kind that carries no obligation");
                }
                self.on_held(n)
            }),
            Event::NodeOk(k) => self.body_node(k).and_then(|n| self.on_ok(n)),
            Event::NodeErr(k, kind) => match self.body_node(k) {
                Ok(n) => self.on_err(n, kind),
                Err(reason) => Err(reason),
            },
            Event::NodeCancelled { node, held } => self
                .body_node(node)
                .and_then(|n| self.on_cancelled(n, held)),
            Event::ServeEnded { node, fault } => match self.node(node) {
                Ok(n) => self.on_serve_ended(n, fault),
                Err(reason) => Err(reason),
            },
            Event::Timer(id) => self.on_timer(id),
            Event::ShutdownRequested => self.on_shutdown(),
            Event::CancelRequested => self.on_cancel(),
            Event::TaskJoined { node, joined } => {
                self.body_node(node).and_then(|n| self.on_joined(n, joined))
            }
            Event::InstanceSpawned {
                spawner,
                template,
                id,
            } => {
                if self.scopes[0].st == RunState::Ended {
                    Err("the run has ended")
                } else if self.scopes[0].st == RunState::Planned {
                    Err("the run has not begun")
                } else {
                    self.on_instance_spawned(spawner, template, id)
                }
            }
            Event::StopInstance(id) => {
                if self.scopes[0].st == RunState::Ended {
                    Err("the run has ended")
                } else {
                    self.on_stop_instance(id)
                }
            }
        };
        if let Err(reason) = result {
            self.fx.clear();
            self.fx.push(Effect::Reject(Rejected {
                event: described,
                reason,
            }));
        }
        std::mem::take(&mut self.fx)
    }

    fn node(&self, key: RawKey) -> Result<usize, &'static str> {
        if self.scopes[0].st == RunState::Ended {
            return Err("the run has ended");
        }
        if self.scopes[0].st == RunState::Planned {
            return Err("the run has not begun");
        }
        self.t.index_of(key).ok_or("unknown node")
    }

    /// The node behind a key for an event that reports on a **body**.
    ///
    /// A component and a join have no body — a component's attempt is its inner
    /// graph coming up, a join's is nothing at all — so the engine never spawns
    /// one and an outcome for one can only be a driver's mistake. Taken, it
    /// gives a component a fault vector the exit helpers assume is always empty
    /// (Stage 1 report § 4.3), and leaves its inner scope live behind a `Ready`
    /// or `Failed` component. D1 says so: a `Reject`, not a silent state.
    fn body_node(&self, key: RawKey) -> Result<usize, &'static str> {
        let n = self.node(key)?;
        match self.t.nodes[n].kind {
            Kind::Component | Kind::Join | Kind::Template => Err("this kind has no body"),
            _ => Ok(n),
        }
    }

    fn on_started(&mut self, n: usize) -> Result<(), &'static str> {
        match self.slots[n].st {
            St::Running | St::Abandoned => Ok(()),
            _ => Err("Started for a node with no body in flight"),
        }
    }

    fn on_joined(&mut self, n: usize, joined: super::JoinedLabel) -> Result<(), &'static str> {
        match joined {
            super::JoinedLabel::Cancelled => self.on_cancelled(n, self.slots[n].held),
            super::JoinedLabel::Panicked => {
                // `OD-PANIC-CANCELLED`: a body the engine had already cancelled
                // panicked on the way out. For a cooperative cancel `on_err`
                // sees `signalled` and does this; for a drop-mode `Abort` —
                // the default for resources, steps and effects — `signalled` is
                // false, and the panic the driver reports on the *join* is not
                // the body's own return. It is observed in the trace and is
                // not a fault. A `NodeErr(_, Panic)` still is: that is the
                // body's own outcome, delivered because it was already due.
                let s = &self.slots[n];
                if s.st == St::Running && s.cancelling && !s.signalled {
                    let phase = self.body_phase(n);
                    self.emit(
                        n,
                        crate::report::TraceKind::Fail(phase, crate::report::FaultLabel::Panic),
                    );
                    self.finish_interrupted(n);
                    return Ok(());
                }
                self.on_err(n, crate::report::FaultKind::Panic(Box::new("panicked")))
            }
            super::JoinedLabel::Done => Ok(()),
        }
    }

    fn on_timer(&mut self, id: super::TimerId) -> Result<(), &'static str> {
        if self.scopes[0].st == RunState::Ended {
            return Err("the run has ended");
        }
        let Some(pos) = self.timers.iter().position(|(t, _, _)| *t == id) else {
            // A timer the machine forgot (or never set): harmless.
            return if id.0 < self.next_timer {
                Ok(())
            } else {
                Err("unknown timer")
            };
        };
        let (_, purpose, at) = self.timers.remove(pos);
        self.advance(at);
        match purpose {
            Purpose::Within(n) => self.on_within_timer(n),
            Purpose::Grace(n) => self.on_grace_timer(n),
            Purpose::Backoff(n) => self.on_backoff_timer(n),
            Purpose::StopDeadline(n) => self.on_stop_deadline(n),
            Purpose::Budget(s) => self.on_budget_timer(s),
            Purpose::Zero(s) => self.on_zero_timer(s),
        }
        Ok(())
    }

    // ------------------------------------------------------------ readers

    /// Where the run is.
    pub fn run_state(&self) -> RunState {
        self.scopes[0].st
    }

    /// Whether `End` has been emitted.
    pub fn ended(&self) -> bool {
        self.scopes[0].st == RunState::Ended
    }

    /// The report, once, after `End`. Already in F4 order; `output` and
    /// `trace` are the host's to fill.
    pub fn take_report(&mut self) -> Option<Report<()>> {
        self.report.take()
    }

    /// The key behind a path.
    pub fn key_of(&self, path: &str) -> Option<RawKey> {
        self.t.index_of_path(path).map(|i| self.t.nodes[i].key)
    }

    /// The path behind a key.
    pub fn path_of(&self, key: RawKey) -> Option<&NodePath> {
        self.t.index_of(key).map(|i| &self.t.nodes[i].path)
    }

    /// What kind of node this key names.
    ///
    /// A run driver needs it: a service's readiness hands over a serve future
    /// and a component never gets a body at all.
    pub fn kind_of(&self, key: RawKey) -> Option<Kind> {
        self.t.index_of(key).map(|i| self.t.nodes[i].kind)
    }

    /// The deadline a body of this node must respect: its declared `within`
    /// from now, or else the scope's remaining shutdown budget.
    ///
    /// What `cx.deadline()` reads (contract § 6). The machine arms the timer
    /// either way; this is the same number, for the body to see.
    pub fn deadline_for(&self, key: RawKey) -> Option<Time> {
        let i = self.t.index_of(key)?;
        match self.t.nodes[i].attrs.within {
            Some(d) => Some(self.now + d),
            None => self.scopes[self.t.nodes[i].scope].deadline,
        }
    }

    /// Every node of the run, in flat table order: its key, its path and its
    /// kind. What a driver builds its own index from.
    pub fn nodes(&self) -> Vec<(RawKey, NodePath, Kind)> {
        self.t
            .nodes
            .iter()
            .map(|n| (n.key, n.path.clone(), n.kind))
            .collect()
    }

    /// Where a node is, by key.
    pub fn state(&self, key: RawKey) -> Option<NodeState> {
        self.t.index_of(key).map(|i| self.node_state(i))
    }

    /// Where a node is, by path.
    pub fn state_of(&self, path: &str) -> Option<NodeState> {
        self.t.index_of_path(path).map(|i| self.node_state(i))
    }

    /// Why a node has not started: the run-time answer, naming the need that
    /// is not ready, the holder of the lock, or the full pool.
    pub fn why(&self, path: &str) -> Option<Why> {
        let i = self.t.index_of_path(path)?;
        Some(Why {
            node: self.t.nodes[i].path.clone(),
            waits_on: self.waits_on(i),
        })
    }

    fn waits_on(&self, i: usize) -> Vec<(NodePath, Reason)> {
        let node = &self.t.nodes[i];
        let mut out = Vec::new();
        if !matches!(self.slots[i].st, St::Pending | St::Waiting) {
            return out;
        }
        for &d in &node.needs {
            if !matches!(self.slots[d].st, St::Ready | St::Finished) {
                let reason = if self.t.nodes[d].scope == node.scope {
                    Reason::DeclaredNeed
                } else {
                    Reason::Import
                };
                out.push((self.t.nodes[d].path.clone(), reason));
            }
        }
        for &r in &node.exclusive {
            if let Some(h) = self.locks[r].exclusive {
                out.push((self.t.nodes[h].path.clone(), Reason::Exclusive));
            }
            for &h in &self.locks[r].shared {
                out.push((self.t.nodes[h].path.clone(), Reason::Exclusive));
            }
        }
        for &r in &node.shared {
            if let Some(h) = self.locks[r].exclusive {
                out.push((self.t.nodes[h].path.clone(), Reason::Shared));
            }
        }
        if let Some(p) = node.pool {
            let decl = &self.t.scopes[node.scope].pools[p];
            if self.scopes[node.scope].pools[p] >= decl.limit {
                out.push((NodePath::root(&decl.name), Reason::Pool));
            }
        }
        // T1's FIFO: an earlier waiter's unmet want refuses this grant even
        // when the grant itself is free. Without this the delay is silent and
        // `Waiting{on}` is empty, which contract § 2 forbids.
        if let Some(b) = self.slots[i].blocked_by {
            out.push((self.t.nodes[b].path.clone(), Reason::QueuedBehind));
        }
        if !self.admitting(node.scope) {
            out.push((
                NodePath::root(&self.t.scopes[node.scope].name),
                Reason::ScopeNotAdmitting,
            ));
        }
        out
    }

    fn node_state(&self, i: usize) -> NodeState {
        let s = &self.slots[i];
        match s.st {
            St::Pending => NodeState::Pending,
            St::Waiting => NodeState::Waiting {
                on: self.waits_on(i),
            },
            St::Running => NodeState::Running {
                attempt: s.attempt,
                held: s.held,
            },
            St::Backoff => NodeState::Backoff {
                attempt: s.attempt,
                until: s.backoff_until,
            },
            St::RetryRelease => NodeState::Releasing,
            St::Publishing => NodeState::Publishing,
            St::Ready => NodeState::Ready,
            St::Live => NodeState::Live,
            St::StoppingInstances => NodeState::Stopping,
            St::Finished => NodeState::Finished,
            St::Failed => NodeState::Failed { held: s.held },
            St::Interrupted => NodeState::Interrupted {
                held: s.held || (self.t.nodes[i].kind == Kind::Component && s.started),
            },
            St::Ambiguous => NodeState::Ambiguous,
            St::Skipped => NodeState::Skipped {
                because: s.because.map(|b| self.t.nodes[b].path.clone()),
            },
            St::Releasing => NodeState::Releasing,
            St::Compensating => NodeState::Compensating,
            St::Stopping => NodeState::Stopping,
            St::Released => NodeState::Released,
            St::Compensated => NodeState::Compensated,
            St::Stopped => NodeState::Stopped,
            St::ReleaseFailed => NodeState::ReleaseFailed,
            St::Abandoned => NodeState::Abandoned,
        }
    }
}
