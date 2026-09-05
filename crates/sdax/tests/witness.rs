//! Suite (a), the compile-pass half (`CanonicalTests.md` W-14, W-15).
//!
//! This test sees the crate from outside, exactly as an author does: nothing
//! `pub(crate)` is reachable here. What it witnesses is *types and
//! declarations* — Stage 0 runs no lifecycle, so a pass here is not
//! behavioural conformance.

use sdax::host::{BoxFuture, Clock, CxInner, RawKey, Time};
use sdax::*;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::time::Duration;

// ---- a small vocabulary with one trait-object output

pub trait Endpoint: Send + Sync {
    fn addr(&self) -> String;
}
struct Quic;
impl Endpoint for Quic {
    fn addr(&self) -> String {
        "127.0.0.1:9000".into()
    }
}
struct PeerStore {
    via: String,
}
struct RoutingTable;
struct Receipt(#[allow(dead_code)] String);

fn assert_send_sync<T: Send + Sync>() {}
fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

struct TestClock(Arc<AtomicU64>);
impl Clock for TestClock {
    fn now(&self) -> Time {
        Time::from_nanos(self.0.load(Ordering::SeqCst))
    }
    fn sleep(&self, d: Duration) -> BoxFuture<'static, ()> {
        let nanos = self.0.clone();
        let deadline = nanos.load(Ordering::SeqCst) + d.as_nanos() as u64;
        Box::pin(std::future::poll_fn(move |_| {
            if nanos.load(Ordering::SeqCst) >= deadline {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        }))
    }
}

struct NoopWake;
impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

fn poll_once<F: Future + ?Sized>(f: Pin<&mut F>) -> Poll<F::Output> {
    let waker = Waker::from(Arc::new(NoopWake));
    f.poll(&mut Context::from_waker(&waker))
}

/// An effect that is `Pending` once, so registration is observed across a real
/// suspension.
async fn open_peers(t: Arc<dyn Endpoint>) -> Result<PeerStore, std::io::Error> {
    let mut once = false;
    std::future::poll_fn(move |_| {
        if once {
            Poll::Ready(())
        } else {
            once = true;
            Poll::Pending
        }
    })
    .await;
    Ok(PeerStore { via: t.addr() })
}

/// W-14 (A1, A3, A4, A5, A8) — the I-01 shape with a trait-object resource, a
/// tuple dependency, free attribute order and `?` inference in bodies.
#[test]
fn a_plan_records_typed_declarations_and_is_send_sync() {
    let mut p = Plan::builder("Startup");

    let transport: Key<dyn Endpoint> = p
        .resource("Transport")
        .acquire(|cx, ()| async move {
            let ep: Box<dyn Endpoint> = Box::new(Quic);
            Ok(cx.hold_value(ep))
        })
        .release(|_cx, _ep: Arc<dyn Endpoint>| async move { Ok(()) });

    // A5: `.within` before `.needs` is the same declaration as after it.
    let peers = p
        .resource("PeerStore")
        .within(secs(3))
        .needs(transport)
        .acquire(|cx, t: Arc<dyn Endpoint>| async move { cx.hold(open_peers(t)).await })
        .release(|_cx, _s| async move { Ok(()) });

    let routes = p
        .resource("RoutingTable")
        .needs(transport)
        .retry(Retry::attempts(3).backoff(Backoff::fixed(secs(1))))
        .idempotent()
        .acquire(|cx, _t: Arc<dyn Endpoint>| async move { Ok(cx.hold_value(RoutingTable)) })
        .release(release::by_drop());

    // A1: a two-key tuple dependency arrives as `(Arc<PeerStore>, Arc<RoutingTable>)`.
    let receipt = p
        .effect("Registration")
        .needs((peers, routes))
        .on_ambiguous(Ambiguity::Report)
        .perform(
            |cx, (s, _r): (Arc<PeerStore>, Arc<RoutingTable>)| async move {
                Ok(cx.hold_value(Receipt(s.via.clone())))
            },
        )
        .compensate(|_cx, _r| async move { Ok(()) });

    // A8: `?` infers the boxed error type from the builder's bound.
    p.service("Api")
        .needs((receipt, transport))
        .stop_within(secs(1))
        .start(
            |cx, (_r, t): (Arc<Receipt>, Arc<dyn Endpoint>)| async move {
                let _port: u16 = "9000".parse()?;
                Ok(Serving::new(t.addr(), async move {
                    cx.until_stop(std::future::pending::<()>()).await;
                    Ok(())
                }))
            },
        );

    let plan = p
        .build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Resident)
        .expect("valid");

    assert_send_sync::<Plan>();
    assert_send_sync::<Key<dyn Endpoint>>();
    fn is_copy<T: Copy>() {}
    is_copy::<Key<dyn Endpoint>>();

    let v = plan.inspect();
    assert_eq!(
        v.nodes
            .iter()
            .map(|n| n.path.to_string())
            .collect::<Vec<_>>(),
        [
            "Transport",
            "PeerStore",
            "RoutingTable",
            "Registration",
            "Api"
        ]
    );
    assert_eq!(v.node("PeerStore").unwrap().attr("within"), Some("3s"));
    assert_eq!(
        v.node("RoutingTable").unwrap().attr("release"),
        Some("drop")
    );
    assert_eq!(v.node("Registration").unwrap().kind, Kind::Effect);
    assert_eq!(plan.semantics(), "sdax/1");
}

/// A6 — the validator's findings are values that name the rule and the node.
#[test]
fn validation_findings_are_values() {
    let mut other = Plan::builder("Other");
    let stray = other.step("Stray").run(|_cx, ()| async { Ok(7u8) });

    let mut mine = Plan::builder("Mine");
    mine.step("UsesStray")
        .needs(stray)
        .run(|_cx, _n: Arc<u8>| async { Ok(()) });
    mine.step("UsesStray").run(|_cx, ()| async { Ok(()) });
    let inv = mine
        .build(Policy::Isolate, Shutdown::within(secs(1)), Mode::Finite)
        .expect_err("two rules are violated");
    let rules: Vec<&str> = inv.checks.iter().map(|f| f.rule.id()).collect();
    assert_eq!(rules, ["V-FOREIGN-KEY", "V-DUP-NAME"]);
    assert_eq!(inv.checks[0].nodes, ["UsesStray"]);
    assert!(inv.to_string().contains("V-DUP-NAME"));
}

/// A9 — `cx.hold(fut)` registers the value in the poll that observes the
/// effect completing: nothing while it is pending, nothing when it fails.
///
/// This is the assertion `sdax-v1/B/experiments/typed_keys` was written to
/// witness, re-run against this crate's own `Cx`.
#[test]
fn hold_registers_in_the_completing_poll() {
    let clock = Arc::new(TestClock(Arc::new(AtomicU64::new(0))));
    let inner = CxInner::new(RawKey { plan: 1, idx: 0 }, clock.clone());
    let cx: Cx<Acquire> = Cx::new(inner.clone());

    let mut held = Box::pin(cx.hold(async {
        let mut once = false;
        std::future::poll_fn(move |_| {
            if once {
                Poll::Ready(())
            } else {
                once = true;
                Poll::Pending
            }
        })
        .await;
        Ok::<u8, std::io::Error>(9)
    }));

    assert!(poll_once(held.as_mut()).is_pending());
    assert_eq!(
        inner.hold_count(),
        0,
        "nothing registered while the effect is in flight"
    );
    match poll_once(held.as_mut()) {
        Poll::Ready(Ok(v)) => assert_eq!(*v, 9),
        _ => panic!("expected the effect to complete"),
    }
    assert_eq!(inner.hold_count(), 1, "registered in the completing poll");

    let inner = CxInner::new(RawKey { plan: 1, idx: 0 }, clock);
    let cx: Cx<Acquire> = Cx::new(inner.clone());
    let mut failing = Box::pin(cx.hold(async { Err::<u8, _>(std::io::Error::other("no")) }));
    assert!(matches!(poll_once(failing.as_mut()), Poll::Ready(Err(_))));
    assert_eq!(inner.hold_count(), 0, "a failed effect registers nothing");
}

/// W-15 — a start body that moves its context into the serve future
/// type-checks against `start`'s bound, and the serve future observes `stop`.
#[test]
fn a_service_hands_over_a_serve_future_that_observes_stop() {
    let clock = Arc::new(TestClock(Arc::new(AtomicU64::new(0))));
    let inner = CxInner::new(RawKey { plan: 1, idx: 0 }, clock);
    let cx: Cx<Start> = Cx::new(inner.clone());

    let serving = Serving::new("127.0.0.1:9000".to_string(), async move {
        cx.until_stop(std::future::pending::<()>()).await;
        Ok(())
    });
    let (handle, mut serve) = serving.into_parts();
    assert_eq!(handle, "127.0.0.1:9000");
    assert!(poll_once(serve.as_mut()).is_pending());
    inner.stop_signal().request();
    assert!(matches!(poll_once(serve.as_mut()), Poll::Ready(Ok(()))));
}

/// The release context has a clock, a stop signal and a timeout, so bounded
/// cooperative teardown is writable (the I-42 shape).
#[test]
fn a_release_body_can_wait_then_kill_against_the_injected_clock() {
    let nanos = Arc::new(AtomicU64::new(0));
    let clock = Arc::new(TestClock(nanos.clone()));
    let inner = CxInner::new(RawKey { plan: 1, idx: 0 }, clock);
    let cx: Cx<Release> = Cx::new(inner);

    let killed = Arc::new(AtomicU64::new(0));
    let k = killed.clone();
    let mut body = Box::pin(async move {
        if cx
            .timeout(secs(5), std::future::pending::<()>())
            .await
            .is_err()
        {
            k.fetch_add(1, Ordering::SeqCst);
        }
        Ok::<(), Error>(())
    });
    assert!(poll_once(body.as_mut()).is_pending());
    assert_eq!(killed.load(Ordering::SeqCst), 0);
    nanos.store(6_000_000_000, Ordering::SeqCst);
    assert!(matches!(poll_once(body.as_mut()), Poll::Ready(Ok(()))));
    assert_eq!(
        killed.load(Ordering::SeqCst),
        1,
        "the grace expired, so the body killed"
    );
}

/// Stage 0 ships no execution: there is no `start`, and a `spawn` from a
/// context with no run attached says so rather than pretending.
#[test]
fn stage_0_has_no_execution_and_says_so() {
    let clock = Arc::new(TestClock(Arc::new(AtomicU64::new(0))));
    let inner = CxInner::new(RawKey { plan: 1, idx: 0 }, clock);
    let cx: Cx<Start> = Cx::new(inner);
    let mut t = Plan::template::<u8>("Link");
    t.step("Inner").run(|_cx, ()| async { Ok(()) });
    let child = t
        .build(Policy::Isolate, Shutdown::within(secs(1)), Mode::Finite)
        .expect("valid");
    let mut p = Plan::builder("Mesh");
    let links = p.template("Link", &child);
    assert_eq!(cx.spawn(&links, 1u8).unwrap_err(), SpawnError::NotRunning);
}
