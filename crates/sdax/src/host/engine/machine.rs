//! The machine's public face: construction, the clock hand-off, `begin`,
//! `step`, and what a host may read back.

use super::state::{Machine, NodeState, Purpose, St};
use super::table::Table;
use super::{Effect, Event, Rejected, RunState};
use crate::contracts::Time;
use crate::key::RawKey;
use crate::plan::{Kind, Plan};
use crate::report::Report;
use crate::sim::Schedule;
use crate::view::{NodePath, Reason, Why};

impl Machine {
    /// A machine for one run of a static plan (resources, steps, services,
    /// effects, joins and components).
    ///
    /// Refuses a plan with templates — dynamic instances are Stage 3, and a
    /// silent no-op would be a claim this crate cannot make — a root plan
    /// with unresolved imports (`L-IMPORTS`), and one child plan used as two
    /// components.
    pub fn new<Out>(plan: &Plan<Out>) -> Result<Machine, super::EngineError> {
        Table::build(plan.ir()).map(Machine::from_table)
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
            Event::Started(k) => self.node(k).and_then(|n| self.on_started(n)),
            Event::Held(k) => self.node(k).and_then(|n| self.on_held(n)),
            Event::NodeOk(k) => self.node(k).and_then(|n| self.on_ok(n)),
            Event::NodeErr(k, kind) => match self.node(k) {
                Ok(n) => self.on_err(n, kind),
                Err(reason) => Err(reason),
            },
            Event::NodeCancelled { node, held } => {
                self.node(node).and_then(|n| self.on_cancelled(n, held))
            }
            Event::ServeEnded { node, fault } => match self.node(node) {
                Ok(n) => self.on_serve_ended(n, fault),
                Err(reason) => Err(reason),
            },
            Event::Timer(id) => self.on_timer(id),
            Event::ShutdownRequested => self.on_shutdown(),
            Event::CancelRequested => self.on_cancel(),
            Event::TaskJoined { node, joined } => {
                self.node(node).and_then(|n| self.on_joined(n, joined))
            }
            Event::InstanceSpawned { .. } | Event::InstanceEnded { .. } => {
                Err("template instances are Stage 3")
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
            St::Ready => NodeState::Ready,
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
