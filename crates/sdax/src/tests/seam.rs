//! The seam API on `Cx<Phase>` (`Proposal.md` § A.3 table).
//!
//! `hold_registers_in_the_completing_poll` re-witnesses assertion A9 of
//! `sdax-v1/B/experiments/typed_keys` against this crate's own types.

use super::exec::{block_on, pending_once, poll_once};
use crate::host::{BoxFuture, Clock, CxInner, RawKey, Time};
use crate::*;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

/// A clock a test drives by hand. The testkit ships the published `FakeClock`;
/// this one keeps the core's own tests free of a dev-dependency cycle.
pub struct TestClock {
    nanos: Arc<AtomicU64>,
}

impl TestClock {
    pub fn new() -> Arc<Self> {
        Arc::new(TestClock {
            nanos: Arc::new(AtomicU64::new(0)),
        })
    }
    pub fn advance(&self, d: Duration) {
        self.nanos.fetch_add(d.as_nanos() as u64, Ordering::SeqCst);
    }
}

impl Clock for TestClock {
    fn now(&self) -> Time {
        Time::from_nanos(self.nanos.load(Ordering::SeqCst))
    }
    fn sleep(&self, d: Duration) -> BoxFuture<'static, ()> {
        let nanos = self.nanos.clone();
        let deadline = self.nanos.load(Ordering::SeqCst) + d.as_nanos() as u64;
        Box::pin(std::future::poll_fn(move |_| {
            if nanos.load(Ordering::SeqCst) >= deadline {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        }))
    }
}

fn cx_for_test(clock: Arc<dyn Clock>) -> Cx<Acquire> {
    Cx::new(CxInner::new(RawKey { plan: 1, idx: 0 }, clock))
}

#[test]
fn hold_registers_in_the_completing_poll_and_not_before() {
    let cx = cx_for_test(TestClock::new());
    let inner = cx.inner();
    let mut fut: Pin<Box<dyn std::future::Future<Output = Result<Held<u8>, Error>> + Send>> =
        Box::pin(cx.hold(|| pending_once(Ok::<u8, std::io::Error>(9))));

    assert!(matches!(poll_once(fut.as_mut()), Poll::Pending));
    assert_eq!(
        inner.hold_count(),
        0,
        "nothing registered while the effect is in flight"
    );
    let held = match poll_once(fut.as_mut()) {
        Poll::Ready(Ok(h)) => h,
        other => panic!(
            "expected Ready(Ok), got {}",
            if other.is_pending() { "Pending" } else { "Err" }
        ),
    };
    assert_eq!(*held, 9);
    assert_eq!(
        inner.hold_count(),
        1,
        "registered in the poll that observed completion"
    );
    assert_eq!(
        *inner.take_held().unwrap().downcast::<Arc<u8>>().unwrap(),
        Arc::new(9u8)
    );
}

#[test]
fn a_failed_effect_registers_nothing() {
    let cx = cx_for_test(TestClock::new());
    let inner = cx.inner();
    let err =
        block_on(cx.hold(|| pending_once(Err::<u8, _>(std::io::Error::other("no"))))).unwrap_err();
    assert_eq!(err.to_string(), "no");
    assert_eq!(inner.hold_count(), 0);
    assert!(inner.take_held().is_none());
}

#[test]
fn hold_value_registers_before_it_returns() {
    let cx = cx_for_test(TestClock::new());
    let inner = cx.inner();
    let held = cx.hold_value(7u8);
    assert_eq!(*held, 7);
    assert_eq!(inner.hold_count(), 1);
}

#[test]
fn hold_carries_unsized_outputs() {
    trait Port: Send + Sync {
        fn name(&self) -> &'static str;
    }
    struct P;
    impl Port for P {
        fn name(&self) -> &'static str {
            "p"
        }
    }
    let cx = cx_for_test(TestClock::new());
    let boxed: Box<dyn Port> = Box::new(P);
    let held: Held<dyn Port> = cx.hold_value(boxed);
    assert_eq!(held.name(), "p");
}

#[test]
fn stop_is_observable_three_ways() {
    let clock = TestClock::new();
    let inner = CxInner::new(RawKey { plan: 1, idx: 0 }, clock);
    let cx: Cx<Run> = Cx::new(inner.clone());
    assert!(!cx.is_stopping());

    let mut stop = cx.stop();
    assert!(matches!(poll_once(Pin::new(&mut stop)), Poll::Pending));

    inner.stop_signal().request();
    assert!(cx.is_stopping());
    assert!(matches!(poll_once(Pin::new(&mut stop)), Poll::Ready(())));
}

#[test]
fn until_stop_yields_none_when_the_scope_is_stopping() {
    let inner = CxInner::new(RawKey { plan: 1, idx: 0 }, TestClock::new());
    let cx: Cx<Start> = Cx::new(inner.clone());
    inner.stop_signal().request();
    let out: Option<u8> =
        block_on(async move { cx.until_stop(std::future::pending::<u8>()).await });
    assert_eq!(out, None);

    let cx2: Cx<Start> = Cx::new(CxInner::new(RawKey { plan: 1, idx: 0 }, TestClock::new()));
    let out2 = block_on(async move { cx2.until_stop(async { 5u8 }).await });
    assert_eq!(out2, Some(5));
}

#[test]
fn now_and_sleep_use_the_injected_clock() {
    let clock = TestClock::new();
    let inner = CxInner::new(RawKey { plan: 1, idx: 0 }, clock.clone());
    let cx: Cx<Release> = Cx::new(inner);
    assert_eq!(cx.now(), Time::from_nanos(0));
    clock.advance(Duration::from_secs(3));
    assert_eq!(cx.now(), Time::from_nanos(3_000_000_000));
    // The clock is injected: no test sleeps, and an unadvanced sleep never completes.
    let mut sleeping = Box::pin(cx.sleep(Duration::from_secs(1)));
    assert!(matches!(poll_once(sleeping.as_mut()), Poll::Pending));
}

#[test]
fn timeout_reports_expiry_against_the_injected_clock() {
    let clock = TestClock::new();
    let cx: Cx<Release> = Cx::new(CxInner::new(RawKey { plan: 1, idx: 0 }, clock.clone()));
    let ready = block_on(async move { cx.timeout(Duration::from_secs(1), async { 4u8 }).await });
    assert_eq!(ready, Ok(4));

    let clock2 = TestClock::new();
    let cx2: Cx<Release> = Cx::new(CxInner::new(RawKey { plan: 1, idx: 0 }, clock2.clone()));
    let mut fut = Box::pin(async move {
        cx2.timeout(Duration::from_secs(1), std::future::pending::<u8>())
            .await
    });
    assert!(matches!(poll_once(fut.as_mut()), Poll::Pending));
    clock2.advance(Duration::from_secs(2));
    assert_eq!(poll_once(fut.as_mut()), Poll::Ready(Err(Timeout)));
}

#[test]
fn attempt_is_readable_and_starts_at_one() {
    let inner = CxInner::new(RawKey { plan: 1, idx: 0 }, TestClock::new());
    let cx: Cx<Acquire> = Cx::new(inner);
    assert_eq!(cx.attempt(), 1);
    let retried: Cx<Acquire> =
        Cx::new(CxInner::new(RawKey { plan: 1, idx: 0 }, TestClock::new()).with_attempt(3));
    assert_eq!(retried.attempt(), 3);
}

#[test]
fn serving_phase_reports_the_current_episode() {
    let inner = CxInner::new(RawKey { plan: 1, idx: 0 }, TestClock::new()).with_episode(3);
    let cx: Cx<ServingPhase> = Cx::new(inner);
    assert_eq!(cx.episode(), 3);
}

#[test]
fn spawn_without_an_attached_run_is_refused_rather_than_panicking() {
    // Stage 0 has no engine: no scope is attached, so the seam says so.
    let inner = CxInner::new(RawKey { plan: 1, idx: 0 }, TestClock::new());
    let cx: Cx<Start> = Cx::new(inner);
    let err = cx
        .spawn_raw(RawKey { plan: 1, idx: 4 }, Box::new(()))
        .unwrap_err();
    assert_eq!(err, SpawnError::NotRunning);
}

#[test]
fn safety_second_registration_preserves_first_obligation() {
    let inner = CxInner::new(RawKey { plan: 1, idx: 0 }, TestClock::new());
    let first: Cx<Acquire> = Cx::new(inner.clone());
    let second: Cx<Acquire> = Cx::new(inner.clone());
    let _held = first.hold_value(7u8);
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| second.hold_value(9u8)));
    assert_eq!(
        **inner.take_held().unwrap().downcast::<Arc<u8>>().unwrap(),
        7
    );
}

#[test]
fn safety_rejected_factory_never_runs_even_while_first_is_reserved() {
    let inner = CxInner::new(RawKey { plan: 1, idx: 0 }, TestClock::new());
    let first = inner
        .acquire()
        .hold(std::future::pending::<Result<u8, Error>>);
    let called = std::sync::atomic::AtomicBool::new(false);
    let rejected = inner.acquire().hold(|| {
        called.store(true, Ordering::SeqCst);
        async { Ok::<u8, Error>(9) }
    });
    assert!(!called.load(Ordering::SeqCst));
    assert!(block_on(rejected).is_err());
    assert_eq!(inner.hold_count(), 0);
    drop(first);
    assert!(block_on(inner.acquire().hold(|| async { Ok::<u8, Error>(3) })).is_err());
}
