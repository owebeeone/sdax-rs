//! In-memory backend shared by the complete starter plans.
//! Replace these operations with application I/O while retaining the plan structure.

use sdax::Error;
use std::collections::BTreeSet;
use std::future::{ready, Future, Ready};
use std::pin::Pin;
use std::sync::{Arc, Mutex};

/// An operation failure with its original nested cause.
#[derive(Debug)]
pub struct OperationError {
    message: &'static str,
    source: Option<Box<OperationError>>,
}
impl std::fmt::Display for OperationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message)
    }
}
impl std::error::Error for OperationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source.as_deref().map(|error| error as _)
    }
}
/// Creates a modeled operation error retaining its underlying cause.
pub(crate) fn fault(message: &'static str, cause: &'static str) -> Error {
    Box::new(OperationError {
        message,
        source: Some(Box::new(OperationError {
            message: cause,
            source: None,
        })),
    })
}

/// An acquired object whose name identifies its live backend registration.
#[derive(Debug)]
pub struct Lease {
    /// The backend identity, which must be unique while the lease is live.
    pub name: String,
    /// The value used by the starter's completed-output calculation.
    pub value: u32,
}
#[derive(Default)]
struct State {
    events: Vec<String>,
    live: BTreeSet<String>,
    work: u32,
}

/// A receipt that may remain pending when the external outcome is unknown.
pub type ReceiptFuture = Pin<Box<dyn Future<Output = Result<u32, Error>> + Send>>;

/// Shared in-memory operations and observations for the starter plans.
#[derive(Clone, Default)]
pub struct Environment {
    state: Arc<Mutex<State>>,
    fail_close: Option<String>,
    never_ready: bool,
    known_failure: bool,
    /// Whether reconciliation confirms the previously unknown outcome.
    pub resolve: bool,
}
impl Environment {
    /// Configure a release failure after the lease is closed.
    pub fn with_cleanup_failure(mut self, name: &str) -> Self {
        self.fail_close = Some(name.into());
        self
    }
    /// Keep both transfer acquisition attempts unavailable.
    pub fn with_unavailable_transfer(mut self) -> Self {
        self.never_ready = true;
        self
    }
    /// Model reconciliation that confirms the external outcome.
    pub fn with_resolved_recovery(mut self) -> Self {
        self.resolve = true;
        self
    }
    /// Model an explicit known rejection rather than an unknown outcome.
    pub fn with_request_failure(mut self) -> Self {
        self.known_failure = true;
        self
    }
    /// Record an operation or lifecycle observation.
    pub fn record(&self, event: impl Into<String>) {
        self.state.lock().unwrap().events.push(event.into());
    }
    /// Read the observations in their recorded order.
    pub fn events(&self) -> Vec<String> {
        self.state.lock().unwrap().events.clone()
    }
    /// Count the leases that still belong to the backend.
    pub fn live(&self) -> usize {
        self.state.lock().unwrap().live.len()
    }
    /// Count the service's outstanding units of work.
    pub fn work(&self) -> u32 {
        self.state.lock().unwrap().work
    }
    /// Register a lease immediately and return its ready acquisition future.
    ///
    /// This factory changes backend state before its future is polled. Invoke
    /// it inside the lazy closure passed to `cx.hold`.
    pub fn acquire(
        &self,
        name: &str,
        value: u32,
        parent: Option<&Lease>,
    ) -> Ready<Result<Lease, Error>> {
        let mut state = self.state.lock().unwrap();
        if let Some(parent) = parent {
            assert!(state.live.contains(&parent.name), "parent must remain live");
        }
        assert!(state.live.insert(name.to_owned()), "duplicate acquisition");
        state.events.push(format!("open:{name}:{value}"));
        ready(Ok(Lease {
            name: name.to_owned(),
            value,
        }))
    }
    /// Close a lease, retaining a configured release error and its cause.
    pub fn release(&self, lease: &Lease) -> Result<(), Error> {
        let mut state = self.state.lock().unwrap();
        assert!(state.live.remove(&lease.name), "duplicate release");
        state.events.push(format!("close:{}", lease.name));
        if self.fail_close.as_deref() == Some(lease.name.as_str()) {
            Err(fault("flush seal", "disk rack 7"))
        } else {
            Ok(())
        }
    }
    /// Attempt a transfer that normally becomes available on attempt two.
    pub fn transfer(&self, attempt: u32, parent: &Lease) -> Ready<Result<Lease, Error>> {
        self.record(format!("transfer-attempt:{attempt}"));
        if attempt == 1 || self.never_ready {
            ready(Err(fault("gate warming", "thermostat")))
        } else {
            self.acquire("transfer", parent.value + 11, Some(parent))
        }
    }
    /// Record the identity and model either known rejection or a pending result.
    ///
    /// The pending branch intentionally exercises timeout and reconciliation;
    /// it is an in-memory scenario, not an external-service implementation.
    pub fn request(&self, identity: &Arc<u32>) -> ReceiptFuture {
        self.record(format!(
            "request:{}:{}",
            **identity,
            Arc::as_ptr(identity) as usize
        ));
        if self.known_failure {
            Box::pin(ready(Err(fault("request rejected", "schema 6"))))
        } else {
            Box::pin(std::future::pending())
        }
    }
    /// Record which stable identity was used to reconcile the pending request.
    pub fn reconcile(&self, identity: &Arc<u32>) {
        self.record(format!(
            "reconcile:{}:{}",
            **identity,
            Arc::as_ptr(identity) as usize
        ));
    }
    /// Register one unit of work owned by the serving episode.
    pub fn begin_work(&self) {
        let mut state = self.state.lock().unwrap();
        state.work += 1;
        state.events.push("work-start".into());
    }
    /// Finish that unit of work before releasing the service's dependencies.
    pub fn drain_work(&self) {
        let mut state = self.state.lock().unwrap();
        assert_eq!(state.work, 1);
        state.work = 0;
        state.events.push("work-drained".into());
    }
}
