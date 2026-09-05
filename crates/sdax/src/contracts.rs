//! The contracts the core needs from a host (LBT-002): a clock, a task
//! spawner, task handles, and an observer.
//!
//! The core is std-only and executes nothing. Everything time-shaped goes
//! through [`Clock`] so that tests are deterministic and never sleep
//! (LBT-008), and everything task-shaped goes through [`Runtime`] so that the
//! only crate that mentions `tokio` is `sdax-tokio`.
//!
//! Futures are boxed in every trait signature: the crate's MSRV predates
//! `async fn` in traits, and these traits must stay usable as `dyn`.

use crate::report::TraceEvent;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

/// The seam's error type. One boxed error so `?` works in bodies without
/// annotations, and typed errors survive as `downcast_ref` targets on a
/// [`Fault`](crate::Fault).
pub type Error = Box<dyn std::error::Error + Send + Sync + 'static>;

/// A boxed, `Send` future — the shape every trait method here returns.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// A point on the engine's clock, in nanoseconds from that clock's origin.
///
/// Not a wall clock and not `std::time::Instant`: a [`Clock`] implementation
/// chooses the origin, so a fake clock starts at zero and a run's trace is
/// reproducible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Time(u64);

impl Time {
    /// The clock's origin.
    pub const ZERO: Time = Time(0);

    /// A time this many nanoseconds after the origin.
    pub fn from_nanos(nanos: u64) -> Time {
        Time(nanos)
    }

    /// Nanoseconds since the origin.
    pub fn as_nanos(&self) -> u64 {
        self.0
    }

    /// How long after `earlier` this time is, or `None` if it is not after it.
    pub fn checked_duration_since(&self, earlier: Time) -> Option<Duration> {
        self.0.checked_sub(earlier.0).map(Duration::from_nanos)
    }
}

impl std::ops::Add<Duration> for Time {
    type Output = Time;
    fn add(self, d: Duration) -> Time {
        Time(self.0.saturating_add(d.as_nanos() as u64))
    }
}

impl std::fmt::Display for Time {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", Duration::from_nanos(self.0))
    }
}

/// The engine's source of time.
///
/// Required operations: read the current time, and produce a future that
/// completes once a duration has elapsed on *this* clock. Backoff waits,
/// `within` deadlines, `stop_within` and the shutdown budget are all measured
/// on it; a conforming implementation must never consult a different clock.
pub trait Clock: Send + Sync {
    /// The current time on this clock.
    fn now(&self) -> Time;

    /// A future that completes `d` after the call, on this clock.
    fn sleep(&self, d: Duration) -> BoxFuture<'static, ()>;
}

/// What a task ended as, once joined.
#[derive(Debug)]
pub enum Joined {
    /// The task's future returned.
    Done,
    /// The task was aborted before it returned.
    Cancelled,
    /// The task's future panicked; the payload is carried, never re-raised by
    /// the engine.
    Panicked(Box<dyn std::any::Any + Send>),
}

/// A handle to one task the engine spawned.
///
/// `abort` is a *request*: on the tokio substrate it takes effect between
/// polls, so the engine always [`join`](TaskHandle::join)s before treating a
/// node as settled (T5, INV-15).
pub trait TaskHandle: Send + 'static {
    /// Ask the runtime to stop polling this task.
    fn abort(&self);

    /// Wait for the task to finish and report how it ended.
    fn join(self) -> BoxFuture<'static, Joined>;
}

/// Everything the engine needs from a host runtime.
///
/// Implemented once per substrate; `sdax-tokio` is the reference
/// implementation and the only crate permitted to spawn raw tasks.
pub trait Runtime: Send + Sync + 'static {
    /// The handle type this runtime hands back for a spawned task.
    type Task: TaskHandle;

    /// Spawn an async task.
    fn spawn(&self, fut: BoxFuture<'static, ()>) -> Self::Task;

    /// Spawn a synchronous, possibly blocking, closure off the async workers.
    fn spawn_blocking(&self, f: Box<dyn FnOnce() + Send>) -> Self::Task;

    /// The clock every deadline in this run is measured on.
    fn clock(&self) -> &dyn Clock;

    /// Where trace events go.
    fn observer(&self) -> &dyn Observer;
}

/// A sink for trace events. Called from engine context; an implementation must
/// not block and must not panic.
pub trait Observer: Send + Sync {
    /// Record one event.
    fn event(&self, e: &TraceEvent);

    /// The run's report, once it has ended.
    ///
    /// The driver calls this on every run, awaited or dropped: a dropped
    /// [`Running`](../../sdax_tokio/struct.Running.html) has nobody left to
    /// hand a report to, and losing it would break INV-9 exactly when the run
    /// went least well (`C-14`). The default does nothing, so an observer that
    /// only wants events is unaffected.
    fn report(&self, _r: &crate::report::Report<()>) {}
}

/// An observer that drops every event.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoObserver;

impl Observer for NoObserver {
    fn event(&self, _e: &TraceEvent) {}
}
