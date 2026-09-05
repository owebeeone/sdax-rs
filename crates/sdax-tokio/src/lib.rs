//! `sdax-tokio` — the tokio adapter for [`sdax`].
//!
//! This is the only crate in the workspace that may mention tokio types, and
//! the only one that may spawn a raw task. Everywhere else a raw spawn creates
//! work the engine does not own (`X5`), which `clippy.toml` refuses; the two
//! call sites here carry a scoped `#[allow]` and say why.
//!
//! **Stage 0 delivers the [`Runtime`] contract**: spawning, task handles, the
//! clock and the observer. The run driver, the `Running` drop guard and the
//! drainer that runs a release graph after a drop are Stage 2, and no stub of
//! them exists here.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use sdax::host::{BoxFuture, Clock, Joined, NoObserver, Observer, Runtime, TaskHandle, Time};
use sdax::TraceEvent;
use std::sync::Arc;
use std::time::Duration;
use tokio::runtime::Handle;
use tokio_util::task::TaskTracker;

/// The tokio implementation of [`Runtime`].
///
/// Built from a [`Handle`], so a consumer keeps ownership of its own runtime
/// and this crate never creates one. Every task it spawns is registered in a
/// [`TaskTracker`], which is what lets [`close_and_wait`](Self::close_and_wait)
/// answer "is anything of mine still running?" — the property INV-15 ("no
/// orphans") rests on.
pub struct TokioRuntime {
    handle: Handle,
    clock: TokioClock,
    observer: Arc<dyn Observer>,
    tracker: TaskTracker,
}

impl TokioRuntime {
    /// An adapter over an existing runtime handle, recording nothing.
    pub fn new(handle: Handle) -> Self {
        TokioRuntime {
            handle,
            clock: TokioClock::default(),
            observer: Arc::new(NoObserver),
            tracker: TaskTracker::new(),
        }
    }

    /// Send trace events to this observer.
    ///
    /// Named `with_observer` rather than `observer`: the `Runtime` contract
    /// already has an `observer(&self)` reader, and one name cannot be both.
    pub fn with_observer(self, observer: Arc<dyn Observer>) -> Self {
        TokioRuntime { observer, ..self }
    }

    /// How many tasks this adapter spawned that have not finished.
    pub fn tracked(&self) -> usize {
        self.tracker.len()
    }

    /// Stop accepting new tasks and wait, up to `budget`, for the ones already
    /// spawned.
    ///
    /// `Ok(())` means everything finished. `Err(n)` means the budget expired
    /// with `n` tasks still running — the honest answer, not a silent success.
    pub async fn close_and_wait(&self, budget: Duration) -> Result<(), usize> {
        self.tracker.close();
        match tokio::time::timeout(budget, self.tracker.wait()).await {
            Ok(()) => Ok(()),
            Err(_) => Err(self.tracker.len()),
        }
    }

    /// The runtime handle this adapter was built from.
    pub fn handle(&self) -> &Handle {
        &self.handle
    }
}

impl Runtime for TokioRuntime {
    type Task = TokioTask;

    fn spawn(&self, fut: BoxFuture<'static, ()>) -> TokioTask {
        // The one place a raw spawn is correct: the task is tracked, so the
        // engine still owns it and `close_and_wait` can account for it.
        #[allow(clippy::disallowed_methods)]
        let handle = self.handle.spawn(self.tracker.track_future(fut));
        TokioTask { handle }
    }

    fn spawn_blocking(&self, f: Box<dyn FnOnce() + Send>) -> TokioTask {
        // Same exception, same reason. A blocking body cannot be aborted, so
        // the tracker is the only way to know it is still out there.
        #[allow(clippy::disallowed_methods)]
        let inner = self.handle.spawn_blocking(f);
        #[allow(clippy::disallowed_methods)]
        let handle = self.handle.spawn(self.tracker.track_future(async move {
            let _ = inner.await;
        }));
        TokioTask { handle }
    }

    fn clock(&self) -> &dyn Clock {
        &self.clock
    }

    fn observer(&self) -> &dyn Observer {
        self.observer.as_ref()
    }
}

/// A handle to one task this adapter spawned.
pub struct TokioTask {
    handle: tokio::task::JoinHandle<()>,
}

impl TaskHandle for TokioTask {
    fn abort(&self) {
        // A request, not an event: tokio's abort takes effect between polls,
        // which is why the engine always joins before treating a node as
        // settled (T5).
        self.handle.abort();
    }

    fn join(self) -> BoxFuture<'static, Joined> {
        Box::pin(async move {
            match self.handle.await {
                Ok(()) => Joined::Done,
                Err(e) if e.is_cancelled() => Joined::Cancelled,
                Err(e) => Joined::Panicked(e.into_panic()),
            }
        })
    }
}

/// The engine's clock, backed by tokio's timer.
///
/// Under `start_paused(true)` it is fully deterministic, which is how the
/// conformance suite runs against the real adapter without sleeping.
///
/// The origin is per clock and is taken on the first `now()`, inside whatever
/// runtime is driving it. A process-global origin would be wrong: two runtimes
/// with paused time have two unrelated clocks, and a `Time` from one says
/// nothing about the other.
#[derive(Debug, Default)]
pub struct TokioClock {
    origin: std::sync::OnceLock<tokio::time::Instant>,
}

impl TokioClock {
    fn origin(&self) -> tokio::time::Instant {
        *self.origin.get_or_init(tokio::time::Instant::now)
    }
}

impl Clock for TokioClock {
    fn now(&self) -> Time {
        let origin = self.origin();
        Time::from_nanos(
            tokio::time::Instant::now()
                .duration_since(origin)
                .as_nanos() as u64,
        )
    }

    fn sleep(&self, d: Duration) -> BoxFuture<'static, ()> {
        Box::pin(tokio::time::sleep(d))
    }
}

/// An observer that forwards to a closure.
pub struct FnObserver<F>(pub F);

impl<F: Fn(&TraceEvent) + Send + Sync> Observer for FnObserver<F> {
    fn event(&self, e: &TraceEvent) {
        (self.0)(e)
    }
}
