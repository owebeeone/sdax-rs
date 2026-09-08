use sdax::{FaultKind, Outcome as SdaxOutcome, Report, TraceKind};
use std::error::Error as StdError;
use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;


#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Case {
    Normal,
    StartupFailure,
    CleanupFailure,
    Cancellation,
}

impl Case {
    pub const ALL: [Case; 4] = [
        Case::Normal,
        Case::StartupFailure,
        Case::CleanupFailure,
        Case::Cancellation,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Case::Normal => "normal_release",
            Case::StartupFailure => "partial_startup_failure",
            Case::CleanupFailure => "downstream_cleanup_failure",
            Case::Cancellation => "cancel_after_acquisition",
        }
    }

    pub fn nodes(self) -> usize {
        match self {
            Case::Normal | Case::StartupFailure => 2,
            Case::CleanupFailure => 3,
            Case::Cancellation => 1,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Ok,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fault {
    pub node: String,
    pub attempt: u32,
    pub phase: String,
    pub message: String,
    pub sources: Vec<String>,
    pub typed_fixture_error: bool,
}

#[derive(Debug)]
pub struct Summary {
    pub outcome: Outcome,
    pub output: Option<u64>,
    pub faults: Vec<Fault>,
    pub cleanup_failures: Vec<Fault>,
    pub events: Vec<&'static str>,
    pub spawned: usize,
    pub joined: usize,
    pub active: usize,
    pub disposed: usize,
    pub internal_records: usize,
}

pub struct Expected {
    outcome: Outcome,
    output: Option<u64>,
    faults: Vec<Fault>,
    cleanup_failures: Vec<Fault>,
    events: Vec<&'static str>,
    tasks: usize,
    disposed: usize,
}

#[derive(Clone)]
pub struct Evidence {
    inner: Arc<EvidenceInner>,
}

struct EvidenceInner {
    events: Mutex<Vec<&'static str>>,
    disposed: AtomicUsize,
    ready: Mutex<Option<oneshot::Sender<()>>>,
}

impl Evidence {
    pub fn new() -> Self {
        Evidence {
            inner: Arc::new(EvidenceInner {
                events: Mutex::new(Vec::new()),
                disposed: AtomicUsize::new(0),
                ready: Mutex::new(None),
            }),
        }
    }

    pub fn begin(&self, case: Case) -> Option<oneshot::Receiver<()>> {
        self.inner.events.lock().unwrap().clear();
        self.inner.disposed.store(0, Ordering::Relaxed);
        let mut ready = self.inner.ready.lock().unwrap();
        if case == Case::Cancellation {
            let (tx, rx) = oneshot::channel();
            *ready = Some(tx);
            Some(rx)
        } else {
            *ready = None;
            None
        }
    }

    pub fn record(&self, event: &'static str) {
        self.inner.events.lock().unwrap().push(event);
    }

    pub fn signal_ready(&self) {
        self.inner
            .ready
            .lock()
            .unwrap()
            .take()
            .expect("acquisition-ready sender installed")
            .send(())
            .expect("acquisition-ready receiver alive");
    }

    pub fn events(&self) -> Vec<&'static str> {
        self.inner.events.lock().unwrap().clone()
    }

    pub fn disposed(&self) -> usize {
        self.inner.disposed.load(Ordering::Relaxed)
    }

    pub(crate) fn resource(&self, value: u64) -> TrackedResource {
        TrackedResource {
            value,
            disposed: self.inner.clone(),
        }
    }
}

pub struct TrackedResource {
    pub value: u64,
    disposed: Arc<EvidenceInner>,
}

impl Drop for TrackedResource {
    fn drop(&mut self) {
        self.disposed.disposed.fetch_add(1, Ordering::Relaxed);
    }
}

#[derive(Debug)]
pub struct FixtureError {
    message: &'static str,
    source: std::io::Error,
}

impl FixtureError {
    pub fn startup() -> Self {
        FixtureError {
            message: "fixture startup failure",
            source: std::io::Error::other("fixture startup source"),
        }
    }

    pub fn cleanup() -> Self {
        FixtureError {
            message: "fixture downstream release failure",
            source: std::io::Error::other("fixture cleanup source"),
        }
    }
}

impl fmt::Display for FixtureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message)
    }
}

impl StdError for FixtureError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        Some(&self.source)
    }
}

pub fn normalize_error(
    node: &str,
    attempt: u32,
    phase: &str,
    error: &(dyn StdError + 'static),
) -> Fault {
    let mut sources = Vec::new();
    let mut source = error.source();
    while let Some(cause) = source {
        sources.push(cause.to_string());
        source = cause.source();
    }
    Fault {
        node: node.to_owned(),
        attempt,
        phase: phase.to_owned(),
        message: error.to_string(),
        sources,
        typed_fixture_error: error.downcast_ref::<FixtureError>().is_some(),
    }
}

pub fn expected(case: Case) -> Expected {
    let fault = |node: &str, phase: &str, message: &str, source: &str| Fault {
        node: node.to_owned(),
        attempt: 1,
        phase: phase.to_owned(),
        message: message.to_owned(),
        sources: vec![source.to_owned()],
        typed_fixture_error: true,
    };
    match case {
        Case::Normal => Expected {
            outcome: Outcome::Ok,
            output: Some(42),
            faults: Vec::new(),
            cleanup_failures: Vec::new(),
            events: vec!["acquire_resource", "use_resource", "release_resource"],
            tasks: 3,
            disposed: 1,
        },
        Case::StartupFailure => Expected {
            outcome: Outcome::Failed,
            output: None,
            faults: vec![fault(
                "failed",
                "run",
                "fixture startup failure",
                "fixture startup source",
            )],
            cleanup_failures: Vec::new(),
            events: vec![
                "acquire_upstream",
                "fail_downstream_run",
                "release_upstream",
            ],
            tasks: 3,
            disposed: 1,
        },
        Case::CleanupFailure => Expected {
            outcome: Outcome::Ok,
            output: Some(41),
            faults: Vec::new(),
            cleanup_failures: vec![fault(
                "downstream",
                "release",
                "fixture downstream release failure",
                "fixture cleanup source",
            )],
            events: vec![
                "acquire_upstream",
                "acquire_downstream",
                "use_resource",
                "fail_downstream_release",
                "release_upstream",
            ],
            tasks: 5,
            disposed: 2,
        },
        Case::Cancellation => Expected {
            outcome: Outcome::Cancelled,
            output: None,
            faults: Vec::new(),
            cleanup_failures: Vec::new(),
            events: vec!["hold_resource", "release_resource"],
            tasks: 2,
            disposed: 1,
        },
    }
}

pub fn check(summary: &Summary, expected: &Expected) -> Result<(), &'static str> {
    if summary.outcome != expected.outcome {
        return Err("outcome");
    }
    if summary.output != expected.output {
        return Err("output");
    }
    if summary.faults != expected.faults {
        return Err("faults");
    }
    if summary.cleanup_failures != expected.cleanup_failures {
        return Err("cleanup_failures");
    }
    if summary.events != expected.events {
        return Err("events");
    }
    if summary.spawned != expected.tasks {
        return Err("spawned");
    }
    if summary.joined != expected.tasks {
        return Err("joined");
    }
    if summary.active != 0 {
        return Err("active");
    }
    if summary.disposed != expected.disposed {
        return Err("disposed");
    }
    if summary.internal_records == 0 {
        return Err("internal_records");
    }
    Ok(())
}

pub fn checksum(summary: &Summary) -> u64 {
    summary.output.unwrap_or(0)
        ^ summary.spawned as u64
        ^ (summary.joined as u64).rotate_left(7)
        ^ (summary.disposed as u64).rotate_left(13)
        ^ summary.events.len() as u64
        ^ summary.faults.len() as u64
        ^ summary.cleanup_failures.len() as u64
}

fn normalize_fault(fault: &sdax::Fault) -> Fault {
    match &fault.kind {
        FaultKind::Error(error) => normalize_error(
            fault.node.leaf(),
            fault.order.attempt,
            &fault.phase.to_string(),
            error.as_ref(),
        ),
        other => panic!("lifecycle fixture expected typed error, got {other}"),
    }
}

pub fn normalize_report(report: &Report<u64>, evidence: &Evidence) -> Summary {
    assert!(report.incomplete.is_empty(), "lifecycle fixture left incomplete work");
    assert!(report.ambiguous.is_empty(), "lifecycle fixture left ambiguity");
    let trace = report.trace.as_ref().expect("full lifecycle trace");
    let spawned = trace
        .events
        .iter()
        .filter(|event| matches!(event.kind, TraceKind::Start(_) | TraceKind::ReleaseStart))
        .count();
    let joined = trace
        .events
        .iter()
        .filter(|event| {
            matches!(
                event.kind,
                TraceKind::Ready
                    | TraceKind::Fail(_, _)
                    | TraceKind::Interrupted { .. }
                    | TraceKind::ReleaseOk
                    | TraceKind::ReleaseFail(_)
            )
        })
        .count();
    Summary {
        outcome: match report.outcome {
            SdaxOutcome::Ok => Outcome::Ok,
            SdaxOutcome::Failed => Outcome::Failed,
            SdaxOutcome::Cancelled => Outcome::Cancelled,
        },
        output: report.output.as_deref().copied(),
        faults: report.faults.iter().map(normalize_fault).collect(),
        cleanup_failures: report.cleanup_failures.iter().map(normalize_fault).collect(),
        events: evidence.events(),
        spawned,
        joined,
        active: usize::MAX,
        disposed: usize::MAX,
        internal_records: trace.events.len(),
    }
}
