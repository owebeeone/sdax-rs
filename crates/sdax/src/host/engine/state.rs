//! The machine's state: what the contract's § 2 tables say, as data, plus the
//! bookkeeping T1–T8 need and the helpers that write the trace and the report.

use super::table::Table;
use super::{Effect, TimerId};
use crate::contracts::Time;
use crate::cx::InstanceId;
use crate::plan::Kind;
use crate::plan::ReleaseStyle;
use crate::policy::Ambiguity;
use crate::report::{
    Fault, FaultKind, NodeRecord, Outcome, Phase, RecordOrder, Report, TraceEvent, TraceKind,
};
use crate::view::{NodePath, Reason};

/// Why a plan cannot be run by this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineError {
    /// A plan that declares a per-run input was offered to a run that supplies
    /// **no** value for it — a template used as a scope, or a root started
    /// through an entry point that takes no input. Names the input node.
    TemplateAsScope(NodePath),
    /// A root run cannot resolve these import nodes (`L-IMPORTS`).
    UnresolvedImports(Vec<NodePath>),
    /// One child plan is registered as two components. The engine addresses
    /// nodes by key, and a plan used twice cannot keep those unique.
    DuplicateComponent {
        /// The child plan's name.
        plan: String,
        /// Where it was used first.
        first: NodePath,
        /// Where it was used again.
        second: NodePath,
    },
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names = |v: &[NodePath]| -> String {
            v.iter()
                .map(|p| p.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        };
        match self {
            EngineError::TemplateAsScope(p) => write!(
                f,
                "this plan declares the input {p} and nothing supplies a value for it: start it \
                 with start(rt, input), or instantiate it with cx.spawn"
            ),
            EngineError::UnresolvedImports(v) => write!(
                f,
                "a root run cannot resolve import(s) {} (L-IMPORTS)",
                names(v)
            ),
            EngineError::DuplicateComponent {
                plan,
                first,
                second,
            } => write!(
                f,
                "child plan {plan:?} is used as component {first} and again as {second}; the \
                 engine addresses nodes by key, so one child plan can be one component"
            ),
        }
    }
}

impl std::error::Error for EngineError {}

/// Where a run is (contract § 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RunState {
    /// Built; `begin` not yet called.
    Planned,
    /// Nodes start as they become eligible.
    Admitting,
    /// Every node settled; services serving.
    Steady,
    /// No new starts; in-flight bodies being cancelled.
    Settling,
    /// The release graph is running.
    Cleanup,
    /// `End` was emitted.
    Ended,
}

/// Where a node is (contract § 2), as the host may read it back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeState {
    /// Declared; not yet considered.
    Pending,
    /// Need-ready, but a lock or pool grant is missing.
    Waiting {
        /// Each reason, as `why` would list it.
        on: Vec<(NodePath, Reason)>,
    },
    /// The prepare body is in flight.
    Running {
        /// Which attempt.
        attempt: u32,
        /// Whether a value was registered in this attempt.
        held: bool,
    },
    /// Between attempts.
    Backoff {
        /// The attempt that failed.
        attempt: u32,
        /// When the next one starts.
        until: Time,
    },
    /// The body returned `Ok` (a service: serving; a component: inner steady).
    Ready,
    /// A template, admitting instances. A template has no body and never
    /// becomes `Ready` (contract § 1); it is live from the moment its imports
    /// are ready until its obligation — stopping every live instance — opens.
    Live,
    /// A service's serve future, or a component's inner run, ended on its own.
    Finished,
    /// Attempts exhausted.
    Failed {
        /// Whether a release is owed.
        held: bool,
    },
    /// The engine cancelled the body; never a fault.
    Interrupted {
        /// Whether a release is owed.
        held: bool,
    },
    /// An effect interrupted or timed out after start and before `hold`.
    Ambiguous,
    /// Never started.
    Skipped {
        /// The node whose outcome caused it, when a fault did.
        because: Option<NodePath>,
    },
    /// A release body is running.
    Releasing,
    /// The release returned `Ok`.
    Released,
    /// The release failed.
    ReleaseFailed,
    /// A compensation is running.
    Compensating,
    /// The compensation returned `Ok`.
    Compensated,
    /// A service was signalled and is being waited for.
    Stopping,
    /// The service stopped.
    Stopped,
    /// The budget expired while the obligation was still running.
    Abandoned,
}

/// The internal node state; [`NodeState`] is its public rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum St {
    Pending,
    Waiting,
    Running,
    Backoff,
    /// A held attempt failed; its release runs before the next attempt.
    RetryRelease,
    Ready,
    /// A template that admits instances.
    Live,
    /// A template whose instances are being stopped.
    StoppingInstances,
    Finished,
    Failed,
    Interrupted,
    Ambiguous,
    Skipped,
    Releasing,
    Compensating,
    Stopping,
    Released,
    Compensated,
    Stopped,
    ReleaseFailed,
    Abandoned,
}

/// Per-node bookkeeping.
pub(super) struct Slot {
    pub st: St,
    pub attempt: u32,
    pub held: bool,
    /// An `Abort` or `Signal` was emitted for the in-flight body.
    pub cancelling: bool,
    /// A `Signal` was emitted: the body was asked to stop and kept running,
    /// so whatever it returns now is its answer to the cancel (C-60).
    pub signalled: bool,
    /// The abort was for a `within` deadline.
    pub timing_out: bool,
    /// A blocking attempt's `within` expired. The thread cannot be aborted
    /// (T7), so the attempt keeps its grants and the slot stays `Running`
    /// until the thread reports: the next attempt never overlaps it (INV-12)
    /// and the pool is never over-subscribed.
    pub timed_out: bool,
    pub timer: Option<TimerId>,
    pub backoff_until: Time,
    pub restarts: u32,
    /// FIFO position among waiters, taken when the node became need-ready.
    pub queued: Option<u64>,
    /// The earlier waiter whose unmet want refused this one its grant (T1's
    /// FIFO). Recorded so `Waiting{on}` can name the reason (contract § 2).
    pub blocked_by: Option<usize>,
    pub granted: bool,
    /// A body was spawned at least once (a component: its inner run began).
    pub started: bool,
    pub because: Option<usize>,
    /// Faults of attempts not yet exhausted: moved to the report if the node
    /// never becomes Ready, dropped (they stay in the trace) if it does.
    pub faults: Vec<Fault>,
    /// An ambiguous attempt's record, parked with the faults: reported if the
    /// node never becomes Ready, dropped if an `Ambiguity::Retry` resolves it.
    pub ambiguity: Option<NodeRecord>,
}

impl Slot {
    pub fn new() -> Slot {
        Slot {
            st: St::Pending,
            attempt: 0,
            held: false,
            cancelling: false,
            signalled: false,
            timing_out: false,
            timed_out: false,
            timer: None,
            backoff_until: Time::ZERO,
            restarts: 0,
            queued: None,
            blocked_by: None,
            granted: false,
            started: false,
            because: None,
            faults: Vec::new(),
            ambiguity: None,
        }
    }
}

/// A resource's arbitration state.
#[derive(Default)]
pub(super) struct Lock {
    pub exclusive: Option<usize>,
    pub shared: Vec<usize>,
}

/// What ended a scope's admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Cause {
    Fault(usize),
    Cancel,
    Shutdown,
    Finite,
    Terminal,
    /// The parent scope settled or reached this component in its cleanup;
    /// carries the parent's fault node when a fault did it.
    Parent(Option<usize>),
}

/// Per-scope bookkeeping.
pub(super) struct ScopeRun {
    pub st: RunState,
    pub cause: Option<Cause>,
    pub deadline: Option<Time>,
    pub budget_timer: Option<TimerId>,
    pub zero_timer: Option<TimerId>,
    /// The budget expired: anything still running is abandoned at once.
    pub spent: bool,
    pub pools: Vec<usize>,
    /// A node of this scope faulted, which is what an instance's own outcome
    /// is read from (the report's fault list is the whole run's).
    pub faulted: bool,
}

/// One template instance of a run.
pub(super) struct Instance {
    /// Its identity, minted by the host and echoed in every record.
    pub id: InstanceId,
    /// The template node it is an instance of.
    pub template: usize,
    /// The scope its nodes form.
    pub scope: usize,
    /// The instance the spawning body itself belongs to, if any: what the host
    /// needs to find the right slot table for a nested template.
    pub parent: Option<InstanceId>,
    /// `InstanceEnded` has been observed for it.
    pub ended: bool,
}

/// What a timer the machine set is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Purpose {
    Within(usize),
    Grace(usize),
    Backoff(usize),
    StopDeadline(usize),
    Budget(usize),
    Zero(usize),
}

/// The pure state machine. See the module docs.
pub struct Machine {
    pub(super) t: Table,
    pub(super) slots: Vec<Slot>,
    pub(super) locks: Vec<Lock>,
    pub(super) scopes: Vec<ScopeRun>,
    pub(super) rank: Vec<u32>,
    /// The schedule preference, kept so an instance's nodes are ranked the way
    /// the static graph's are (INV-14).
    pub(super) schedule: Vec<String>,
    pub(super) instances: Vec<Instance>,
    pub(super) now: Time,
    pub(super) timers: Vec<(TimerId, Purpose, Time)>,
    pub(super) next_timer: u64,
    pub(super) seq: u64,
    /// How many bodies the engine has started. `admit_all` uses it to know
    /// whether a pass made progress.
    pub(super) starts: u64,
    pub(super) fx: Vec<Effect>,
    pub(super) faults: Vec<Fault>,
    pub(super) cleanup_failures: Vec<Fault>,
    pub(super) incomplete: Vec<NodeRecord>,
    pub(super) ambiguous: Vec<NodeRecord>,
    pub(super) report: Option<Report<()>>,
}

impl Machine {
    pub(super) fn from_table(t: Table) -> Machine {
        let n = t.nodes.len();
        let scopes = t
            .scopes
            .iter()
            .map(|s| ScopeRun {
                st: RunState::Planned,
                cause: None,
                deadline: None,
                budget_timer: None,
                zero_timer: None,
                spent: false,
                pools: vec![0; s.pools.len()],
                faulted: false,
            })
            .collect();
        Machine {
            t,
            slots: (0..n).map(|_| Slot::new()).collect(),
            locks: (0..n).map(|_| Lock::default()).collect(),
            scopes,
            rank: vec![u32::MAX; n],
            schedule: Vec::new(),
            instances: Vec::new(),
            now: Time::ZERO,
            timers: Vec::new(),
            next_timer: 1,
            seq: 0,
            starts: 0,
            fx: Vec::new(),
            faults: Vec::new(),
            cleanup_failures: Vec::new(),
            incomplete: Vec::new(),
            ambiguous: Vec::new(),
            report: None,
        }
    }

    // ------------------------------------------------------------ helpers

    pub(super) fn order(&self, n: usize) -> RecordOrder {
        RecordOrder {
            steps: self.t.nodes[n].steps.clone(),
            attempt: self.slots[n].attempt.max(1),
        }
    }

    /// Write one observation about a node.
    pub(super) fn emit(&mut self, n: usize, kind: TraceKind) {
        let ev = TraceEvent::node(self.now, self.t.nodes[n].path.clone(), self.order(n), kind);
        self.fx.push(Effect::Emit(Box::new(ev)));
    }

    /// Write one observation about the run.
    pub(super) fn emit_run(&mut self, kind: TraceKind) {
        self.fx
            .push(Effect::Emit(Box::new(TraceEvent::at(self.now, kind))));
    }

    pub(super) fn fault(&self, n: usize, phase: Phase, kind: FaultKind) -> Fault {
        Fault {
            node: self.t.nodes[n].path.clone(),
            order: self.order(n),
            phase,
            kind,
        }
    }

    pub(super) fn record(&self, n: usize) -> NodeRecord {
        NodeRecord {
            node: self.t.nodes[n].path.clone(),
            order: self.order(n),
        }
    }

    pub(super) fn timer(&mut self, purpose: Purpose, at: Time) -> TimerId {
        let id = TimerId(self.next_timer);
        self.next_timer += 1;
        self.timers.push((id, purpose, at));
        self.fx.push(Effect::Timer { id, at });
        id
    }

    /// Forget a node's timer, telling the host to drop it.
    pub(super) fn drop_timer(&mut self, id: Option<TimerId>) {
        if let Some(id) = id {
            self.timers.retain(|(t, _, _)| *t != id);
            self.fx.push(Effect::CancelTimer(id));
        }
    }

    /// Whether the node's kind carries a release obligation once it held.
    pub(super) fn can_owe(&self, n: usize) -> bool {
        let node = &self.t.nodes[n];
        match node.kind {
            Kind::Resource => true,
            Kind::Effect => node.attrs.release != ReleaseStyle::Persistent,
            _ => false,
        }
    }

    pub(super) fn compensates_ambiguity(&self, n: usize) -> bool {
        let node = &self.t.nodes[n];
        node.kind == Kind::Effect
            && node.attrs.on_ambiguous == Some(Ambiguity::Compensate)
            && node.attrs.release != ReleaseStyle::Persistent
    }

    /// The outcome a scope's end reports.
    pub(super) fn outcome(&self, scope: usize) -> Outcome {
        match self.scopes[scope].cause {
            Some(Cause::Cancel) => Outcome::Cancelled,
            _ if self.faults.is_empty() => Outcome::Ok,
            _ => Outcome::Failed,
        }
    }
}

impl std::fmt::Debug for Machine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Machine")
            .field("run", &self.scopes[0].st)
            .field("now", &self.now)
            .field("nodes", &self.t.nodes.len())
            .finish()
    }
}
