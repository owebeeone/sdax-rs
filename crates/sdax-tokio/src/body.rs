//! What the driver spawns, and what comes back.
//!
//! Every message the run driver consumes is tagged with the *epoch* of the
//! attempt that produced it. The machine's events name a node, not an attempt
//! (`Event::NodeOk(RawKey)`), so a superseded attempt's late outcome would
//! otherwise be credited to the attempt that replaced it — a blocking body is
//! the case that forces this, because it cannot be aborted (T7) and its thread
//! finishes after the deadline already failed the attempt. The driver drops a
//! message whose epoch is not the node's current one (Stage 1 report § 10,
//! ordering rule 2).

use sdax::host::engine::{Event, TimerId};
use sdax::host::{BoxFuture, CxInner, Joined, RawKey, TaskHandle};
use sdax::{Error, FaultKind};
use std::any::Any;
use std::future::Future;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::sync::mpsc::UnboundedSender;

/// What reaches the driver's loop.
pub(crate) enum Msg {
    /// Something one attempt of a node's body observed.
    Body {
        /// Which node.
        node: RawKey,
        /// Which attempt produced it.
        epoch: u64,
        /// What happened.
        ev: Event,
    },
    /// A timer the machine armed has fired.
    Timer(TimerId),
    /// `shutdown()`, `cancel()`, or the drop of `Running`.
    Request(Event),
    /// `Running` was dropped while the run was live.
    Dropped,
}

/// The driver's inbox.
pub(crate) type Tx = UnboundedSender<Msg>;

/// The outcome of one guarded body: its return, or the panic payload.
type Outcome = Result<Result<(), Error>, Box<dyn Any + Send>>;

/// A body under the engine's wrapper: panics are caught rather than unwound
/// into the runtime, and a registration is reported in the poll that observed
/// it (T2) — before the body's continuation can be polled again.
struct Guarded {
    fut: BoxFuture<'static, Result<(), Error>>,
    cx: Option<Arc<CxInner>>,
    node: RawKey,
    epoch: u64,
    tx: Tx,
    held: bool,
}

impl Future for Guarded {
    type Output = Outcome;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Outcome> {
        let me = self.get_mut();
        let polled = catch_unwind(AssertUnwindSafe(|| me.fut.as_mut().poll(cx)));
        if !me.held {
            if let Some(inner) = &me.cx {
                if inner.hold_count() > 0 {
                    me.held = true;
                    let _ = me.tx.send(Msg::Body {
                        node: me.node,
                        epoch: me.epoch,
                        ev: Event::Held(me.node),
                    });
                }
            }
        }
        match polled {
            Ok(Poll::Pending) => Poll::Pending,
            Ok(Poll::Ready(r)) => Poll::Ready(Ok(r)),
            Err(payload) => Poll::Ready(Err(payload)),
        }
    }
}

/// What the machine is told a finished body did.
///
/// A body that registered twice in one attempt broke the seam's one-value
/// rule, and it broke it whatever it then returned: only the last value can be
/// discharged and the first is dropped with the body's locals, unreleased and
/// unrecorded. So `DoubleHold` outranks the body's own `Err` — the error is
/// the body's opinion of its work, the double hold is the engine's observation
/// that the seam was broken, and only one of them can be the attempt's fault.
/// A panic still outranks both: its payload is carried nowhere else.
fn outcome_event(node: RawKey, cx: &Arc<CxInner>, out: Outcome) -> Event {
    match out {
        Err(payload) => Event::NodeErr(node, FaultKind::Panic(payload)),
        Ok(_) if cx.hold_count() > 1 => Event::NodeErr(node, FaultKind::DoubleHold),
        Ok(Err(e)) => Event::NodeErr(node, FaultKind::Error(e)),
        Ok(Ok(())) => Event::NodeOk(node),
    }
}

/// One attempt of an async prepare, run, start, release or compensate body.
///
/// `announce` is `Event::Started`, which belongs to a *prepare* attempt only:
/// the machine has no body in flight for a node whose release it just opened,
/// and would refuse one (D1).
pub(crate) async fn run_body(
    tx: Tx,
    node: RawKey,
    epoch: u64,
    cx: Arc<CxInner>,
    fut: BoxFuture<'static, Result<(), Error>>,
    watch_holds: bool,
    announce: bool,
) {
    if announce {
        let _ = tx.send(Msg::Body {
            node,
            epoch,
            ev: Event::Started(node),
        });
    }
    let out = Guarded {
        fut,
        cx: if watch_holds { Some(cx.clone()) } else { None },
        node,
        epoch,
        tx: tx.clone(),
        held: false,
    }
    .await;
    let ev = outcome_event(node, &cx, out);
    let _ = tx.send(Msg::Body { node, epoch, ev });
}

/// A service's serve future, once its start body handed it over.
pub(crate) async fn run_serve(
    tx: Tx,
    node: RawKey,
    epoch: u64,
    serve: BoxFuture<'static, Result<(), Error>>,
) {
    let out = Guarded {
        fut: serve,
        cx: None,
        node,
        epoch,
        tx: tx.clone(),
        held: true,
    }
    .await;
    let fault = match out {
        Ok(Ok(())) => None,
        Ok(Err(e)) => Some(FaultKind::Error(e)),
        Err(payload) => Some(FaultKind::Panic(payload)),
    };
    let _ = tx.send(Msg::Body {
        node,
        epoch,
        ev: Event::ServeEnded { node, fault },
    });
}

/// One attempt of a blocking body, as the pool wants it: a plain closure.
///
/// It cannot be aborted (T7), so nothing here listens for a cancel; the
/// machine fails the attempt at its deadline and the driver drops whatever
/// this sends afterwards, by epoch.
///
/// `announce` carries the same rule as [`run_body`]'s: `Event::Started`
/// belongs to a prepare attempt, and a host `BodySource` may hand back a
/// blocking *cleanup*, for which the machine has no body in flight and would
/// rightly refuse one (D1).
pub(crate) fn blocking_job(
    tx: Tx,
    node: RawKey,
    epoch: u64,
    f: Box<dyn FnOnce() -> Result<(), Error> + Send>,
    announce: bool,
) -> Box<dyn FnOnce() + Send> {
    Box::new(move || {
        if announce {
            let _ = tx.send(Msg::Body {
                node,
                epoch,
                ev: Event::Started(node),
            });
        }
        let out = catch_unwind(AssertUnwindSafe(f));
        let ev = match out {
            Err(payload) => Event::NodeErr(node, FaultKind::Panic(payload)),
            Ok(Err(e)) => Event::NodeErr(node, FaultKind::Error(e)),
            Ok(Ok(())) => Event::NodeOk(node),
        };
        let _ = tx.send(Msg::Body { node, epoch, ev });
    })
}

/// Join an aborted task and tell the machine how it ended (T5: the join comes
/// before the node counts as settled).
///
/// `Joined::Done` means the task had already sent its own outcome — the body
/// finished before the abort could land, which a real runtime allows and which
/// the machine then treats as that outcome (ordering rule 1). Nothing is sent
/// for it. A `Panicked` join is the engine's cancel taking effect through a
/// panicking drop of the body's locals; the panic is not the work's failure
/// (OD-PANIC-CANCELLED), so it is reported as the cancellation it is.
pub(crate) async fn join_aborted<H: TaskHandle>(
    tx: Tx,
    node: RawKey,
    epoch: u64,
    cx: Arc<CxInner>,
    handle: H,
) {
    let joined = handle.join().await;
    let ev = match joined {
        Joined::Done => return,
        _ => Event::NodeCancelled {
            node,
            held: cx.hold_count() > 0,
        },
    };
    let _ = tx.send(Msg::Body { node, epoch, ev });
}
