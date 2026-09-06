//! `sdax-tokio` — the tokio adapter for [`sdax`].
//!
//! This is the only crate in the workspace that may mention tokio types, and
//! the only one that may spawn a raw task. Everywhere else a raw spawn creates
//! work the engine does not own (`X5`), which `clippy.toml` refuses; the three
//! call sites here — one in [`spawn`](Runtime::spawn) and two in
//! [`spawn_blocking`](Runtime::spawn_blocking), which spawns a pool job and a
//! tracked task to await it — carry a scoped `#[allow]` and say why.
//!
//! [`TokioRuntime`] is the [`Runtime`] contract on this substrate: spawning,
//! task handles, the clock and the observer. [`PlanStart`] is the run driver:
//! `plan.start(rt, input)` gives a [`Running`] whose drop cancels the run and
//! leaves one tracked drainer to finish the release graph. The input is the
//! run's own — `()` for a plan that declares none, a typed value for a plan
//! built with `Plan::with_input`.
//!
//! # What this substrate adds to the contract
//!
//! Three facts about tokio that the engine's own rules do not imply, and that
//! a supervisor has to know:
//!
//! - **A blocking body that never returns blocks `Runtime::drop` for ever.**
//!   `spawn_blocking` work cannot be aborted (T7, T7c): the engine abandons
//!   the node at the budget and says so — [`shutdown`](TokioRuntime::shutdown)
//!   answers `Err(n)`, honestly — but `tokio::runtime::Runtime`'s own `Drop`
//!   waits indefinitely for pool work, so a supervisor that drops its runtime
//!   after such a run hangs there. `shutdown_timeout` and
//!   `shutdown_background` do not.
//! - **The drainer needs a driver thread.** Dropping a live [`Running`] leaves
//!   the release graph to the driver task, which is a task like any other. On
//!   a `current_thread` runtime it makes no progress between `block_on` calls:
//!   drop a `Running` inside a `block_on` that then returns and nothing is
//!   released until the next one. Give the drainer its budget inside a
//!   `block_on`, or use a multi-threaded runtime. This one is named at the
//!   constructor rather than left to a page: [`TokioRuntime::new`] takes a
//!   multi-threaded handle and a `current_thread` handle goes through
//!   [`TokioRuntime::current_thread_no_background_drain`], which says what the
//!   caller is taking on. Tearing the runtime down under it is reported, never
//!   silent (`TraceKind::RuntimeDroppedWithLiveRuns`).
//! - **Panics are caught only if the profile unwinds.** The engine's promise
//!   that a body panic is a fault and never re-raised rests on `catch_unwind`.
//!   Under `panic = "abort"` there is nothing to catch: a body panic aborts the
//!   process, and `FaultKind::Panic` is unreachable. Nothing in this workspace
//!   sets that profile.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod body;
mod driver;
mod running;
mod scope;

pub use running::{PlanStart, RunHandle, RunOptions, RunRecord, Running, Snapshot, WhyAt};
pub use scope::InstanceEnded;

use sdax::host::{BoxFuture, Clock, Joined, NoObserver, Observer, Runtime, TaskHandle, Time};
use sdax::TraceEvent;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::runtime::{Handle, RuntimeFlavor};
use tokio_util::task::TaskTracker;

/// The tokio implementation of [`Runtime`].
///
/// Built from a [`Handle`], so a consumer keeps ownership of its own runtime
/// and this crate never creates one. Every task it spawns is registered in a
/// [`TaskTracker`], which is what lets [`shutdown`](Self::shutdown)
/// answer "is anything of mine still running?" — the property INV-15 ("no
/// orphans") rests on.
pub struct TokioRuntime {
    handle: Handle,
    clock: Arc<dyn Clock>,
    seen: Arc<AtomicU64>,
    observer: Arc<dyn Observer>,
    tracker: TaskTracker,
}

impl TokioRuntime {
    /// An adapter over an existing **multi-threaded** runtime handle,
    /// recording nothing.
    ///
    /// A `current_thread` handle goes through
    /// [`current_thread_no_background_drain`](Self::current_thread_no_background_drain)
    /// instead, whose name states what this one may assume: that a drainer
    /// left behind by a dropped [`Running`] runs. The flavour is a property of
    /// the handle, so it is read once, here, rather than re-derived per run
    /// (`OD-CT-DRAIN`).
    ///
    /// A caller that does not know its flavour statically dispatches on it:
    ///
    /// ```
    /// use sdax_tokio::TokioRuntime;
    /// use tokio::runtime::{Handle, RuntimeFlavor};
    ///
    /// fn adapter(handle: Handle) -> TokioRuntime {
    ///     match handle.runtime_flavor() {
    ///         RuntimeFlavor::CurrentThread => {
    ///             TokioRuntime::current_thread_no_background_drain(handle)
    ///         }
    ///         _ => TokioRuntime::new(handle),
    ///     }
    /// }
    /// ```
    ///
    /// # Panics
    ///
    /// If `handle` names a `current_thread` runtime. That is a programmer
    /// error settled at construction — no value the process could compute
    /// changes the answer, and the flavour cannot be converted — so the panic
    /// names the constructor that takes one rather than returning an error
    /// nobody could act on differently.
    pub fn new(handle: Handle) -> Self {
        if matches!(handle.runtime_flavor(), RuntimeFlavor::CurrentThread) {
            panic!(
                "TokioRuntime::new was handed a current_thread runtime handle. \
                 A dropped `Running` hands its release graph to a drainer, and a \
                 drainer is a task: on a current_thread runtime it makes no progress \
                 between `block_on` calls, so a dropped run is left un-released in \
                 silence. Build the adapter with \
                 `TokioRuntime::current_thread_no_background_drain(handle)`, which says \
                 so at the call site, and always await `Running::shutdown()`."
            );
        }
        Self::build(handle)
    }

    /// An adapter whose name is the acknowledgement: **a dropped [`Running`]
    /// cannot drain here, so always await [`Running::shutdown`].**
    ///
    /// Dropping a `Running` cancels the run, aborts its bodies and leaves the
    /// release graph to a drainer. The drainer is a task like any other. On a
    /// `current_thread` runtime a task makes no progress between `block_on`
    /// calls, so a `Running` dropped inside a `block_on` that then returns
    /// releases nothing until the next one — and a process that never enters
    /// another leaks the run's scope without a word. Nothing here fixes that;
    /// this constructor exists so the exposure is named where the runtime is
    /// chosen instead of discovered in a teardown that produces no output.
    ///
    /// **What the caller owes.** End every run explicitly — await
    /// [`Running::shutdown`] (or `cancel()`, or the run's own end) inside the
    /// `block_on` that started it, and then await
    /// [`TokioRuntime::shutdown`](Self::shutdown) to see whether anything is
    /// still out there. A current-thread run that does this drains inside the
    /// caller's own await and is entirely correct: the hazard is *drop*, not
    /// `current_thread`, which is why this is an acknowledgement and not a
    /// refusal.
    ///
    /// A multi-threaded handle is accepted too — the promise this constructor
    /// asks for is correct on every flavour, and refusing the safe direction
    /// would refuse legitimate use for no gain — but [`new`](Self::new) is the
    /// constructor for one, and says less.
    pub fn current_thread_no_background_drain(handle: Handle) -> Self {
        Self::build(handle)
    }

    /// The adapter itself, once a constructor has settled what the caller
    /// knows about the flavour. Not public: every way in states its terms.
    fn build(handle: Handle) -> Self {
        let seen = Arc::new(AtomicU64::new(0));
        TokioRuntime {
            handle,
            clock: Arc::new(TokioClock::new(seen.clone())),
            seen,
            observer: Arc::new(NoObserver),
            tracker: TaskTracker::new(),
        }
    }

    /// Measure every deadline on this clock instead of tokio's own.
    ///
    /// The engine only ever reads time through [`Clock`], so a run can be put
    /// on a compressed clock — which is how the suite runs against a
    /// multi-threaded runtime, where `start_paused` is not available.
    ///
    /// The clock is wrapped so that it still records its readings for
    /// [`Drop`](Self::drop), which has no runtime to read a fresh time from.
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = Arc::new(SeenClock {
            inner: clock,
            seen: self.seen.clone(),
        });
        self
    }

    /// Send trace events to this observer.
    ///
    /// Named `with_observer` rather than `observer`: the `Runtime` contract
    /// already has an `observer(&self)` reader, and one name cannot be both.
    pub fn with_observer(mut self, observer: Arc<dyn Observer>) -> Self {
        self.observer = observer;
        self
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
    ///
    /// `Err(n)` on a run with a blocking body is not always recoverable: a
    /// blocking body cannot be aborted (T7), so a body that never returns
    /// keeps its thread for ever and, unlike this call, `Runtime::drop` waits
    /// for it without a budget. See the crate docs.
    ///
    /// This is the *runtime's* shutdown, not a run's: it is what a process
    /// calls after every [`Running`] has ended or been dropped, to see whether
    /// any drainer is still out there. Stage 0 left the name open
    /// (`close_and_wait`); there is one operation, so there is one name.
    pub async fn shutdown(&self, budget: Duration) -> Result<(), usize> {
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

impl Drop for TokioRuntime {
    /// A runtime dropped with tasks of ours still running is reported, never
    /// silent: whatever they were doing, nobody is left to join them (INV-15).
    ///
    /// The time is the last reading anything took from this clock, not a fresh
    /// one: `Drop` can run on a thread with no runtime and on a clock of the
    /// caller's own ([`with_clock`](Self::with_clock)), and a reading taken
    /// there would not be on the same scale as the run's. This is why a custom
    /// clock is wrapped rather than stored as given.
    ///
    /// This covers the order in which the adapter is dropped before the
    /// runtime. The other order — the runtime torn down first, taking the
    /// driver task with it — is covered by the run driver's own `Drop`, which
    /// reports the same event and hands over the report.
    fn drop(&mut self) {
        if self.tracker.is_empty() {
            return;
        }
        let at = Time::from_nanos(self.seen.load(Ordering::SeqCst));
        let ev = TraceEvent::at(at, sdax::TraceKind::RuntimeDroppedWithLiveRuns);
        // § 10 forbids an observer to panic; one that does must not turn a
        // report of a lost run into a panic out of a `Drop`.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.observer.event(&ev)));
    }
}

impl Runtime for TokioRuntime {
    type Task = TokioTask;

    fn spawn(&self, fut: BoxFuture<'static, ()>) -> TokioTask {
        // The one place a raw spawn is correct: the task is tracked, so the
        // engine still owns it and `shutdown` can account for it.
        #[allow(clippy::disallowed_methods)]
        let handle = self.handle.spawn(self.tracker.track_future(fut));
        TokioTask {
            handle,
            blocking: false,
        }
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
        TokioTask {
            handle,
            blocking: true,
        }
    }

    fn clock(&self) -> &dyn Clock {
        self.clock.as_ref()
    }

    fn observer(&self) -> &dyn Observer {
        self.observer.as_ref()
    }
}

/// A handle to one task this adapter spawned.
pub struct TokioTask {
    handle: tokio::task::JoinHandle<()>,
    /// A pool job, awaited by the tracked task `handle` names. A thread cannot
    /// be taken back, so this handle has no abort to offer.
    blocking: bool,
}

impl TaskHandle for TokioTask {
    fn abort(&self) {
        if self.blocking {
            // T7: a `spawn_blocking` thread cannot be aborted. Aborting the
            // tracked task that awaits it would abort only the *wrapper* and
            // detach the thread, so `tracked()` would read zero and
            // `shutdown()` would say `Ok` while the thread worked on — the one
            // claim INV-15 rests on, false. The request is dropped instead and
            // the wrapper stays until the thread returns, which is what the
            // engine's own abandonment already assumes (T7c).
            return;
        }
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
    seen: Arc<AtomicU64>,
}

impl TokioClock {
    /// A clock that also records its last reading in `seen`, so that
    /// [`TokioRuntime`]'s `Drop` can timestamp its event without touching
    /// tokio's clock from outside a runtime.
    pub fn new(seen: Arc<AtomicU64>) -> Self {
        TokioClock {
            origin: std::sync::OnceLock::new(),
            seen,
        }
    }

    fn origin(&self) -> tokio::time::Instant {
        *self.origin.get_or_init(tokio::time::Instant::now)
    }
}

impl Clock for TokioClock {
    fn now(&self) -> Time {
        let origin = self.origin();
        let nanos = tokio::time::Instant::now()
            .duration_since(origin)
            .as_nanos() as u64;
        self.seen.fetch_max(nanos, Ordering::SeqCst);
        Time::from_nanos(nanos)
    }

    fn sleep(&self, d: Duration) -> BoxFuture<'static, ()> {
        Box::pin(tokio::time::sleep(d))
    }
}

/// A [`Clock`] of the caller's own, still recording its readings.
///
/// [`TokioRuntime::drop`] timestamps its event from the last reading anything
/// took, because it may run where no clock can be read. A custom clock handed
/// to [`with_clock`](TokioRuntime::with_clock) is wrapped in this so that the
/// reading is the run's own, not `Time::ZERO`.
struct SeenClock {
    inner: Arc<dyn Clock>,
    seen: Arc<AtomicU64>,
}

impl Clock for SeenClock {
    fn now(&self) -> Time {
        let t = self.inner.now();
        self.seen.fetch_max(t.as_nanos(), Ordering::SeqCst);
        t
    }

    fn sleep(&self, d: Duration) -> BoxFuture<'static, ()> {
        self.inner.sleep(d)
    }
}

/// An observer that forwards to a closure.
pub struct FnObserver<F>(pub F);

impl<F: Fn(&TraceEvent) + Send + Sync> Observer for FnObserver<F> {
    fn event(&self, e: &TraceEvent) {
        (self.0)(e)
    }
}
