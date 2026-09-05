//! An observer that keeps what it is told.

use sdax::host::Observer;
use sdax::{Outcome, Report, Trace, TraceEvent, TraceKind};
use std::sync::Mutex;

/// What a [`Report`] said, as a value a test can keep.
///
/// A `Report` is not `Clone` — a `FaultKind` carries a boxed error or panic
/// payload — so an observer that wants to remember one keeps this instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReportSummary {
    /// How the run ended.
    pub outcome: Outcome,
    /// How many faults it carried.
    pub faults: usize,
    /// How many cleanup failures.
    pub cleanup_failures: usize,
    /// How many abandoned obligations.
    pub incomplete: usize,
    /// How many ambiguous effects.
    pub ambiguous: usize,
}

/// An [`Observer`] that records every event in observation order.
///
/// The order is the observation order and is never sorted: a trace that has
/// been reordered no longer says what happened.
#[derive(Default)]
pub struct TraceRecorder {
    events: Mutex<Vec<TraceEvent>>,
    reports: Mutex<Vec<ReportSummary>>,
}

impl TraceRecorder {
    /// An empty recorder.
    pub fn new() -> Self {
        TraceRecorder::default()
    }

    /// Everything recorded so far.
    pub fn trace(&self) -> Trace {
        Trace {
            events: self.events.lock().expect("recorder poisoned").clone(),
        }
    }

    /// Whether an event of this kind was recorded.
    pub fn contains(&self, kind: &TraceKind) -> bool {
        self.events
            .lock()
            .expect("recorder poisoned")
            .iter()
            .any(|e| &e.kind == kind)
    }

    /// How many events were recorded.
    pub fn len(&self) -> usize {
        self.events.lock().expect("recorder poisoned").len()
    }

    /// Whether nothing was recorded.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Every report a run driver handed over, in order.
    ///
    /// A dropped `Running` has nobody awaiting its report, so this is where
    /// `C-14`'s report is read from.
    pub fn reports(&self) -> Vec<ReportSummary> {
        self.reports.lock().expect("recorder poisoned").clone()
    }

    /// Forget everything recorded so far.
    pub fn clear(&self) {
        self.events.lock().expect("recorder poisoned").clear();
        self.reports.lock().expect("recorder poisoned").clear();
    }
}

impl Observer for TraceRecorder {
    fn event(&self, e: &TraceEvent) {
        self.events
            .lock()
            .expect("recorder poisoned")
            .push(e.clone());
    }

    fn report(&self, r: &Report<()>) {
        self.reports
            .lock()
            .expect("recorder poisoned")
            .push(ReportSummary {
                outcome: r.outcome,
                faults: r.faults.len(),
                cleanup_failures: r.cleanup_failures.len(),
                incomplete: r.incomplete.len(),
                ambiguous: r.ambiguous.len(),
            });
    }
}
