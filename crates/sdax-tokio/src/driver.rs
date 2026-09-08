//! The run driver: a loop around [`Machine::step`] that performs every effect
//! through the [`Runtime`] port and feeds the events back.
//!
//! The shape is the stepping simulator's (`sdax::host::sim`), with the virtual
//! queue replaced by a real task set, real timers and a real request channel.
//! The machine is single-threaded and lives here, behind one task; effects are
//! performed in the order returned and never reordered.

use crate::body::{blocking_job, join_aborted, run_body, run_serve, Msg, Tx};
use crate::running::{Control, RunRecord};
use crate::scope::RunScope;
use sdax::host::engine::{Effect, Event, Machine, TimerId};
use sdax::host::sim::SimStep;
use sdax::host::{BodySource, Clock, CxInner, InstanceId, RawKey, Runtime, Task, TaskHandle, Time};
use sdax::{FaultKind, Kind, NodePath, Outcome, Report, Trace, TraceKind};
use std::any::Any;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::oneshot;

mod observer;

/// A body error the driver itself raises.
#[derive(Debug)]
struct DriverError(&'static str);
impl std::fmt::Display for DriverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for DriverError {}

/// One node's current attempt: the context its body holds, and the task it
/// runs in. Exactly one per node — a superseded attempt's handle is dropped
/// with it, and its messages are refused by epoch.
struct Live<H> {
    epoch: u64,
    cx: Arc<CxInner>,
    handle: Option<H>,
}

/// What a finished run hands back: the machine's report, and the exported
/// value still erased (only `Running<Out>` knows `Out`).
pub(crate) type Finished = (Report<()>, Option<Box<dyn Any + Send + Sync>>);

/// The clock of a [`Runtime`], as an owned handle a body context can hold.
struct RtClock<R>(Arc<R>);

impl<R: Runtime> Clock for RtClock<R> {
    fn now(&self) -> Time {
        self.0.clock().now()
    }
    fn sleep(&self, d: std::time::Duration) -> sdax::host::BoxFuture<'static, ()> {
        self.0.clock().sleep(d)
    }
}

pub(crate) struct Driver<R: Runtime> {
    rt: Arc<R>,
    clock: Arc<dyn Clock>,
    machine: Machine,
    src: Arc<dyn BodySource>,
    tx: Tx,
    rx: UnboundedReceiver<Msg>,
    live: HashMap<RawKey, Live<R::Task>>,
    kinds: HashMap<RawKey, Kind>,
    /// The declaration a run key was made from, and the instance it belongs
    /// to: what a `BodySource` is addressed by. An instance's nodes join it
    /// when the instance is opened.
    origins: HashMap<RawKey, (RawKey, Option<InstanceId>)>,
    paths: Vec<(RawKey, NodePath)>,
    scope: Arc<RunScope>,
    timers: HashMap<TimerId, R::Task>,
    trace: Trace,
    ctl: Arc<Control>,
    record: Option<Arc<Mutex<RunRecord>>>,
    done: Option<oneshot::Sender<Finished>>,
    epoch: u64,
    ended: Option<Outcome>,
    publications: VecDeque<Event>,
}

impl<R: Runtime> Driver<R> {
    // Eight fields, all of them the run's own state; a `DriverParts` struct
    // would be the same eight names one indirection further away.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        rt: Arc<R>,
        machine: Machine,
        src: Arc<dyn BodySource>,
        tx: Tx,
        rx: UnboundedReceiver<Msg>,
        ctl: Arc<Control>,
        record: Option<Arc<Mutex<RunRecord>>>,
        done: oneshot::Sender<Finished>,
        scope: Arc<RunScope>,
    ) -> Self {
        let kinds = machine.nodes().iter().map(|(k, _, n)| (*k, *n)).collect();
        let origins = machine
            .nodes()
            .iter()
            .map(|(k, _, _)| (*k, (*k, None)))
            .collect();
        let paths = machine
            .nodes()
            .iter()
            .map(|(k, p, _)| (*k, p.clone()))
            .collect();
        Driver {
            clock: Arc::new(RtClock(rt.clone())),
            rt,
            machine,
            src,
            tx,
            rx,
            live: HashMap::new(),
            kinds,
            origins,
            paths,
            scope,
            timers: HashMap::new(),
            trace: Trace::default(),
            ctl,
            record,
            done: Some(done),
            epoch: 0,
            ended: None,
            publications: VecDeque::new(),
        }
    }

    /// Drive the run to `End`.
    pub(crate) async fn run(mut self) {
        // Requests that arrived before the first poll precede the ship
        // boundary, so `cancel()` before anything started ends the run with no
        // effect at all (C-11).
        let mut pending = Vec::new();
        while let Ok(m) = self.rx.try_recv() {
            pending.push(m);
        }
        for m in pending {
            self.handle(m);
        }
        if self.ended.is_none() {
            let fx = self.machine.begin();
            self.after("begin".to_string(), fx);
        }
        while self.ended.is_none() {
            let Some(m) = self.rx.recv().await else { break };
            self.handle(m);
        }
        self.finish();
    }

    /// One message: filter it, keep the driver's own books, feed the machine.
    fn handle(&mut self, m: Msg) {
        // The run is over. A late message is not a driver bug — a request can
        // be made while the last release is finishing, and a body task's
        // outcome can be queued behind the `End` — so it is dropped rather
        // than fed to a machine that would rightly refuse it (D1).
        if self.ended.is_some() {
            return;
        }
        let ev = match m {
            Msg::Timer(id) => Event::Timer(id),
            Msg::Request(ev) => ev,
            Msg::Dropped => {
                self.emit_dropped();
                Event::CancelRequested
            }
            Msg::Body { node, epoch, ev } => {
                // Ordering rule 2: a superseded attempt's outcome maps to
                // nothing, so it is dropped rather than credited to the
                // attempt that replaced it.
                if self.live.get(&node).map(|l| l.epoch) != Some(epoch) {
                    return;
                }
                match self.settled(node, ev) {
                    Some(ev) => ev,
                    None => return,
                }
            }
        };
        self.step(ev);
    }

    /// A body outcome: bank what it registered before the machine acts on it.
    fn settled(&mut self, node: RawKey, ev: Event) -> Option<Event> {
        let terminal = matches!(
            ev,
            Event::NodeOk(_) | Event::NodeErr(..) | Event::NodeCancelled { .. }
        );
        if !terminal {
            return Some(ev);
        }
        let Some(live) = self.live.get(&node) else {
            return Some(ev);
        };
        let cx = live.cx.clone();
        if let Some(v) = cx.take_held() {
            // INV-3: taken whatever the outcome, so a held-then-failed attempt
            // still has a value for its release to discharge.
            let (decl, inst) = self.origin(node);
            self.src.store(decl, inst, v);
        }
        Some(ev)
    }

    /// Start one explicit serving episode with a fresh stop context.
    fn start_serving(&mut self, node: RawKey, episode: u32) {
        self.epoch += 1;
        let epoch = self.epoch;
        let mut cx = CxInner::new(node, self.clock.clone())
            .with_episode(episode)
            .with_scope(self.scope.clone());
        if let Some(at) = self.machine.deadline_for(node) {
            cx = cx.with_deadline(at);
        }
        let tx = self.tx.clone();
        let (decl, inst) = self.origin(node);
        let handle = match self.src.serve(decl, inst, &cx) {
            Some(serve) => Some(self.rt.spawn(Box::pin(run_serve(tx, node, epoch, serve)))),
            None => {
                let _ = tx.send(Msg::Body {
                    node,
                    epoch,
                    ev: Event::ServeEnded {
                        node,
                        fault: Some(FaultKind::Error(Box::new(DriverError(
                            "the body source has no serving factory for this service",
                        )))),
                    },
                });
                None
            }
        };
        self.live.insert(node, Live { epoch, cx, handle });
    }

    fn step(&mut self, ev: Event) {
        self.machine.advance(self.clock.now());
        let described = format!("{ev:?}");
        let fx = self.machine.step(ev);
        self.after(described, fx);
    }

    /// Perform an effect list in order, then publish what a watcher may read.
    fn after(&mut self, event: String, fx: Vec<Effect>) {
        self.perform_batch(event, fx);
        while let Some(ack) = self.publications.pop_front() {
            let described = format!("{ack:?}");
            let effects = self.machine.step(ack);
            self.perform_batch(described, effects);
        }
        self.ctl
            .publish(&self.machine, &self.paths, self.live.len());
        // All host publication obligations settle before bodies can observe
        // readiness through the gate or child latches.
        self.scope.publish(self.machine.spawn_table());
        self.scope.resolve(&self.machine.instances());
    }

    fn perform_batch(&mut self, event: String, fx: Vec<Effect>) {
        let rendered: Vec<String> = if self.record.is_some() {
            fx.iter().map(|e| format!("{e:?}")).collect()
        } else {
            Vec::new()
        };
        for e in fx {
            self.perform(e);
        }
        let at = self.machine.now();
        if let Some(rec) = &self.record {
            let mut r = rec.lock().expect("record poisoned");
            r.steps.push(SimStep {
                at,
                event,
                effects: rendered,
            });
            for (_, path) in &self.paths {
                let p = path.to_string();
                if let Some(sdax::host::engine::NodeState::Waiting { on }) =
                    self.machine.state_of(&p)
                {
                    let on = on.into_iter().map(|(q, r)| (q.to_string(), r)).collect();
                    r.whys.push((at, p, on));
                }
            }
        }
    }

    /// The declaration key and instance behind a run key.
    fn origin(&self, node: RawKey) -> (RawKey, Option<InstanceId>) {
        self.origins.get(&node).copied().unwrap_or((node, None))
    }

    fn perform(&mut self, e: Effect) {
        match e {
            Effect::PublishReady { node } => {
                let (declaration, instance) = self.origin(node);
                let result = self.src.publish_ready(declaration, instance);
                self.publications
                    .push_back(Event::ReadyPublished { node, result });
            }
            Effect::Spawn { node, attempt } => self.spawn_body(node, attempt),
            Effect::SpawnBlocking { node, attempt } => self.spawn_body(node, attempt),
            Effect::Serve { node, episode } => self.start_serving(node, episode),
            Effect::Abort(node) => self.abort(node),
            Effect::Signal(node) | Effect::StopService(node) => {
                if let Some(l) = self.live.get(&node) {
                    l.cx.set_deadline(self.machine.deadline_for(node));
                    l.cx.stop_signal().request();
                }
            }
            Effect::RefreshDeadline(node) => {
                if let Some(l) = self.live.get(&node) {
                    l.cx.set_deadline(self.machine.deadline_for(node));
                }
            }
            Effect::Release(node) | Effect::Compensate(node) => self.spawn_cleanup(node, false),
            Effect::Recover(node) => self.spawn_cleanup(node, true),
            Effect::Timer { id, at } => self.arm(id, at),
            Effect::CancelTimer(id) => {
                if let Some(t) = self.timers.remove(&id) {
                    t.abort();
                }
            }
            Effect::Emit(ev) => {
                // An instance that has ended has no bodies left, so its slot
                // tables go with it (INV-13: they were per instance).
                if let TraceKind::InstanceEnded(id, outcome) = ev.kind {
                    self.scope.ended(id, outcome);
                    self.src.close_instance(id);
                }
                self.notify(&ev);
                self.trace.events.push(*ev);
            }
            Effect::End(outcome) => self.ended = Some(outcome),
            Effect::Reject(r) => self.reject(r),
            Effect::SpawnInstance {
                template,
                parent,
                id,
            } => {
                // The slots first, with the per-instance input already in
                // them: the machine's very next effects spawn the instance's
                // bodies, and they read this table.
                let input = self.scope.take_input(id);
                self.src.open_instance(template, parent, id, input);
                for (key, decl, path, kind) in self.machine.instance_nodes(id) {
                    self.kinds.insert(key, kind);
                    self.origins.insert(key, (decl, Some(id)));
                    self.paths.push((key, path));
                }
            }
        }
    }

    fn spawn_body(&mut self, node: RawKey, attempt: u32) {
        // A component has no body: the machine pushes no spawn for one, and a
        // body outcome delivered for one would give it a fault vector the
        // engine assumes is always empty (`exits.rs`).
        debug_assert_ne!(self.kinds.get(&node), Some(&Kind::Component));
        self.epoch += 1;
        let epoch = self.epoch;
        // The scope is attached before the deadline: `with_*` rebuilds the
        // context, and a body must reach `cx.spawn` (contract § 5).
        let mut cx = CxInner::new(node, self.clock.clone())
            .with_attempt(attempt)
            .with_scope(self.scope.clone());
        if let Some(at) = self.machine.deadline_for(node) {
            cx = cx.with_deadline(at);
        }
        let watch = self.kinds.get(&node).map(|k| k.can_hold()).unwrap_or(false);
        let tx = self.tx.clone();
        let (decl, inst) = self.origin(node);
        let handle = match self.src.body(decl, inst, &cx) {
            Some(Task::Async(f)) => Some(self.rt.spawn(Box::pin(run_body(
                tx,
                node,
                epoch,
                cx.clone(),
                f,
                watch,
                true,
            )))),
            Some(Task::Blocking(f)) => Some(
                self.rt
                    .spawn_blocking(blocking_job(tx, node, epoch, f, true)),
            ),
            None => {
                let _ = tx.send(Msg::Body {
                    node,
                    epoch,
                    ev: Event::NodeErr(
                        node,
                        FaultKind::Error(Box::new(DriverError(
                            "the body source has no body for this node",
                        ))),
                    ),
                });
                None
            }
        };
        self.live.insert(node, Live { epoch, cx, handle });
    }

    fn spawn_cleanup(&mut self, node: RawKey, recovery: bool) {
        let epoch = self.live.get(&node).map(|l| l.epoch).unwrap_or_else(|| {
            self.epoch += 1;
            self.epoch
        });
        let mut cx = CxInner::new(node, self.clock.clone()).with_scope(self.scope.clone());
        if let Some(at) = self.machine.deadline_for(node) {
            cx = cx.with_deadline(at);
        }
        if let Some(live) = self.live.get(&node) {
            cx = cx.with_attempt(live.cx.context::<sdax::Run>().attempt());
        }
        if recovery {
            cx = cx.with_recovery();
        }
        let tx = self.tx.clone();
        let (decl, inst) = self.origin(node);
        let handle = match self.src.cleanup(decl, inst, &cx) {
            Some(Task::Async(f)) => Some(self.rt.spawn(Box::pin(run_body(
                tx,
                node,
                epoch,
                cx.clone(),
                f,
                false,
                false,
            )))),
            // A cleanup announces nothing: the machine has no body in flight
            // for a node whose release it just opened, and would refuse a
            // `Started` for it (D1, `run_body`'s `announce`).
            Some(Task::Blocking(f)) => Some(
                self.rt
                    .spawn_blocking(blocking_job(tx, node, epoch, f, false)),
            ),
            None => {
                let _ = tx.send(Msg::Body {
                    node,
                    epoch,
                    ev: Event::NodeErr(
                        node,
                        FaultKind::Error(Box::new(DriverError(
                            "the body source has no cleanup body for this node",
                        ))),
                    ),
                });
                None
            }
        };
        self.live.insert(node, Live { epoch, cx, handle });
    }

    /// T5: abort, then join in a task of the engine's own, so the node counts
    /// as settled only once the join is in.
    fn abort(&mut self, node: RawKey) {
        let Some(live) = self.live.get_mut(&node) else {
            return;
        };
        let Some(handle) = live.handle.take() else {
            return;
        };
        handle.abort();
        let (epoch, cx, tx) = (live.epoch, live.cx.clone(), self.tx.clone());
        self.rt
            .spawn(Box::pin(join_aborted(tx, node, epoch, cx, handle)));
    }

    fn arm(&mut self, id: TimerId, at: Time) {
        let now = self.clock.now();
        let wait = at.checked_duration_since(now).unwrap_or_default();
        let sleep = self.clock.sleep(wait);
        let tx = self.tx.clone();
        let task = self.rt.spawn(Box::pin(async move {
            sleep.await;
            let _ = tx.send(Msg::Timer(id));
        }));
        if let Some(old) = self.timers.insert(id, task) {
            old.abort();
        }
    }
}
