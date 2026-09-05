//! An observer that keeps what it is told.

use sdax::{Observer, Trace, TraceEvent, TraceKind};
use std::sync::Mutex;

/// An [`Observer`] that records every event in observation order.
///
/// The order is the observation order and is never sorted: a trace that has
/// been reordered no longer says what happened.
#[derive(Default)]
pub struct TraceRecorder {
    events: Mutex<Vec<TraceEvent>>,
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

    /// Forget everything recorded so far.
    pub fn clear(&self) {
        self.events.lock().expect("recorder poisoned").clear();
    }
}

impl Observer for TraceRecorder {
    fn event(&self, e: &TraceEvent) {
        self.events
            .lock()
            .expect("recorder poisoned")
            .push(e.clone());
    }
}
