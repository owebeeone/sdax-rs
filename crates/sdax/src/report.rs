//! What a run says about itself.
//!
//! Nothing the engine observes is dropped: every fault, cleanup failure,
//! panic, timeout, abandonment and ambiguity is a record here, and
//! [`Report::into_result`] is `Err` unless every list is empty (INV-9).
//!
//! **Record order (F4).** `faults`, `cleanup_failures`, `incomplete` and
//! `ambiguous` are ordered by [`RecordOrder`]: node declaration order, a
//! template's own record before its instances', instances by id, and attempts
//! within a node in attempt order. `trace` is **not** reordered — it is the
//! observation order, and re-sorting it would destroy what it records.
//! [`Report::sort`] establishes that order; producing a report already in it is
//! the engine's job (Stage 1).

use crate::contracts::{Error, Time};
use crate::cx::InstanceId;
use crate::view::NodePath;
use std::sync::Arc;

/// Where a record sits in the report's order.
///
/// `steps` is the path from the root plan to the node, one entry per level:
/// the node's declaration index, and the instance it belongs to when that
/// level is a template instance. Lexicographic comparison then gives exactly
/// the documented order, because `None` sorts before `Some`, so a template's
/// own record precedes its instances'.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct RecordOrder {
    /// One `(declaration index, instance)` pair per nesting level.
    pub steps: Vec<(u32, Option<InstanceId>)>,
    /// Which attempt of that node this record belongs to, counting from 1.
    pub attempt: u32,
}

/// Which body of a node a record is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Phase {
    /// `acquire` or `perform`.
    Prepare,
    /// `run`, or a service's `start`.
    Run,
    /// A service's serve future.
    Serve,
    /// A resource's `release`.
    ReleaseBody,
    /// An effect's `compensate`.
    Compensate,
    /// Stopping a service.
    Stop,
}

impl std::fmt::Display for Phase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Phase::Prepare => "prepare",
            Phase::Run => "run",
            Phase::Serve => "serve",
            Phase::ReleaseBody => "release",
            Phase::Compensate => "compensate",
            Phase::Stop => "stop",
        })
    }
}

/// A `Copy` summary of a fault, for traces and engine events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FaultLabel {
    /// The body returned `Err`.
    Error,
    /// The body panicked.
    Panic,
    /// The body exceeded `within` or a budget.
    Timeout,
    /// A start body dropped its context without returning `Serving`.
    NeverReady,
    /// A body registered twice in one attempt.
    DoubleHold,
}

/// What went wrong.
#[derive(Debug)]
pub enum FaultKind {
    /// The body returned `Err`. Typed errors survive as `downcast_ref` targets.
    Error(Error),
    /// The body panicked. The payload is carried, never re-raised by the engine.
    Panic(Box<dyn std::any::Any + Send>),
    /// The body exceeded `within` or the remaining budget.
    Timeout,
    /// A start body returned without a `Serving`.
    NeverReady,
    /// A body called `hold` more than once in one attempt.
    DoubleHold,
}

impl FaultKind {
    /// The `Copy` summary of this fault.
    pub fn label(&self) -> FaultLabel {
        match self {
            FaultKind::Error(_) => FaultLabel::Error,
            FaultKind::Panic(_) => FaultLabel::Panic,
            FaultKind::Timeout => FaultLabel::Timeout,
            FaultKind::NeverReady => FaultLabel::NeverReady,
            FaultKind::DoubleHold => FaultLabel::DoubleHold,
        }
    }
}

/// One thing that went wrong, and where.
#[derive(Debug)]
pub struct Fault {
    /// Which node.
    pub node: NodePath,
    /// Where it sits in the report's order.
    pub order: RecordOrder,
    /// Which body.
    pub phase: Phase,
    /// What happened.
    pub kind: FaultKind,
}

/// A node named in a report list that carries no error of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeRecord {
    /// Which node.
    pub node: NodePath,
    /// Where it sits in the report's order.
    pub order: RecordOrder,
}

/// How a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Outcome {
    /// Every node that started settled without a fault.
    Ok,
    /// At least one fault was recorded.
    Failed,
    /// An external `cancel()` or a drop ended the run before `End`.
    Cancelled,
}

impl std::fmt::Display for Outcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Outcome::Ok => "ok",
            Outcome::Failed => "failed",
            Outcome::Cancelled => "cancelled",
        })
    }
}

/// What one observation says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceKind {
    /// A body was spawned for this phase.
    Start(Phase),
    /// The body registered a value (no EOL event; the `iff-held` witness).
    Held,
    /// The node became ready.
    Ready,
    /// The body failed.
    Fail(Phase, FaultLabel),
    /// The engine cancelled the body; `held` decides whether a release runs.
    Interrupted {
        /// Whether a value had been registered.
        held: bool,
    },
    /// An effect was interrupted after starting and before `hold`.
    Ambiguous,
    /// The node never started.
    Skipped {
        /// Which node's outcome caused it, when one did. `None` when the run
        /// itself ended the node's eligibility — a `shutdown()`, a `cancel()`,
        /// a `Finite` scope reaching steady state, a `terminal` service
        /// finishing. Emitted either way: without the event a reader of a trace
        /// could not tell "never eligible" from "eligible and skipped by the
        /// request", and the state is otherwise readable only through
        /// `Machine::state`, which no report consumer sees.
        because: Option<NodePath>,
    },
    /// A service was asked to stop.
    StopRequested,
    /// A service stopped within its budget.
    Stopped,
    /// The shutdown budget expired while this obligation was still running.
    Abandoned,
    /// A release body started.
    ReleaseStart,
    /// A release body returned `Ok`.
    ReleaseOk,
    /// A release body failed.
    ReleaseFail(FaultLabel),
    /// A compensation started.
    CompensateStart,
    /// A compensation returned `Ok`.
    CompensateOk,
    /// A compensation failed.
    CompensateFail(FaultLabel),
    /// A template instance was created.
    InstanceSpawned(InstanceId),
    /// A template instance ended.
    InstanceEnded(InstanceId, Outcome),
    /// The run stopped admitting starts: a request, a fault under
    /// `FailFast`, a terminal service finishing, or `Steady` under `Finite`.
    /// The shutdown budget starts here (T7, INV-8).
    Settling,
    /// `Running` was dropped while the run was live.
    DroppedWhileRunning,
    /// A cancel or shutdown arrived while cleanup was already running; it is
    /// recorded and ignored (INV-7).
    RequestDuringCleanup,
    /// The runtime was dropped with live runs.
    RuntimeDroppedWithLiveRuns,
    /// An [`Observer`](crate::host::Observer) callback panicked and the driver
    /// caught it.
    ///
    /// The contract forbids it (§ 10), so this is a defect in the observer —
    /// but an unguarded panic on the driver's task would orphan every live
    /// body, which is exactly what INV-15 exists to prevent, so the driver
    /// contains it and says so here instead. The note sits immediately before
    /// the event whose delivery panicked, and never after `End` (T8); the
    /// event itself is still in the trace, and the run goes on.
    ObserverPanicked,
    /// The machine refused an event the driver fed it (D1): a driver bug,
    /// carrying the refusal and the event that drew it.
    ///
    /// Emitted by the driver, not by the machine — the machine's answer is
    /// [`Effect::Reject`](crate::host::engine::Effect::Reject), and what a
    /// driver does with it is the driver's own. A conforming driver never
    /// produces one.
    Rejected(String),
    /// The run ended.
    End(Outcome),
}

/// One observation, timestamped on the engine's clock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceEvent {
    /// When, on the engine's clock.
    pub at: Time,
    /// Which node, when the event is about one.
    pub node: Option<NodePath>,
    /// Where that node sits in the report's order.
    pub order: Option<RecordOrder>,
    /// What happened.
    pub kind: TraceKind,
}

impl TraceEvent {
    /// An event with no node attached.
    pub fn at(at: Time, kind: TraceKind) -> Self {
        TraceEvent {
            at,
            node: None,
            order: None,
            kind,
        }
    }

    /// An event about one node.
    pub fn node(at: Time, node: NodePath, order: RecordOrder, kind: TraceKind) -> Self {
        TraceEvent {
            at,
            node: Some(node),
            order: Some(order),
            kind,
        }
    }
}

/// Every observation of one run, in the order they were made.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Trace {
    /// The events, in observation order. Never re-sorted.
    pub events: Vec<TraceEvent>,
}

/// What a run says about itself.
#[must_use = "a report carries every fault the run observed; check it"]
#[derive(Debug)]
pub struct Report<Out = ()> {
    /// How the run ended.
    pub outcome: Outcome,
    /// The plan's exported value, if the run produced one.
    pub output: Option<Arc<Out>>,
    /// Faults from prepare, run and serve bodies.
    pub faults: Vec<Fault>,
    /// Faults from release, compensate and stop.
    pub cleanup_failures: Vec<Fault>,
    /// Obligations the shutdown budget abandoned.
    pub incomplete: Vec<NodeRecord>,
    /// Effects whose outcome is unknown (INV-11).
    pub ambiguous: Vec<NodeRecord>,
    /// The observations, if tracing was on.
    pub trace: Option<Trace>,
}

impl<Out> Report<Out> {
    /// An otherwise-empty report with this outcome.
    pub fn empty(outcome: Outcome) -> Self {
        Report {
            outcome,
            output: None,
            faults: Vec::new(),
            cleanup_failures: Vec::new(),
            incomplete: Vec::new(),
            ambiguous: Vec::new(),
            trace: None,
        }
    }

    /// Whether every list is empty. `Outcome::Ok` alone is not enough: an
    /// abandoned worker or an ambiguous effect leaves the run unclean.
    pub fn is_clean(&self) -> bool {
        self.faults.is_empty()
            && self.cleanup_failures.is_empty()
            && self.incomplete.is_empty()
            && self.ambiguous.is_empty()
            && self.outcome == Outcome::Ok
    }

    /// `Ok(output)` when the report is clean, and the whole report otherwise.
    ///
    /// The error *is* the report: boxing it to shrink the `Result` would hide
    /// the very thing a caller has to read.
    #[allow(clippy::result_large_err)]
    pub fn into_result(self) -> Result<Option<Arc<Out>>, Report<Out>> {
        if self.is_clean() {
            Ok(self.output)
        } else {
            Err(self)
        }
    }

    /// The panics the run caught, for a consumer that wants to resume
    /// unwinding. The engine never re-raises one itself.
    pub fn panics(&self) -> Vec<&Fault> {
        self.faults
            .iter()
            .chain(self.cleanup_failures.iter())
            .filter(|f| f.kind.label() == FaultLabel::Panic)
            .collect()
    }

    /// Put every record list into the documented order (F4). The trace is left
    /// exactly as observed.
    pub fn sort(&mut self) {
        self.faults.sort_by(|a, b| a.order.cmp(&b.order));
        self.cleanup_failures.sort_by(|a, b| a.order.cmp(&b.order));
        self.incomplete.sort_by(|a, b| a.order.cmp(&b.order));
        self.ambiguous.sort_by(|a, b| a.order.cmp(&b.order));
    }
}

impl<Out> std::fmt::Display for Report<Out> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "outcome: {}", self.outcome)?;
        for (label, list) in [
            ("faults", &self.faults),
            ("cleanup_failures", &self.cleanup_failures),
        ] {
            for x in list.iter() {
                writeln!(f, "{label}: {} ({}) {:?}", x.node, x.phase, x.kind.label())?;
            }
        }
        for (label, list) in [
            ("incomplete", &self.incomplete),
            ("ambiguous", &self.ambiguous),
        ] {
            let names: Vec<String> = list.iter().map(|r| r.node.to_string()).collect();
            if !names.is_empty() {
                writeln!(f, "{label}: {{{}}}", names.join(", "))?;
            }
        }
        Ok(())
    }
}
