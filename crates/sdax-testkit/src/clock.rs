//! A clock a test advances by hand.

use sdax::{BoxFuture, Clock, Time};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// A [`Clock`] that only moves when a test moves it.
///
/// Every deadline in the engine — backoff, `within`, `stop_within`, the
/// shutdown budget — is measured on the injected clock, so a suite built on
/// this one never sleeps and never depends on wall-clock timing (LBT-008).
pub struct FakeClock {
    nanos: Arc<AtomicU64>,
}

impl FakeClock {
    /// A clock at its origin.
    pub fn new() -> Arc<Self> {
        Arc::new(FakeClock {
            nanos: Arc::new(AtomicU64::new(0)),
        })
    }

    /// A clock starting at `t`.
    pub fn starting_at(t: Time) -> Arc<Self> {
        Arc::new(FakeClock {
            nanos: Arc::new(AtomicU64::new(t.as_nanos())),
        })
    }

    /// Move time forward. Every sleep whose deadline has passed becomes ready
    /// on its next poll.
    pub fn advance(&self, d: Duration) {
        self.nanos.fetch_add(d.as_nanos() as u64, Ordering::SeqCst);
    }

    /// Move time forward to `t`, or do nothing if it is already past.
    pub fn advance_to(&self, t: Time) {
        let target = t.as_nanos();
        let mut cur = self.nanos.load(Ordering::SeqCst);
        while cur < target {
            match self
                .nanos
                .compare_exchange(cur, target, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => return,
                Err(actual) => cur = actual,
            }
        }
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Time {
        Time::from_nanos(self.nanos.load(Ordering::SeqCst))
    }

    fn sleep(&self, d: Duration) -> BoxFuture<'static, ()> {
        let nanos = self.nanos.clone();
        let deadline = nanos
            .load(Ordering::SeqCst)
            .saturating_add(d.as_nanos() as u64);
        Box::pin(std::future::poll_fn(move |_| {
            if nanos.load(Ordering::SeqCst) >= deadline {
                std::task::Poll::Ready(())
            } else {
                std::task::Poll::Pending
            }
        }))
    }
}
