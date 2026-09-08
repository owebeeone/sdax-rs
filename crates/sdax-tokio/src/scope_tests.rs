use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll, Wake, Waker};

struct ReentrantWake {
    children: Arc<Children>,
    wakes: AtomicUsize,
}

impl Wake for ReentrantWake {
    fn wake(self: Arc<Self>) {
        // Readiness notifications must not hold the child registry lock.
        assert!(self.children.latches.try_lock().is_ok());
        self.wakes.fetch_add(1, Ordering::Relaxed);
    }
}

fn register(scope: &RunScope, id: u64) {
    scope.children.latches.lock().unwrap().insert(
        InstanceId(id),
        ChildLatch {
            latch: Latch::new(),
            answered: false,
        },
    );
}

#[test]
fn readiness_handles_out_of_order_ids_and_wakes_once_outside_registry_lock() {
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let scope = RunScope::new(tx);
    register(&scope, 90);
    register(&scope, 2);
    let wake = Arc::new(ReentrantWake {
        children: scope.children.clone(),
        wakes: AtomicUsize::new(0),
    });
    let waker = Waker::from(wake.clone());
    let mut cx = Context::from_waker(&waker);
    let mut first = scope.children.ready(InstanceId(90));
    let mut second = scope.children.ready(InstanceId(2));
    assert!(first.as_mut().poll(&mut cx).is_pending());
    assert!(second.as_mut().poll(&mut cx).is_pending());
    scope.resolve(&[
        (InstanceId(2), RunState::Steady),
        (InstanceId(90), RunState::Admitting),
    ]);
    assert!(matches!(second.as_mut().poll(&mut cx), Poll::Ready(Ok(()))));
    assert!(first.as_mut().poll(&mut cx).is_pending());
    scope.resolve(&[
        (InstanceId(2), RunState::Steady),
        (InstanceId(999), RunState::Steady),
    ]);
    assert_eq!(wake.wakes.load(Ordering::Relaxed), 1);
    scope.resolve(&[(InstanceId(90), RunState::Steady)]);
    scope.ended(InstanceId(90), Outcome::Failed);
    assert!(matches!(first.as_mut().poll(&mut cx), Poll::Ready(Ok(()))));
    assert_eq!(wake.wakes.load(Ordering::Relaxed), 2);
}

#[test]
fn ending_before_ready_preserves_failure_for_existing_waiter() {
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let scope = RunScope::new(tx);
    register(&scope, 7);
    let wake = Arc::new(ReentrantWake {
        children: scope.children.clone(),
        wakes: AtomicUsize::new(0),
    });
    let waker = Waker::from(wake.clone());
    let mut cx = Context::from_waker(&waker);
    let mut ready = scope.children.ready(InstanceId(7));
    assert!(ready.as_mut().poll(&mut cx).is_pending());
    scope.ended(InstanceId(7), Outcome::Cancelled);
    let Poll::Ready(Err(error)) = ready.as_mut().poll(&mut cx) else {
        panic!("ended child must answer its waiter");
    };
    assert_eq!(
        error.downcast_ref::<InstanceEnded>().unwrap().0,
        Outcome::Cancelled
    );
    assert_eq!(wake.wakes.load(Ordering::Relaxed), 1);
}
