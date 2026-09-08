//! The engine's vocabulary: what happens to a run, and what the host must do
//! about it.
//!
//! The semantics of a plan is a state machine over these two types: the host
//! feeds it [`Event`]s and performs the [`Effect`]s it returns. Both are
//! public and their names are stable, because a scripted driver, a tokio
//! adapter and a trace checker all speak them.
//!
//! **The machine.** [`Machine`] is the pure state machine that gives every
//! declaration its run-time meaning (contract § 2–4, T1–T8). It owns no clock
//! and no task: the host tells it the time ([`Machine::advance`]) and what
//! happened ([`Machine::step`]), and performs what it returns. Every deadline
//! is a [`Effect::Timer`] the host sets and echoes back, so the machine is
//! deterministic given a script and a schedule (INV-14).
//!
//! From Stage 3 the machine also runs templates and dynamic instances: a
//! body's `cx.spawn` reaches it as [`Event::InstanceSpawned`], the host is
//! told to open the instance's slots by [`Effect::SpawnInstance`], and the
//! instance's own scope is admitted, settled and cleaned up as one unit
//! (INV-16).
//!
//! This module is part of [`host`](crate::host) and not of the author API: a
//! plan is declared, validated and inspected without ever naming an `Event` or
//! an `Effect`.

use crate::contracts::Time;
use crate::cx::InstanceId;
use crate::key::RawKey;
use crate::report::{FaultKind, Outcome, TraceEvent};

mod admit;
mod cleanup;
mod compaction;
mod exits;
mod faults;
mod history;
mod instances;
mod machine;
mod publication;
mod settle;
mod state;
mod table;

pub use instances::SpawnTable;
pub use state::{EngineError, Machine, NodeState, RunState};

/// Identity of one timer the host was asked to set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TimerId(pub u64);

/// How a task the engine spawned ended, as a `Copy` label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JoinedLabel {
    /// The future returned.
    Done,
    /// The task was aborted before it returned.
    Cancelled,
    /// The task panicked.
    Panicked,
}

/// Something that happened, told to the machine.
#[derive(Debug)]
pub enum Event {
    /// The host completed a structural value transfer requested by
    /// [`Effect::PublishReady`]. This is not a body event.
    ReadyPublished {
        /// Structural node's run key.
        node: RawKey,
        /// Whether its value was installed before any dependent can start.
        result: Result<(), crate::Error>,
    },
    /// A body was spawned and has been polled at least once.
    Started(RawKey),
    /// The body registered a value through `hold` (T2).
    Held(RawKey),
    /// The body returned `Ok`.
    NodeOk(RawKey),
    /// The body returned `Err`, panicked or timed out. The fault is carried
    /// whole so the report can hold the error or the panic payload.
    NodeErr(RawKey, FaultKind),
    /// The body was cancelled; `held` decides whether a release is owed.
    NodeCancelled {
        /// Which node.
        node: RawKey,
        /// Whether a value had been registered.
        held: bool,
    },
    /// A service's serve future returned.
    ServeEnded {
        /// Which service.
        node: RawKey,
        /// `None` if it returned `Ok`; otherwise what went wrong.
        fault: Option<FaultKind>,
    },
    /// A timer the machine asked for has fired.
    Timer(TimerId),
    /// `shutdown()` was called.
    ShutdownRequested,
    /// `cancel()` was called, or `Running` was dropped.
    CancelRequested,
    /// A body instantiated a template (`cx.spawn`).
    InstanceSpawned {
        /// The node whose body spawned it, so the template is resolved in the
        /// scope that declared it.
        spawner: RawKey,
        /// The template's **declaration** key, as a `Template` handle carries
        /// it.
        template: RawKey,
        /// The identity the host minted for the new instance.
        id: InstanceId,
    },
    /// `Child::stop()`: this instance is asked to stop. How it *ends* is the
    /// machine's own observation ([`TraceKind::InstanceEnded`](crate::TraceKind::InstanceEnded)), never an
    /// input (`OD-INSTANCE-EVENTS`).
    StopInstance(InstanceId),
    /// A task the engine aborted has been joined (T5: the engine joins before
    /// treating a node as settled).
    TaskJoined {
        /// Which node's task.
        node: RawKey,
        /// How it ended.
        joined: JoinedLabel,
    },
}

/// Something the host must do, returned by the machine.
#[derive(Debug)]
pub enum Effect {
    /// Install a join or component value through `BodySource::publish_ready`.
    /// Finish this effects batch, then acknowledge with [`Event::ReadyPublished`]
    /// before external events or readiness snapshots. No body is spawned.
    PublishReady {
        /// Structural node's run key; resolve its declaration and instance.
        node: RawKey,
    },
    /// Spawn an async body.
    Spawn {
        /// Which node.
        node: RawKey,
        /// Which attempt.
        attempt: u32,
    },
    /// Run a blocking body on its pool.
    SpawnBlocking {
        /// Which node.
        node: RawKey,
        /// Which attempt.
        attempt: u32,
    },
    /// Start one serving episode against the already-published service handle.
    Serve {
        /// Which service.
        node: RawKey,
        /// The serving episode, counting from 1.
        episode: u32,
    },
    /// Abort a node's task. The host must still join it.
    Abort(RawKey),
    /// Raise the node's stop signal without aborting it.
    Signal(RawKey),
    /// Run a resource's release body.
    Release(RawKey),
    /// Run an effect's compensation.
    Compensate(RawKey),
    /// Recover an unknown effect without a success receipt.
    Recover(RawKey),
    /// Signal a service, wait up to its budget, then abort.
    StopService(RawKey),
    /// Refresh the deadline visible to an already-running shielded cleanup.
    /// This changes context metadata only; it does not signal or abort.
    RefreshDeadline(RawKey),
    /// Set a timer on the engine's clock.
    Timer {
        /// Its identity, echoed back by [`Event::Timer`].
        id: TimerId,
        /// When it should fire.
        at: Time,
    },
    /// Forget a timer the machine no longer needs (a backoff cut short by a
    /// cancel, a deadline that no longer applies). Firing it anyway is
    /// harmless: the machine ignores a timer it has forgotten.
    CancelTimer(TimerId),
    /// Open one instance of a template: the host makes its slot table and
    /// puts the per-instance input in it, before any body of the instance is
    /// spawned.
    SpawnInstance {
        /// The template's **declaration** key, which addresses its bodies.
        template: RawKey,
        /// The instance the spawning body itself belongs to, if any: a nested
        /// template's bodies live in *that* instance's tables.
        parent: Option<InstanceId>,
        /// The instance to create.
        id: InstanceId,
    },
    /// Hand an observation to the observer.
    Emit(Box<TraceEvent>),
    /// The run is over.
    End(Outcome),
    /// The event made no sense here (an unknown node, a body that is not in
    /// flight, anything after `End`). The machine never panics on input; it
    /// says so instead (D1 totality).
    Reject(Rejected),
}

/// An event the machine refused, and why.
#[derive(Debug)]
pub struct Rejected {
    /// The event as received, rendered.
    pub event: String,
    /// Why it was refused.
    pub reason: &'static str,
}

pub(crate) use table::Table;
pub(crate) fn compile_layout(
    ir: &crate::plan::PlanIr,
) -> Result<std::sync::Arc<Table>, EngineError> {
    Table::build(ir).map(std::sync::Arc::new)
}

#[cfg(test)]
#[path = "engine/compaction_tests.rs"]
mod compaction_tests;
