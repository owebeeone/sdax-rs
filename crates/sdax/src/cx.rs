//! The seam: the complete list of ways a node body can affect lifecycle state.
//!
//! A body is ordinary `async` Rust and nothing about it is sandboxed. What is
//! bounded is its *interface* to the engine, and this module is that interface:
//! [`Cx`] carries a phase marker so an operation that makes no sense in a phase
//! does not exist there, [`Held`] cannot be forged, and [`Serving`] is the only
//! way a service becomes ready.
//!
//! **The one rule of the body contract**: perform external effects *inside*
//! `cx.hold(..)`. `hold` registers the value in the same poll that observes the
//! effect completing, so there is no await point between "the effect happened"
//! and "the engine owns the cleanup". A body that performs the effect itself,
//! awaits something else, and then calls [`Cx::hold_value`] has re-created that
//! window by hand; the engine cannot see it (`Proposal.md` LG-2 limit).
//!
//! A service's own acquisitions belong in a resource node, not in its `start`
//! body: values a start body creates have no ledger entry and no async release
//! (`AdversarialReview.md` F-B6). Use `needs` on a resource, or keep the value
//! in the serve future's locals and drop it there.

mod instances;

pub use instances::{Child, ChildControl, InstanceId, Scope, SpawnError, Stop, StopSignal};

use crate::contracts::{BoxFuture, Clock, Error, Time};
use crate::key::RawKey;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

/// Phase marker: a resource's `acquire` body and an effect's `perform` body.
/// The only phase with [`Cx::hold`].
#[derive(Debug, Clone, Copy)]
pub struct Acquire;
/// Phase marker: a step's, try-step's or blocking step's `run` body.
#[derive(Debug, Clone, Copy)]
pub struct Run;
/// Phase marker: a service's `start` body. Readiness is its *return*.
#[derive(Debug, Clone, Copy)]
pub struct Start;
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
    clock: Arc<dyn Clock>,
    stop: Arc<StopSignal>,
    scope: Option<Arc<dyn Scope>>,
    held: Mutex<Option<Box<dyn std::any::Any + Send + Sync>>>,
    serve: Mutex<Option<BoxFuture<'static, Result<(), Error>>>>,
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
            clock,
            stop: StopSignal::new(),
            scope: None,
            held: Mutex::new(None),
            serve: Mutex::new(None),
            holds: AtomicU32::new(0),
            deadline: AtomicU64::new(u64::MAX),
        })
    }

    fn rebuilt(&self) -> CxInner {
        CxInner {
            node: self.node,
            attempt: self.attempt,
            clock: self.clock.clone(),
            stop: self.stop.clone(),
            scope: self.scope.clone(),
            held: Mutex::new(None),
            serve: Mutex::new(None),
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

    /// Which node this context belongs to.
    pub fn node(&self) -> RawKey {
        self.node
    }

    /// The stop signal this context observes.
    pub fn stop_signal(&self) -> &Arc<StopSignal> {
        &self.stop
    }

    /// How many values the body registered in this attempt. More than one is
    /// [`FaultKind::DoubleHold`](crate::FaultKind::DoubleHold).
    pub fn hold_count(&self) -> u32 {
        self.holds.load(Ordering::SeqCst)
    }

    /// Take what the body registered. The engine calls this once the body's
    /// future has settled, whatever the outcome (INV-3).
    pub fn take_held(&self) -> Option<Box<dyn std::any::Any + Send + Sync>> {
        self.held.lock().expect("cx poisoned").take()
    }

    /// Store an already-erased output. Used by the step and service terminals,
    /// whose outputs the engine registers on `Ok` rather than the body.
    pub fn put_output(&self, v: Box<dyn std::any::Any + Send + Sync>) {
        *self.held.lock().expect("cx poisoned") = Some(v);
    }

    /// Store the serve future a start body returned, for the engine to drive.
    pub fn put_serve(&self, serve: BoxFuture<'static, Result<(), Error>>) {
        *self.serve.lock().expect("cx poisoned") = Some(serve);
    }

    /// Take the serve future the start body handed over.
    pub fn take_serve(&self) -> Option<BoxFuture<'static, Result<(), Error>>> {
        self.serve.lock().expect("cx poisoned").take()
    }

    fn register<T: ?Sized + Send + Sync + 'static>(&self, arc: Arc<T>) {
        *self.held.lock().expect("cx poisoned") = Some(Box::new(arc));
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

impl<P> Clone for Cx<P> {
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
    pub fn new(inner: Arc<CxInner>) -> Self {
        Cx {
            inner,
            _p: PhantomData,
        }
    }

    /// The engine state behind this context. Host API, like [`Cx::new`].
    pub fn inner(&self) -> Arc<CxInner> {
        self.inner.clone()
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
        input: Box<dyn std::any::Any + Send>,
    ) -> Result<Child, SpawnError> {
        match &self.inner.scope {
            Some(scope) => scope.spawn_instance(self.inner.node, template, input),
            None => Err(SpawnError::NotRunning),
        }
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

// ---------------------------------------------------------------- held values

/// Proof that the engine owns the release (or compensation) obligation for a
/// value. No public constructor: only [`Cx::hold`] and [`Cx::hold_value`] mint
/// one, and both write the engine's record before handing it back.
pub struct Held<T: ?Sized> {
    arc: Arc<T>,
    node: RawKey,
}

impl<T: ?Sized> Held<T> {
    /// Which node registered this value.
    pub fn node(&self) -> RawKey {
        self.node
    }
}

impl<T: ?Sized> std::ops::Deref for Held<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.arc
    }
}

impl<T: ?Sized> std::fmt::Debug for Held<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Held(node {}/{})", self.node.plan, self.node.idx)
    }
}

impl Cx<Acquire> {
    /// Run an external effect under the engine's wrapper.
    ///
    /// The poll that observes `effect` completing writes the engine's record
    /// and *then* returns `Ready`, so no await point exists between the effect
    /// and the obligation. Cancellation can only take effect between polls, so
    /// the X1 window ("acquired, but nobody owns the cleanup") does not exist
    /// in this form. Nothing is registered while the effect is pending, and
    /// nothing is registered when it fails.
    pub fn hold<F, V, E, T>(&self, effect: F) -> Hold<F, T>
    where
        F: Future<Output = Result<V, E>> + Send + 'static,
        V: Into<Arc<T>>,
        E: Into<Error>,
        T: ?Sized + Send + Sync + 'static,
    {
        Hold {
            effect: Box::pin(effect),
            cx: self.inner.clone(),
            _t: PhantomData,
        }
    }

    /// Register a value that carries no external effect — a computed handle, a
    /// boxed adapter, a value the caller already owns.
    ///
    /// Registration happens before this returns. Performing an external effect
    /// outside the wrapper and registering it afterwards re-creates the X1
    /// window by hand; that is escape misuse, not the contract.
    pub fn hold_value<T: ?Sized + Send + Sync + 'static>(&self, v: impl Into<Arc<T>>) -> Held<T> {
        let arc: Arc<T> = v.into();
        self.inner.register(arc.clone());
        Held {
            arc,
            node: self.inner.node,
        }
    }
}

/// The future returned by [`Cx::hold`]. Registration happens inside `poll`.
pub struct Hold<F, T: ?Sized> {
    effect: Pin<Box<F>>,
    cx: Arc<CxInner>,
    _t: PhantomData<fn() -> Arc<T>>,
}

impl<F, V, E, T> Future for Hold<F, T>
where
    F: Future<Output = Result<V, E>> + Send + 'static,
    V: Into<Arc<T>>,
    E: Into<Error>,
    T: ?Sized + Send + Sync + 'static,
{
    type Output = Result<Held<T>, Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.effect.as_mut().poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(e)) => Poll::Ready(Err(e.into())),
            Poll::Ready(Ok(v)) => {
                let arc: Arc<T> = v.into();
                // Same poll as the completion: no await between effect and record.
                self.cx.register(arc.clone());
                let node = self.cx.node;
                Poll::Ready(Ok(Held { arc, node }))
            }
        }
    }
}

// ---------------------------------------------------------------- serving

/// A service's readiness hand-off: the handle dependents receive, plus the
/// future that does the serving.
///
/// Readiness is the *return* of the start body with this value, so "ready
/// because it was spawned" cannot be written (INV-2).
pub struct Serving<H> {
    handle: H,
    serve: BoxFuture<'static, Result<(), Error>>,
}

impl<H> Serving<H> {
    /// Hand the serve future to the engine and declare the node ready.
    pub fn new(handle: H, serve: impl Future<Output = Result<(), Error>> + Send + 'static) -> Self {
        Serving {
            handle,
            serve: Box::pin(serve),
        }
    }

    /// Split into the dependents' handle and the serve future.
    pub fn into_parts(self) -> (H, BoxFuture<'static, Result<(), Error>>) {
        (self.handle, self.serve)
    }
}

impl<H: std::fmt::Debug> std::fmt::Debug for Serving<H> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Serving")
            .field("handle", &self.handle)
            .finish_non_exhaustive()
    }
}
