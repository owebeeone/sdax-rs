//! The seam: the complete list of ways a node body can affect lifecycle state.
//!
//! A body is ordinary `async` Rust and nothing about it is sandboxed. What is
//! bounded is its *interface* to the engine, and this module is that interface:
//! [`Cx`] carries a phase marker so an operation that makes no sense in a phase
//! does not exist there, and [`Held`] cannot be forged.
//!
//! **The one rule of the body contract**: perform external effects *inside*
//! `cx.hold(..)`. `hold` registers the value in the same poll that observes the
//! effect completing, so there is no await point between "the effect happened"
//! and "the engine owns the cleanup". A body that performs the effect itself,
//! awaits something else, and then calls [`Cx::hold_value`] has re-created that
//! window by hand; the engine cannot see it (`Proposal.md` LG-2 limit).
//!
//! A service's own acquisitions belong in a resource node, not in its
//! initializer or serving factory: those values have no ledger entry or async release
//! (`AdversarialReview.md` F-B6). Use `needs` on a resource, or keep the value
//! in the serve future's locals and drop it there.

mod acquisition;
mod instances;
pub use acquisition::{Held, Hold};

pub use instances::{Child, ChildControl, InstanceId, Scope, SpawnError, Stop, StopSignal};

use crate::contracts::{Clock, Error, Time};
use crate::key::RawKey;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

/// Phase marker: a resource's `acquire` body and an effect's `perform` body.
/// The only phase with [`Cx::hold`]. Its context is not cloneable.
#[derive(Debug, Clone, Copy)]
pub struct Acquire;
/// Phase marker: a step's, try-step's or blocking step's `run` body.
#[derive(Debug, Clone, Copy)]
pub struct Run;
/// Phase marker: a service's initializer. Readiness is its successful return.
#[derive(Debug, Clone, Copy)]
pub struct Start;
/// Phase marker: one invocation of a service's serving factory.
#[derive(Debug, Clone, Copy)]
pub struct ServingPhase;
/// Phase marker: a `release` or `compensate` body. Has a clock, a deadline and
/// a stop signal, so bounded cooperative teardown is writable.
#[derive(Debug, Clone, Copy)]
pub struct Release;

// ---------------------------------------------------------------- context

/// The engine-owned state behind one body's [`Cx`].
///
/// Constructed by the engine (Stage 1) and by tests. Holding it is what lets a
/// body register values, read the clock, and observe the stop request.
pub struct CxInner {
    node: RawKey,
    attempt: u32,
    episode: u32,
    recovery: bool,
    clock: Arc<dyn Clock>,
    stop: Arc<StopSignal>,
    scope: Option<Arc<dyn Scope>>,
    held: Mutex<Registration>,
    repeated: AtomicBool,
    holds: AtomicU32,
    deadline: AtomicU64,
}

impl CxInner {
    /// A context for `node`, on `clock`, at attempt 1, with a fresh stop signal
    /// and no attached run.
    pub fn new(node: RawKey, clock: Arc<dyn Clock>) -> Arc<Self> {
        Arc::new(CxInner {
            node,
            attempt: 1,
            episode: 0,
            recovery: false,
            clock,
            stop: StopSignal::new(),
            scope: None,
            held: Mutex::new(Registration::Unused),
            repeated: AtomicBool::new(false),
            holds: AtomicU32::new(0),
            deadline: AtomicU64::new(u64::MAX),
        })
    }

    fn rebuilt(&self) -> CxInner {
        CxInner {
            node: self.node,
            attempt: self.attempt,
            episode: self.episode,
            recovery: self.recovery,
            clock: self.clock.clone(),
            stop: self.stop.clone(),
            scope: self.scope.clone(),
            held: Mutex::new(Registration::Unused),
            repeated: AtomicBool::new(false),
            holds: AtomicU32::new(0),
            deadline: AtomicU64::new(self.deadline.load(Ordering::SeqCst)),
        }
    }

    /// The same context at another attempt number (INV-12: attempts never
    /// overlap, so each gets its own registration cell).
    pub fn with_attempt(self: Arc<Self>, attempt: u32) -> Arc<Self> {
        let mut next = self.rebuilt();
        next.attempt = attempt;
        Arc::new(next)
    }

    /// The same context for a 1-based serving episode.
    pub fn with_episode(self: Arc<Self>, episode: u32) -> Arc<Self> {
        let mut next = self.rebuilt();
        next.episode = episode;
        Arc::new(next)
    }

    /// The same context sharing an existing stop signal.
    pub fn with_stop(self: Arc<Self>, stop: Arc<StopSignal>) -> Arc<Self> {
        let mut next = self.rebuilt();
        next.stop = stop;
        Arc::new(next)
    }

    /// The same context attached to a run's scope, so `cx.spawn` works.
    pub fn with_scope(self: Arc<Self>, scope: Arc<dyn Scope>) -> Arc<Self> {
        let mut next = self.rebuilt();
        next.scope = Some(scope);
        Arc::new(next)
    }

    /// The same context with a deadline the body can read.
    pub fn with_deadline(self: Arc<Self>, deadline: Time) -> Arc<Self> {
        let next = self.rebuilt();
        next.deadline.store(deadline.as_nanos(), Ordering::SeqCst);
        Arc::new(next)
    }

    /// Replace the deadline visible to an already-running body.
    ///
    /// The host uses this immediately before raising a stop signal, when a
    /// serving or cooperatively cancelled body changes from its execution
    /// phase to a bounded stop phase.
    pub fn set_deadline(&self, deadline: Option<Time>) {
        self.deadline.store(
            deadline.map(|d| d.as_nanos()).unwrap_or(u64::MAX),
            Ordering::SeqCst,
        );
    }

    /// Mark a cleanup context as explicit unknown-outcome recovery.
    pub fn with_recovery(self: Arc<Self>) -> Arc<Self> {
        let mut next = self.rebuilt();
        next.recovery = true;
        Arc::new(next)
    }

    /// Whether this cleanup invocation reconciles an unknown outcome.
    pub fn is_recovery(&self) -> bool {
        self.recovery
    }

    /// Which node this context belongs to.
    pub fn node(&self) -> RawKey {
        self.node
    }

    /// The stop signal this context observes.
    pub fn stop_signal(&self) -> &Arc<StopSignal> {
        &self.stop
    }

    /// How many values this attempt successfully registered: zero or one.
    /// Rejected host repetitions are reported by `repeated_registration`.
    pub fn hold_count(&self) -> u32 {
        self.holds.load(Ordering::SeqCst)
    }

    /// Take what the body registered. The engine calls this once the body's
    /// future has settled, whatever the outcome (INV-3).
    pub fn take_held(&self) -> Option<Box<dyn std::any::Any + Send + Sync>> {
        let mut cell = self.held.lock().expect("cx poisoned");
        match std::mem::replace(&mut *cell, Registration::Discharged) {
            Registration::Held(v) => Some(v),
            _ => None,
        }
    }

    /// Store an already-erased output. Used by the step and service terminals,
    /// whose outputs the engine registers on `Ok` rather than the body.
    pub fn put_output(&self, v: Box<dyn std::any::Any + Send + Sync>) {
        let mut cell = self.held.lock().expect("cx poisoned");
        if !matches!(*cell, Registration::Unused) {
            self.repeated.store(true, Ordering::SeqCst);
            return;
        }
        *cell = Registration::Held(v);
    }

    /// Instantiate a template by its declaration key.
    ///
    /// Host API: [`Cx::spawn`](crate::Cx::spawn) is the typed form an author
    /// writes; a harness that supplies erased bodies has only the key, and
    /// this is how it reaches the same seam. `input` must be a boxed
    /// `Arc<I>` — a slot holds `Arc<T>` for every node, the instance's input
    /// included (`OD-SPAWN-INPUT`).
    pub fn spawn_instance(
        &self,
        template: RawKey,
        input: Box<dyn std::any::Any + Send + Sync>,
    ) -> Result<Child, SpawnError> {
        match &self.scope {
            Some(scope) => scope.spawn_instance(self.node, template, input),
            None => Err(SpawnError::NotRunning),
        }
    }

    /// Construct acquisition authority at the host boundary. Repeated authorities
    /// share a defensive reservation and cannot invoke another acquisition.
    pub fn acquire(self: &Arc<Self>) -> Cx<Acquire> {
        Cx::new(self.clone())
    }

    /// Construct ordinary shared context at the host boundary.
    pub fn context<P>(self: &Arc<Self>) -> Cx<P>
    where
        P: SharedPhase,
    {
        Cx::new(self.clone())
    }

    /// Whether a host attempted to overwrite this attempt's registration.
    pub fn repeated_registration(&self) -> bool {
        self.repeated.load(Ordering::SeqCst)
    }

    fn reserve(&self) -> Result<(), Error> {
        let mut cell = self.held.lock().expect("cx poisoned");
        if !matches!(*cell, Registration::Unused) {
            self.repeated.store(true, Ordering::SeqCst);
            return Err(Box::new(AlreadyAcquired));
        }
        *cell = Registration::Reserved;
        Ok(())
    }

    fn register<T: ?Sized + Send + Sync + 'static>(&self, arc: Arc<T>) {
        let mut cell = self.held.lock().expect("cx poisoned");
        assert!(
            matches!(*cell, Registration::Reserved),
            "acquisition reservation lost"
        );
        *cell = Registration::Held(Box::new(arc));
        self.holds.fetch_add(1, Ordering::SeqCst);
    }
}

/// The body's handle on the engine, typed by phase.
///
/// See the module docs for the seam's complete operation list. Every operation
/// is a `cx.` call, so the seam is grep-able.
pub struct Cx<P> {
    inner: Arc<CxInner>,
    _p: PhantomData<fn() -> P>,
}

enum Registration {
    Unused,
    Reserved,
    Held(Box<dyn std::any::Any + Send + Sync>),
    Discharged,
}

#[derive(Debug)]
struct AlreadyAcquired;
impl std::fmt::Display for AlreadyAcquired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("this attempt's acquisition authority has already been consumed")
    }
}
impl std::error::Error for AlreadyAcquired {}

mod sealed {
    pub trait Shared {}
}
/// Phases whose contexts carry no acquisition authority and may be cloned.
pub trait SharedPhase: sealed::Shared {}
impl sealed::Shared for Run {}
impl sealed::Shared for Start {}
impl sealed::Shared for ServingPhase {}
impl sealed::Shared for Release {}
impl SharedPhase for Run {}
impl SharedPhase for Start {}
impl SharedPhase for ServingPhase {}
impl SharedPhase for Release {}

impl<P: SharedPhase> Clone for Cx<P> {
    fn clone(&self) -> Self {
        Cx {
            inner: self.inner.clone(),
            _p: PhantomData,
        }
    }
}

impl<P> Cx<P> {
    /// Wrap engine state as a body context.
    ///
    /// Host API: the run driver builds contexts, an author receives them.
    /// [`CxInner`] lives in [`host`](crate::host) for the same reason.
    pub(crate) fn new(inner: Arc<CxInner>) -> Self {
        Cx {
            inner,
            _p: PhantomData,
        }
    }

    /// The engine state behind this context. Host API, like [`Cx::new`].
    #[cfg(test)]
    pub(crate) fn inner(&self) -> Arc<CxInner> {
        self.inner.clone()
    }

    /// Share clock, cancellation and attempt identity without acquisition authority.
    pub fn shared(&self) -> Cx<Run> {
        Cx::new(self.inner.clone())
    }

    /// Which node this body belongs to.
    pub fn node(&self) -> RawKey {
        self.inner.node
    }

    /// A future that resolves when this scope asks the node to stop.
    pub fn stop(&self) -> Stop {
        Stop::on(self.inner.stop.clone())
    }

    /// Whether a stop has already been requested.
    pub fn is_stopping(&self) -> bool {
        self.inner.stop.is_stopping()
    }

    /// Run `f` until it completes or the scope asks to stop, whichever comes
    /// first. `None` means the stop won.
    pub async fn until_stop<F: Future>(&self, f: F) -> Option<F::Output> {
        let mut stop = self.stop();
        let mut f = std::pin::pin!(f);
        std::future::poll_fn(|cx| {
            if let Poll::Ready(v) = f.as_mut().poll(cx) {
                return Poll::Ready(Some(v));
            }
            if Pin::new(&mut stop).poll(cx).is_ready() {
                return Poll::Ready(None);
            }
            Poll::Pending
        })
        .await
    }

    /// Wait `d` on the engine's injected clock. Never a real sleep in tests.
    pub async fn sleep(&self, d: Duration) {
        self.inner.clock.sleep(d).await;
    }

    /// Run `f` with a bound measured on the engine's clock.
    pub async fn timeout<F: Future>(&self, d: Duration, f: F) -> Result<F::Output, Timeout> {
        let mut sleeping = self.inner.clock.sleep(d);
        let mut f = std::pin::pin!(f);
        std::future::poll_fn(|cx| {
            if let Poll::Ready(v) = f.as_mut().poll(cx) {
                return Poll::Ready(Ok(v));
            }
            if sleeping.as_mut().poll(cx).is_ready() {
                return Poll::Ready(Err(Timeout));
            }
            Poll::Pending
        })
        .await
    }

    /// The current time on the engine's clock.
    pub fn now(&self) -> Time {
        self.inner.clock.now()
    }

    /// The deadline for this body, if the node declared one, or the scope's
    /// remaining budget once the engine sets it.
    pub fn deadline(&self) -> Option<Time> {
        let d = self.inner.deadline.load(Ordering::SeqCst);
        if d == u64::MAX {
            None
        } else {
            Some(Time::from_nanos(d))
        }
    }

    /// This body's attempt number, counting from 1.
    pub fn attempt(&self) -> u32 {
        self.inner.attempt
    }

    pub(crate) fn spawn_raw(
        &self,
        template: RawKey,
        input: Box<dyn std::any::Any + Send + Sync>,
    ) -> Result<Child, SpawnError> {
        self.inner.spawn_instance(template, input)
    }
}

impl Cx<ServingPhase> {
    /// This invocation's serving episode, counting from 1.
    ///
    /// Episode 1 follows successful initialization. Each serving failure that
    /// the declared restart policy recovers starts the next episode. This
    /// counter is independent from initialization [`Cx::attempt`] numbers.
    pub fn episode(&self) -> u32 {
        self.inner.episode
    }
}

/// A `timeout` that expired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timeout;

impl std::fmt::Display for Timeout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("deadline expired")
    }
}
impl std::error::Error for Timeout {}
