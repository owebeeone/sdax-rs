//! The engine's vocabulary: what happens to a run, and what the host must do
//! about it.
//!
//! The semantics of a plan is a state machine over these two types: the host
//! feeds it [`Event`]s and performs the [`Effect`]s it returns. Both are
//! public and their names are stable, because a scripted driver, a tokio
//! adapter and a trace checker all speak them.
//!
//! **Stage 0 has no machine.** The types are here so the vocabulary is fixed
//! and reviewable before the machine is written; `Machine::step` is Stage 1,
//! and no stub of it exists — a stub would be a claim this crate cannot make.

use crate::cx::InstanceId;
use crate::key::RawKey;
use crate::report::{FaultLabel, Outcome, TraceEvent};
use crate::Time;

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
    /// A body was spawned and has been polled at least once.
    Started(RawKey),
    /// The body registered a value through `hold` (T2).
    Held(RawKey),
    /// The body returned `Ok`.
    NodeOk(RawKey),
    /// The body returned `Err`, panicked or timed out.
    NodeErr(RawKey, FaultLabel),
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
        /// Whether it returned `Ok`.
        ok: bool,
    },
    /// A timer the machine asked for has fired.
    Timer(TimerId),
    /// `shutdown()` was called.
    ShutdownRequested,
    /// `cancel()` was called, or `Running` was dropped.
    CancelRequested,
    /// A body instantiated a template.
    InstanceSpawned {
        /// Which template.
        template: RawKey,
        /// The new instance.
        id: InstanceId,
    },
    /// An instance's own run ended.
    InstanceEnded {
        /// Which instance.
        id: InstanceId,
        /// How it ended.
        outcome: Outcome,
    },
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
    /// Abort a node's task. The host must still join it.
    Abort(RawKey),
    /// Raise the node's stop signal without aborting it.
    Signal(RawKey),
    /// Run a resource's release body.
    Release(RawKey),
    /// Run an effect's compensation.
    Compensate(RawKey),
    /// Signal a service, wait up to its budget, then abort.
    StopService(RawKey),
    /// Set a timer on the engine's clock.
    Timer {
        /// Its identity, echoed back by [`Event::Timer`].
        id: TimerId,
        /// When it should fire.
        at: Time,
    },
    /// Instantiate a template.
    SpawnInstance {
        /// Which template.
        template: RawKey,
        /// The instance to create.
        id: InstanceId,
    },
    /// Hand an observation to the observer.
    Emit(Box<TraceEvent>),
    /// The run is over.
    End(Outcome),
}
