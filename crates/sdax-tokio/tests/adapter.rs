//! Suite (d), the part Stage 0 can carry: the `Runtime` contract against the
//! real substrate. The run driver, the `Running` drop guard and the drainer
//! are Stage 2.

use sdax::{
    Joined, NoObserver, Observer, Outcome, Runtime, TaskHandle, Time, TraceEvent, TraceKind,
};
use sdax_tokio::TokioRuntime;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Build the runtime by hand: the adapter takes no `macros` feature, so there
/// is no `#[tokio::main]` and no `#[tokio::test]` anywhere in this workspace.
fn paused() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .expect("runtime")
}

/// A compile witness that the adapter really implements the contract
/// (LBT-002/LBT-009: the compiler resolves this, not a source scan).
fn takes_a_runtime<R: Runtime>(rt: &R) -> Time {
    rt.clock().now()
}

#[test]
fn tokio_runtime_implements_the_runtime_contract() {
    let tokio_rt = paused();
    let rt = TokioRuntime::new(tokio_rt.handle().clone());
    let _guard = tokio_rt.enter();
    assert_eq!(takes_a_runtime(&rt), rt.clock().now());
}

#[test]
fn a_spawned_task_runs_and_joins_as_done() {
    let tokio_rt = paused();
    let rt = TokioRuntime::new(tokio_rt.handle().clone());
    let ran = Arc::new(AtomicU64::new(0));
    let r = ran.clone();
    let joined = tokio_rt.block_on(async move {
        let task = rt.spawn(Box::pin(async move {
            r.fetch_add(1, Ordering::SeqCst);
        }));
        task.join().await
    });
    assert_eq!(ran.load(Ordering::SeqCst), 1);
    assert!(matches!(joined, Joined::Done));
}

#[test]
fn an_aborted_task_joins_as_cancelled_and_the_abort_is_deferred() {
    let tokio_rt = paused();
    let rt = TokioRuntime::new(tokio_rt.handle().clone());
    let reached_second_half = Arc::new(AtomicU64::new(0));
    let flag = reached_second_half.clone();
    let joined = tokio_rt.block_on(async move {
        let task = rt.spawn(Box::pin(async move {
            tokio::time::sleep(Duration::from_secs(10)).await;
            flag.fetch_add(1, Ordering::SeqCst);
        }));
        tokio::task::yield_now().await;
        task.abort();
        task.join().await
    });
    assert!(matches!(joined, Joined::Cancelled));
    assert_eq!(
        reached_second_half.load(Ordering::SeqCst),
        0,
        "an abort lands between polls; the body never resumed"
    );
}

#[test]
fn a_panicking_task_joins_as_panicked_and_the_panic_does_not_escape() {
    let tokio_rt = paused();
    let rt = TokioRuntime::new(tokio_rt.handle().clone());
    let joined = tokio_rt.block_on(async move {
        let task = rt.spawn(Box::pin(async {
            panic!("boom");
        }));
        task.join().await
    });
    match joined {
        Joined::Panicked(payload) => {
            assert_eq!(payload.downcast_ref::<&str>(), Some(&"boom"));
        }
        other => panic!("expected a panic, got {other:?}"),
    }
}

#[test]
fn a_blocking_body_runs_off_the_async_workers() {
    let tokio_rt = paused();
    let rt = TokioRuntime::new(tokio_rt.handle().clone());
    let here = std::thread::current().id();
    let seen = Arc::new(std::sync::Mutex::new(None));
    let s = seen.clone();
    let joined = tokio_rt.block_on(async move {
        let task = rt.spawn_blocking(Box::new(move || {
            *s.lock().unwrap() = Some(std::thread::current().id());
        }));
        task.join().await
    });
    assert!(matches!(joined, Joined::Done));
    assert_ne!(
        seen.lock().unwrap().expect("ran"),
        here,
        "not on the caller's thread"
    );
}

/// The same three properties `sdax_testkit::invariants::check_clock` asserts
/// for a fake clock. They are checked here in the adapter's own idiom because
/// advancing paused tokio time is asynchronous, and the shared suite is a
/// synchronous function.
#[test]
fn the_tokio_clock_only_moves_when_tokio_time_moves() {
    let tokio_rt = paused();
    let rt = TokioRuntime::new(tokio_rt.handle().clone());
    tokio_rt.block_on(async move {
        let start = rt.clock().now();
        tokio::task::yield_now().await;
        assert_eq!(rt.clock().now(), start, "now() does not move by itself");

        let sleeping = rt.clock().sleep(Duration::from_secs(5));
        tokio::time::advance(Duration::from_secs(4)).await;
        assert!(
            tokio::time::timeout(Duration::from_millis(0), sleeping)
                .await
                .is_err(),
            "a 5s sleep is not ready after 4s"
        );
        let sleeping = rt.clock().sleep(Duration::from_secs(1));
        tokio::time::advance(Duration::from_secs(1)).await;
        sleeping.await;
        assert_eq!(
            rt.clock().now().as_nanos() - start.as_nanos(),
            5_000_000_000,
            "the clock advanced by exactly what tokio advanced"
        );
    });
}

#[test]
fn an_observer_can_be_installed_and_defaults_to_recording_nothing() {
    let tokio_rt = paused();
    let rt = TokioRuntime::new(tokio_rt.handle().clone());
    rt.observer()
        .event(&TraceEvent::at(Time::ZERO, TraceKind::End(Outcome::Ok)));

    let recorder = Arc::new(sdax_testkit::TraceRecorder::new());
    let rt = TokioRuntime::new(tokio_rt.handle().clone()).with_observer(recorder.clone());
    rt.observer()
        .event(&TraceEvent::at(Time::ZERO, TraceKind::Ready));
    assert_eq!(recorder.len(), 1);

    let _: &dyn Observer = &NoObserver;
}

#[test]
fn every_spawned_task_is_tracked_so_a_shutdown_can_wait_for_it() {
    let tokio_rt = paused();
    let rt = TokioRuntime::new(tokio_rt.handle().clone());
    let done = Arc::new(AtomicU64::new(0));
    let d = done.clone();
    tokio_rt.block_on(async move {
        let _task = rt.spawn(Box::pin(async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            d.fetch_add(1, Ordering::SeqCst);
        }));
        assert_eq!(rt.tracked(), 1);
        tokio::time::advance(Duration::from_secs(2)).await;
        rt.close_and_wait(Duration::from_secs(5))
            .await
            .expect("drained");
        assert_eq!(rt.tracked(), 0);
    });
    assert_eq!(done.load(Ordering::SeqCst), 1);
}

#[test]
fn a_shutdown_that_runs_out_of_budget_says_what_is_still_running() {
    let tokio_rt = paused();
    let rt = TokioRuntime::new(tokio_rt.handle().clone());
    tokio_rt.block_on(async move {
        let _task = rt.spawn(Box::pin(async {
            std::future::pending::<()>().await;
        }));
        let left = rt
            .close_and_wait(Duration::from_secs(1))
            .await
            .expect_err("cannot drain");
        assert_eq!(
            left, 1,
            "one task was still running when the budget expired"
        );
    });
}
