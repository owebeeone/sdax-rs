//! F4 — the report's defined order, the engine's event/effect vocabulary, and
//! the host contracts.
//!
//! Stage 0 delivers the types and the ordering; a run that *produces* a report
//! in this order is Stage 1.

use super::exec::block_on;
use crate::host::engine::{Effect, Event, JoinedLabel, TimerId};
use crate::host::{
    BoxFuture, Clock, InstanceId, Joined, NoObserver, Observer, RawKey, Runtime, TaskHandle, Time,
};
use crate::*;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn at(steps: &[(u32, Option<u64>)], attempt: u32) -> RecordOrder {
    RecordOrder {
        steps: steps.iter().map(|(n, i)| (*n, i.map(InstanceId))).collect(),
        attempt,
    }
}

fn fault(path: &str, order: RecordOrder) -> Fault {
    let mut segs = path.split('/');
    let mut node = NodePath::root(segs.next().unwrap());
    for s in segs {
        node = node.child(s);
    }
    Fault {
        node,
        order,
        phase: Phase::Prepare,
        kind: FaultKind::Timeout,
    }
}

#[test]
fn records_order_by_declaration_then_instance_then_attempt() {
    // Declaration order 0,1,2; node 2 is a template with instances 1 and 2.
    let mut faults = [
        fault("Link/Entry", at(&[(2, Some(2)), (0, None)], 1)),
        fault("B", at(&[(1, None)], 2)),
        fault("B", at(&[(1, None)], 1)),
        fault("Link", at(&[(2, None)], 1)),
        fault("Link/Entry", at(&[(2, Some(1)), (0, None)], 1)),
        fault("A", at(&[(0, None)], 1)),
    ];
    faults.sort_by(|a, b| a.order.cmp(&b.order));
    let order: Vec<(&str, u32)> = faults
        .iter()
        .map(|f| (f.node.leaf(), f.order.attempt))
        .collect();
    assert_eq!(
        order,
        [
            ("A", 1),
            ("B", 1),
            ("B", 2),
            ("Link", 1),
            ("Entry", 1),
            ("Entry", 1)
        ],
        "declaration order; a template before its instances; attempts within a node"
    );
    assert_eq!(
        faults[4].order.steps[0].1,
        Some(InstanceId(1)),
        "instance 1 before instance 2"
    );
    assert_eq!(faults[5].order.steps[0].1, Some(InstanceId(2)));
}

#[test]
fn report_sort_puts_every_list_in_the_defined_order_and_leaves_the_trace_alone() {
    let mut r: Report = Report {
        outcome: Outcome::Failed,
        output: None,
        faults: vec![
            fault("B", at(&[(1, None)], 1)),
            fault("A", at(&[(0, None)], 1)),
        ],
        cleanup_failures: vec![
            fault("B", at(&[(1, None)], 1)),
            fault("A", at(&[(0, None)], 1)),
        ],
        incomplete: vec![
            NodeRecord {
                node: NodePath::root("B"),
                order: at(&[(1, None)], 1),
            },
            NodeRecord {
                node: NodePath::root("A"),
                order: at(&[(0, None)], 1),
            },
        ],
        ambiguous: vec![
            NodeRecord {
                node: NodePath::root("B"),
                order: at(&[(1, None)], 1),
            },
            NodeRecord {
                node: NodePath::root("A"),
                order: at(&[(0, None)], 1),
            },
        ],
        trace: Some(Trace {
            events: vec![
                TraceEvent::at(Time::from_nanos(2), TraceKind::End(Outcome::Failed)),
                TraceEvent::at(Time::from_nanos(1), TraceKind::Ready),
            ],
        }),
    };
    r.sort();
    assert_eq!(r.faults[0].node.leaf(), "A");
    assert_eq!(r.cleanup_failures[0].node.leaf(), "A");
    assert_eq!(r.incomplete[0].node.leaf(), "A");
    assert_eq!(r.ambiguous[0].node.leaf(), "A");
    let trace = r.trace.as_ref().expect("trace");
    assert!(
        matches!(trace.events[0].kind, TraceKind::End(_)),
        "trace events stay in observation order"
    );
}

#[test]
fn a_report_is_clean_only_when_every_list_is_empty() {
    let clean: Report = Report::empty(Outcome::Ok);
    assert!(clean.is_clean());
    assert!(clean.into_result().is_ok());

    let mut unclean: Report = Report::empty(Outcome::Ok);
    unclean.incomplete.push(NodeRecord {
        node: NodePath::root("Worker"),
        order: at(&[(0, None)], 1),
    });
    assert!(!unclean.is_clean(), "an abandoned node is never silent");
    assert!(unclean.into_result().is_err());

    let mut ambiguous: Report = Report::empty(Outcome::Ok);
    ambiguous.ambiguous.push(NodeRecord {
        node: NodePath::root("Registration"),
        order: at(&[(0, None)], 1),
    });
    assert!(!ambiguous.is_clean());
}

#[test]
fn panics_are_carried_never_re_raised() {
    let mut r: Report = Report::empty(Outcome::Failed);
    r.faults.push(Fault {
        node: NodePath::root("P1"),
        order: at(&[(0, None)], 1),
        phase: Phase::Run,
        kind: FaultKind::Panic(Box::new("boom")),
    });
    assert_eq!(r.panics().len(), 1);
    assert_eq!(r.faults[0].kind.label(), FaultLabel::Panic);
}

#[test]
fn engine_events_and_effects_are_public_named_types() {
    let node = RawKey { plan: 1, idx: 0 };
    let events = [
        Event::Started(node),
        Event::Held(node),
        Event::NodeOk(node),
        Event::NodeErr(node, FaultKind::Timeout),
        Event::NodeCancelled { node, held: true },
        Event::ServeEnded { node, fault: None },
        Event::Timer(TimerId(1)),
        Event::ShutdownRequested,
        Event::CancelRequested,
        Event::InstanceSpawned {
            spawner: node,
            template: node,
            id: InstanceId(1),
        },
        Event::StopInstance(InstanceId(1)),
        Event::TaskJoined {
            node,
            joined: JoinedLabel::Panicked,
        },
    ];
    assert_eq!(events.len(), 12);
    let effects = [
        Effect::Spawn { node, attempt: 1 },
        Effect::SpawnBlocking { node, attempt: 1 },
        Effect::Abort(node),
        Effect::Signal(node),
        Effect::Release(node),
        Effect::Compensate(node),
        Effect::StopService(node),
        Effect::Timer {
            id: TimerId(1),
            at: Time::from_nanos(1),
        },
        Effect::SpawnInstance {
            template: node,
            parent: None,
            id: InstanceId(1),
        },
        Effect::End(Outcome::Ok),
    ];
    assert_eq!(effects.len(), 10);
    assert!(format!("{:?}", Event::Started(node)).contains("Started"));
}

// ---------------------------------------------------------------- contracts

struct FakeRuntime {
    clock: Arc<FakeClock>,
    observer: Recorder,
}

struct FakeClock {
    nanos: AtomicU64,
}

impl Clock for FakeClock {
    fn now(&self) -> Time {
        Time::from_nanos(self.nanos.load(AtomicOrdering::SeqCst))
    }
    fn sleep(&self, _d: Duration) -> BoxFuture<'static, ()> {
        Box::pin(std::future::ready(()))
    }
}

#[derive(Default)]
struct Recorder {
    seen: Mutex<Vec<String>>,
}

impl Observer for Recorder {
    fn event(&self, e: &TraceEvent) {
        self.seen.lock().unwrap().push(format!("{:?}", e.kind));
    }
}

struct FakeTask {
    aborted: Arc<AtomicU64>,
}

impl TaskHandle for FakeTask {
    fn abort(&self) {
        self.aborted.fetch_add(1, AtomicOrdering::SeqCst);
    }
    fn join(self) -> BoxFuture<'static, Joined> {
        Box::pin(std::future::ready(Joined::Done))
    }
}

impl Runtime for FakeRuntime {
    type Task = FakeTask;
    fn spawn(&self, fut: BoxFuture<'static, ()>) -> FakeTask {
        block_on(fut);
        FakeTask {
            aborted: Arc::new(AtomicU64::new(0)),
        }
    }
    fn spawn_blocking(&self, f: Box<dyn FnOnce() + Send>) -> FakeTask {
        f();
        FakeTask {
            aborted: Arc::new(AtomicU64::new(0)),
        }
    }
    fn clock(&self) -> &dyn Clock {
        self.clock.as_ref()
    }
    fn observer(&self) -> &dyn Observer {
        &self.observer
    }
}

#[test]
fn the_host_contracts_are_implementable_and_dyn_usable() {
    let rt = FakeRuntime {
        clock: Arc::new(FakeClock {
            nanos: AtomicU64::new(0),
        }),
        observer: Recorder::default(),
    };
    let flag = Arc::new(AtomicU64::new(0));
    let f = flag.clone();
    let task = rt.spawn(Box::pin(async move {
        f.fetch_add(1, AtomicOrdering::SeqCst);
    }));
    assert_eq!(flag.load(AtomicOrdering::SeqCst), 1);
    task.abort();
    assert!(matches!(block_on(task.join()), Joined::Done));

    rt.spawn_blocking(Box::new(move || {
        flag.fetch_add(1, AtomicOrdering::SeqCst);
    }));

    rt.observer()
        .event(&TraceEvent::at(Time::ZERO, TraceKind::End(Outcome::Ok)));
    assert_eq!(rt.observer.seen.lock().unwrap().len(), 1);

    // The clock is usable as `dyn` and is what `Cx` reads.
    let clock: &dyn Clock = rt.clock();
    assert_eq!(clock.now(), Time::ZERO);

    // `NoObserver` is the "record nothing" implementation.
    let none: &dyn Observer = &NoObserver;
    none.event(&TraceEvent::at(Time::ZERO, TraceKind::Ready));
}
