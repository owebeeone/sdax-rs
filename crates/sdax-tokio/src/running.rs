//! `Plan::start`, the [`Running`] handle, and what a watcher may read of a
//! live run.
//!
//! `start` is **lazy**: nothing is spawned until the returned [`Running`] is
//! first polled, so a `cancel()` before that reaches the machine while the run
//! is still `Planned` and ends it with no effect at all (C-11).
//!
//! Dropping a live `Running` cancels it and leaves the driver task — already
//! tracked by the runtime — to abort the bodies, join them and run the release
//! graph inside the shutdown budget. That is the drainer: one engine-owned
//! task, not a detached one (C-14, INV-15).

use crate::body::{Msg, Tx};
use crate::driver::{Driver, Finished};
use sdax::host::engine::{EngineError, Event, Machine, NodeState, RunState};
use sdax::host::sim::SimStep;
use sdax::host::{RawKey, Runtime, StopSignal, Time};
use sdax::{
    Fault, FaultKind, NodePath, Outcome, Phase, Plan, Reason, RecordOrder, Report, Schedule, Stop,
};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use tokio::sync::mpsc::unbounded_channel;
use tokio::sync::oneshot;

/// One `why` answer recorded at a time, for a harness that reads a run back.
pub type WhyAt = (Time, String, Vec<(String, Reason)>);

/// Everything a harness wants to see of a run, recorded as it happens.
///
/// Off by default: it costs one `Debug` rendering per effect and one state
/// read per node per step. [`RunOptions::record`] turns it on.
#[derive(Default)]
pub struct RunRecord {
    /// Every event fed and every effect returned, rendered, in order.
    pub steps: Vec<SimStep>,
    /// A `why` answer per waiting node after every step.
    pub whys: Vec<WhyAt>,
    /// Every event the machine refused (D1). A non-empty list is a driver bug.
    pub rejections: Vec<String>,
}

/// Where a run is, as an outside observer may read it.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// The engine clock reading at the last step.
    pub at: Time,
    /// The root scope's run state.
    pub run: RunState,
    /// How many nodes the driver has a task or a context for.
    pub live: usize,
    /// Every node's state — empty unless [`RunOptions::observe_states`] asked
    /// for it, because it costs one read per node per step.
    pub nodes: Vec<(NodePath, NodeState)>,
}

/// The shared side of a run: the request channel, the readiness latch, and the
/// last published snapshot.
pub(crate) struct Control {
    tx: Tx,
    states: bool,
    snap: Mutex<Snapshot>,
    ready_sig: Arc<StopSignal>,
    ready_state: Mutex<Option<Result<(), Outcome>>>,
    record: Option<Arc<Mutex<RunRecord>>>,
}

impl Control {
    pub(crate) fn publish(&self, m: &Machine, paths: &[(RawKey, NodePath)], live: usize) {
        let nodes = if self.states {
            paths
                .iter()
                .filter_map(|(_, p)| m.state_of(&p.to_string()).map(|s| (p.clone(), s)))
                .collect()
        } else {
            Vec::new()
        };
        *self.snap.lock().expect("snapshot poisoned") = Snapshot {
            at: m.now(),
            run: m.run_state(),
            live,
            nodes,
        };
        if m.run_state() == RunState::Steady {
            self.latch(Ok(()));
        }
    }

    pub(crate) fn ended(&self, outcome: Outcome) {
        self.latch(Err(outcome));
    }

    fn latch(&self, v: Result<(), Outcome>) {
        let mut st = self.ready_state.lock().expect("ready poisoned");
        if st.is_none() {
            *st = Some(v);
            self.ready_sig.request();
        }
    }

    /// D1: the machine refused an event the driver fed it. That is a driver
    /// bug, so it is never swallowed — recorded when a harness is listening,
    /// and on stderr when none is.
    pub(crate) fn reject(&self, what: String) {
        match &self.record {
            Some(r) => r.lock().expect("record poisoned").rejections.push(what),
            None => eprintln!("sdax-tokio: the machine refused an event: {what}"),
        }
    }

    fn send(&self, ev: Event) {
        let _ = self.tx.send(Msg::Request(ev));
    }
}

/// How one run is started.
#[derive(Default)]
pub struct RunOptions {
    pub(crate) bodies: Option<Arc<dyn sdax::host::BodySource>>,
    pub(crate) record: Option<Arc<Mutex<RunRecord>>>,
    pub(crate) schedule: Schedule,
    pub(crate) states: bool,
}

impl RunOptions {
    /// Defaults: the plan's own bodies, no recording, declaration-order
    /// preference among simultaneously eligible nodes.
    pub fn new() -> RunOptions {
        RunOptions::default()
    }

    /// Run these bodies instead of the plan's own.
    ///
    /// The machine is unchanged either way; this is how a harness drives the
    /// real adapter from a `Script`.
    pub fn bodies(mut self, src: Arc<dyn sdax::host::BodySource>) -> RunOptions {
        self.bodies = Some(src);
        self
    }

    /// Record every step, `why` answer and rejection into this sink.
    pub fn record(mut self, sink: Arc<Mutex<RunRecord>>) -> RunOptions {
        self.record = Some(sink);
        self
    }

    /// Prefer these nodes among ones that become eligible together (T1).
    pub fn schedule(mut self, s: Schedule) -> RunOptions {
        self.schedule = s;
        self
    }

    /// Publish every node's state in each [`Snapshot`].
    pub fn observe_states(mut self) -> RunOptions {
        self.states = true;
        self
    }
}

/// A run, as something other than its awaiter sees it.
#[derive(Clone)]
pub struct RunHandle {
    ctl: Arc<Control>,
}

impl RunHandle {
    /// Ask the run to shut down.
    pub fn shutdown(&self) {
        self.ctl.send(Event::ShutdownRequested);
    }

    /// Ask the run to cancel.
    pub fn cancel(&self) {
        self.ctl.send(Event::CancelRequested);
    }

    /// Where the run was at its last step.
    pub fn snapshot(&self) -> Snapshot {
        self.ctl.snap.lock().expect("snapshot poisoned").clone()
    }

    /// Wait until the run reaches steady state, or ends first.
    pub async fn ready(&self) -> Result<(), Outcome> {
        Stop::on(self.ctl.ready_sig.clone()).await;
        self.ctl
            .ready_state
            .lock()
            .expect("ready poisoned")
            .unwrap_or(Ok(()))
    }
}

/// What the first poll of a [`Running`] has to do.
type Launch = Box<dyn FnOnce(Arc<Control>) -> oneshot::Receiver<Finished> + Send>;

enum State {
    /// Nothing has been spawned; `cancel()` here costs nothing.
    Idle(Launch),
    /// The driver task is running.
    Live(oneshot::Receiver<Finished>),
    /// The report has been taken.
    Done,
}

/// A live run.
///
/// A `Future` whose output is the [`Report`]. Dropping it before it resolves
/// cancels the run and leaves the drainer to finish the release graph; the
/// report still reaches the [`Observer`](sdax::host::Observer).
#[must_use = "a run does nothing until the Running handle is polled; dropping it cancels the run"]
pub struct Running<Out> {
    state: State,
    ctl: Arc<Control>,
    refused: Option<(String, EngineError)>,
    _out: std::marker::PhantomData<fn() -> Arc<Out>>,
}

impl<Out: Send + Sync + 'static> Running<Out> {
    /// Ask the run to shut down: a normal end from wherever it is.
    pub fn shutdown(&self) {
        self.ctl.send(Event::ShutdownRequested);
    }

    /// Ask the run to cancel: in-flight bodies are interrupted, the release
    /// graph still runs (INV-7).
    pub fn cancel(&self) {
        self.ctl.send(Event::CancelRequested);
    }

    /// Wait until every node of the root scope is settled — steady state —
    /// or until the run ends before it gets there.
    ///
    /// `Err(outcome)` is the run having ended first, which is an answer, not a
    /// failure of the call.
    ///
    /// Takes `&mut self` because it *starts* the run if nothing has yet: a
    /// handle nobody ever polls never spawns anything, and waiting for the
    /// readiness of a run that was never started would wait forever.
    pub async fn ready(&mut self) -> Result<(), Outcome> {
        self.launch();
        Stop::on(self.ctl.ready_sig.clone()).await;
        self.ctl
            .ready_state
            .lock()
            .expect("ready poisoned")
            .unwrap_or(Ok(()))
    }

    /// Where the run was at its last step.
    pub fn snapshot(&self) -> Snapshot {
        self.ctl.snap.lock().expect("snapshot poisoned").clone()
    }

    /// A clonable handle for asking this run to stop from somewhere else.
    ///
    /// [`Running`] is a future and is consumed by awaiting it; a supervisor
    /// that wants to call `shutdown()` while something else awaits the report
    /// holds one of these instead.
    pub fn handle(&self) -> RunHandle {
        RunHandle {
            ctl: self.ctl.clone(),
        }
    }

    /// Spawn the driver task if it has not been spawned. Idempotent.
    fn launch(&mut self) {
        if matches!(self.state, State::Idle(_)) && self.refused.is_none() {
            let State::Idle(launch) = std::mem::replace(&mut self.state, State::Done) else {
                unreachable!("just matched")
            };
            self.state = State::Live(launch(self.ctl.clone()));
        }
    }

    fn refusal(&mut self) -> Report<Out> {
        let (plan, e) = self.refused.take().expect("refused");
        let mut r: Report<Out> = Report::empty(Outcome::Failed);
        r.faults.push(Fault {
            node: NodePath::root(&plan),
            order: RecordOrder {
                steps: Vec::new(),
                attempt: 1,
            },
            phase: Phase::Prepare,
            kind: FaultKind::Error(Box::new(e)),
        });
        r
    }
}

impl<Out: Send + Sync + 'static> Future for Running<Out> {
    type Output = Report<Out>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Report<Out>> {
        let me = self.get_mut();
        if me.refused.is_some() {
            me.state = State::Done;
            return Poll::Ready(me.refusal());
        }
        me.launch();
        match &mut me.state {
            State::Idle(_) => unreachable!("launch leaves Idle"),
            State::Live(rx) => match Pin::new(rx).poll(cx) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(Ok((report, out))) => {
                    me.state = State::Done;
                    Poll::Ready(with_output(report, out))
                }
                // The driver task went away without sending: the only way is a
                // panic in the driver itself, which is a bug in this crate and
                // is not dressed up as a clean run.
                Poll::Ready(Err(_)) => {
                    me.state = State::Done;
                    Poll::Ready(Report::empty(Outcome::Cancelled))
                }
            },
            State::Done => panic!("a Running is polled once, to completion"),
        }
    }
}

fn with_output<Out: Send + Sync + 'static>(
    r: Report<()>,
    out: Option<Box<dyn std::any::Any + Send + Sync>>,
) -> Report<Out> {
    Report {
        outcome: r.outcome,
        output: out.and_then(|b| b.downcast::<Arc<Out>>().ok()).map(|b| *b),
        faults: r.faults,
        cleanup_failures: r.cleanup_failures,
        incomplete: r.incomplete,
        ambiguous: r.ambiguous,
        trace: r.trace,
    }
}

impl<Out> Drop for Running<Out> {
    fn drop(&mut self) {
        if matches!(self.state, State::Live(_)) {
            // The drainer is the driver task itself: it is already spawned and
            // tracked, so the cancel it is about to receive runs the release
            // graph inside the shutdown budget with nothing detached.
            let _ = self.ctl.tx.send(Msg::Dropped);
        }
    }
}

/// `Plan::start`: the run driver, on a runtime of your choosing.
///
/// An extension trait rather than an inherent method because the driver lives
/// here and `sdax` has no dependencies at all — not even on this crate. The
/// call site reads the same: `use sdax_tokio::PlanStart;` then
/// `plan.start(rt)`.
pub trait PlanStart<Out> {
    /// Start a run, refusing a plan the machine cannot run before anything is
    /// spawned (`L-IMPORTS`, and templates until Stage 3).
    fn try_start<R: Runtime>(&self, rt: Arc<R>) -> Result<Running<Out>, EngineError>;

    /// Start a run with options: another body source, a record sink, a
    /// schedule preference.
    fn try_start_with<R: Runtime>(
        &self,
        rt: Arc<R>,
        opts: RunOptions,
    ) -> Result<Running<Out>, EngineError>;

    /// Start a run. A refused plan is not a panic and not a silent no-op: the
    /// handle resolves to a `Failed` report carrying the refusal.
    fn start<R: Runtime>(&self, rt: Arc<R>) -> Running<Out>;

    /// [`start`](Self::start) with options.
    fn start_with<R: Runtime>(&self, rt: Arc<R>, opts: RunOptions) -> Running<Out>;
}

impl<Out: Send + Sync + 'static> PlanStart<Out> for Plan<Out> {
    fn try_start<R: Runtime>(&self, rt: Arc<R>) -> Result<Running<Out>, EngineError> {
        self.try_start_with(rt, RunOptions::new())
    }

    fn try_start_with<R: Runtime>(
        &self,
        rt: Arc<R>,
        opts: RunOptions,
    ) -> Result<Running<Out>, EngineError> {
        let machine = Machine::new(self)?.with_schedule(&opts.schedule);
        let src = opts
            .bodies
            .unwrap_or_else(|| sdax::host::bodies_of::<Out, ()>(self));
        let (tx, rx) = unbounded_channel();
        let (done_tx, done_rx) = oneshot::channel();
        let record = opts.record.clone();
        let ctl = Arc::new(Control {
            tx: tx.clone(),
            states: opts.states,
            snap: Mutex::new(Snapshot {
                at: Time::ZERO,
                run: RunState::Planned,
                live: 0,
                nodes: Vec::new(),
            }),
            ready_sig: StopSignal::new(),
            ready_state: Mutex::new(None),
            record: record.clone(),
        });
        let launch: Launch = Box::new(move |ctl: Arc<Control>| {
            let driver = Driver::new(rt.clone(), machine, src, tx, rx, ctl, record, done_tx);
            rt.spawn(Box::pin(driver.run()));
            done_rx
        });
        Ok(Running {
            state: State::Idle(launch),
            ctl,
            refused: None,
            _out: std::marker::PhantomData,
        })
    }

    fn start<R: Runtime>(&self, rt: Arc<R>) -> Running<Out> {
        self.start_with(rt, RunOptions::new())
    }

    fn start_with<R: Runtime>(&self, rt: Arc<R>, opts: RunOptions) -> Running<Out> {
        let name = self.name().to_string();
        match self.try_start_with(rt, opts) {
            Ok(r) => r,
            Err(e) => Running {
                state: State::Done,
                ctl: Arc::new(Control {
                    tx: unbounded_channel().0,
                    states: false,
                    snap: Mutex::new(Snapshot {
                        at: Time::ZERO,
                        run: RunState::Planned,
                        live: 0,
                        nodes: Vec::new(),
                    }),
                    ready_sig: StopSignal::new(),
                    ready_state: Mutex::new(None),
                    record: None,
                }),
                refused: Some((name, e)),
                _out: std::marker::PhantomData,
            },
        }
    }
}
