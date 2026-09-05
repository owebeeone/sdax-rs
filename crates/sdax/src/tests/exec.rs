//! A poll-driver for the Stage 0 tests. Not an executor: it polls a future a
//! bounded number of times with a no-op waker and never sleeps (LBT-008).

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

struct NoopWake;
impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

/// A waker that does nothing. (`Waker::noop()` is newer than the crate's MSRV.)
pub fn noop_waker() -> Waker {
    Waker::from(Arc::new(NoopWake))
}

/// Poll a pinned future once.
pub fn poll_once<F: Future + ?Sized>(fut: Pin<&mut F>) -> Poll<F::Output> {
    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);
    fut.poll(&mut cx)
}

/// Poll a future until it is ready, at most `limit` times. Panics if it is
/// still pending; a Stage 0 test must never depend on a real wake-up.
pub fn poll_n<T>(mut fut: Pin<Box<dyn Future<Output = T> + Send>>, limit: usize) -> T {
    for _ in 0..limit {
        if let Poll::Ready(v) = poll_once(fut.as_mut()) {
            return v;
        }
    }
    panic!("future still pending after {limit} polls");
}

/// Poll a future to completion, at most 16 times.
pub fn block_on<T>(fut: impl Future<Output = T> + Send + 'static) -> T {
    poll_n(Box::pin(fut), 16)
}

/// A future that is `Pending` on its first poll and `Ready(v)` on the second,
/// so a test can observe what happens across a real suspension point.
pub async fn pending_once<T>(v: T) -> T {
    let mut polled = false;
    std::future::poll_fn(move |_| {
        if polled {
            Poll::Ready(())
        } else {
            polled = true;
            Poll::Pending
        }
    })
    .await;
    v
}
