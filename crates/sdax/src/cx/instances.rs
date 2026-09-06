//! The instance side of the seam: the stop signal a body observes, and the
//! handle it gets for a template instance it spawned.

use crate::contracts::{BoxFuture, Error};
use crate::key::RawKey;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

// ---------------------------------------------------------------- stop signal

struct StopState {
    stopped: bool,
    wakers: Vec<Waker>,
}

/// The engine's stop request for one scope, observable from every body.
pub struct StopSignal {
    state: Mutex<StopState>,
}

impl StopSignal {
    /// A signal that has not been requested yet.
    pub fn new() -> Arc<Self> {
        Arc::new(StopSignal {
            state: Mutex::new(StopState {
                stopped: false,
                wakers: Vec::new(),
            }),
        })
    }

    /// Request the stop. Idempotent; wakes everything waiting on it.
    pub fn request(&self) {
        let wakers = {
            let mut st = self.state.lock().expect("stop signal poisoned");
            st.stopped = true;
            std::mem::take(&mut st.wakers)
        };
        for w in wakers {
            w.wake();
        }
    }

    /// Whether the stop has been requested.
    pub fn is_stopping(&self) -> bool {
        self.state.lock().expect("stop signal poisoned").stopped
    }

    pub(crate) fn poll_stopped(&self, cx: &mut Context<'_>) -> Poll<()> {
        let mut st = self.state.lock().expect("stop signal poisoned");
        if st.stopped {
            return Poll::Ready(());
        }
        if !st.wakers.iter().any(|w| w.will_wake(cx.waker())) {
            st.wakers.push(cx.waker().clone());
        }
        Poll::Pending
    }
}

/// The future returned by [`Cx::stop`](crate::Cx::stop): resolves when the scope asks this node
/// to stop. `Unpin`, so it can be held across a `select`-style loop.
pub struct Stop {
    signal: Arc<StopSignal>,
}

impl Stop {
    /// Wait on this signal.
    ///
    /// Host API: a body says `cx.stop()`; a run driver builds one directly,
    /// which is why this is public.
    pub fn on(signal: Arc<StopSignal>) -> Stop {
        Stop { signal }
    }
}

impl Future for Stop {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.signal.poll_stopped(cx)
    }
}

// ---------------------------------------------------------------- instances

/// Identity of one template instance within a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InstanceId(pub u64);

impl std::fmt::Display for InstanceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// Why [`Cx::spawn`](crate::Cx::spawn) refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnError {
    /// The template belongs to another plan.
    ForeignTemplate,
    /// The spawning node did not declare `spawns(&template)`.
    UndeclaredTemplate,
    /// The scope is settling and admits no new instances.
    ScopeStopping,
    /// No run is attached to this context. Stage 0 has no execution, so every
    /// `spawn` from a hand-built context ends here.
    NotRunning,
}

impl std::fmt::Display for SpawnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            SpawnError::ForeignTemplate => "the template belongs to another plan",
            SpawnError::UndeclaredTemplate => "this node did not declare spawns(&template)",
            SpawnError::ScopeStopping => "the scope is stopping and admits no new instances",
            SpawnError::NotRunning => "no run is attached to this context",
        };
        f.write_str(s)
    }
}
impl std::error::Error for SpawnError {}

/// What the engine gives a body so it can drive one live instance.
pub trait ChildControl: Send + Sync {
    /// Ask this instance to stop. Returns immediately.
    fn stop(&self, id: InstanceId);
    /// A future that completes when the instance's own scope reaches steady
    /// state, or errors if it ended before that (F1).
    fn ready(&self, id: InstanceId) -> BoxFuture<'static, Result<(), Error>>;
}

/// A live template instance, as seen by the body that spawned it.
///
/// A `Child` can be stopped and awaited for readiness; it cannot be depended
/// on by a declaration (INV-16) and it carries no output to the parent.
pub struct Child {
    id: InstanceId,
    ctl: Arc<dyn ChildControl>,
}

impl Child {
    /// Engine-side constructor.
    pub fn new(id: InstanceId, ctl: Arc<dyn ChildControl>) -> Self {
        Child { id, ctl }
    }

    /// This instance's identity within the run.
    pub fn id(&self) -> InstanceId {
        self.id
    }

    /// Ask this instance to stop.
    pub fn stop(&self) {
        self.ctl.stop(self.id);
    }

    /// Await this instance's readiness (F1).
    ///
    /// A start body that spawns N instances and awaits each `ready()` before
    /// returning `Serving` makes the scope's readiness include the instances,
    /// which is what makes "a node that depends on all instances being ready"
    /// expressible. The template must not `import` the spawning service's own
    /// key: that would be a readiness deadlock, and
    /// [`Rule::SpawnSelfImport`](crate::Rule::SpawnSelfImport) rejects it at
    /// `build`.
    pub fn ready(&self) -> BoxFuture<'static, Result<(), Error>> {
        self.ctl.ready(self.id)
    }
}

impl std::fmt::Debug for Child {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Child")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// The engine side of `cx.spawn`: the run's scope, as a body may reach it.
pub trait Scope: Send + Sync {
    /// Instantiate a registered template with a per-instance input.
    ///
    /// `input` is a boxed `Arc<I>`: it is written straight into the instance's
    /// slot for the input node, and a slot holds `Arc<T>`.
    fn spawn_instance(
        &self,
        spawner: RawKey,
        template: RawKey,
        input: Box<dyn std::any::Any + Send + Sync>,
    ) -> Result<Child, SpawnError>;
}
