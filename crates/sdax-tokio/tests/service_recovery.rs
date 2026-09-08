use sdax::prelude::*;
use sdax_tokio::{PlanStart, TokioRuntime};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn paused() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .expect("runtime")
}

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn adapter(rt: &tokio::runtime::Runtime) -> Arc<TokioRuntime> {
    Arc::new(TokioRuntime::current_thread_no_background_drain(
        rt.handle().clone(),
    ))
}

#[derive(Debug)]
struct EpisodeFailed;

impl std::fmt::Display for EpisodeFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("episode failed")
    }
}

impl std::error::Error for EpisodeFailed {}

struct Handle {
    value: AtomicUsize,
}

#[test]
fn initialization_publishes_one_stable_handle_across_serving_episodes() {
    let tokio_rt = paused();
    let rt = adapter(&tokio_rt);
    let initializes = Arc::new(AtomicUsize::new(0));
    let dependent_initializes = Arc::new(AtomicUsize::new(0));
    let seen_ptrs = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::new(AtomicUsize::new(0));

    let mut p = Plan::builder("stable service handle");
    let handle = p
        .service("upstream")
        .idempotent()
        .restart(Restart::on_error(Backoff::fixed(secs(1))).max(2))
        .stop_within(secs(1))
        .initialize({
            let initializes = initializes.clone();
            move |_cx, ()| {
                initializes.fetch_add(1, Ordering::SeqCst);
                async {
                    Ok(Handle {
                        value: AtomicUsize::new(0),
                    })
                }
            }
        })
        .serve({
            let seen_ptrs = seen_ptrs.clone();
            move |cx, handle: Arc<Handle>| {
                seen_ptrs
                    .lock()
                    .expect("seen pointers")
                    .push(Arc::as_ptr(&handle) as usize);
                async move {
                    handle.value.store(cx.episode() as usize, Ordering::SeqCst);
                    if cx.episode() < 3 {
                        Err(Box::new(EpisodeFailed) as Error)
                    } else {
                        cx.stop().await;
                        Ok(())
                    }
                }
            }
        });

    p.service("dependent")
        .needs(handle)
        .stop_within(secs(1))
        .initialize({
            let dependent_initializes = dependent_initializes.clone();
            move |_cx, handle: Arc<Handle>| {
                dependent_initializes.fetch_add(1, Ordering::SeqCst);
                async move { Ok(handle) }
            }
        })
        .serve({
            let observed = observed.clone();
            move |cx, handle: Arc<Arc<Handle>>| {
                let observed = observed.clone();
                async move {
                    while !cx.is_stopping() {
                        observed.store(handle.value.load(Ordering::SeqCst), Ordering::SeqCst);
                        cx.sleep(secs(1)).await;
                    }
                    Ok(())
                }
            }
        });

    let plan = p
        .build(Policy::FailFast, Shutdown::within(secs(5)), Mode::Resident)
        .expect("valid");

    let report = tokio_rt.block_on(async {
        let mut running = plan.start(rt.clone(), ());
        running.ready().await.expect("initial readiness");
        tokio::time::sleep(secs(4)).await;
        assert_eq!(initializes.load(Ordering::SeqCst), 1);
        assert_eq!(dependent_initializes.load(Ordering::SeqCst), 1);
        assert_eq!(observed.load(Ordering::SeqCst), 3);
        {
            let ptrs = seen_ptrs.lock().expect("seen pointers");
            assert_eq!(ptrs.len(), 3);
            assert!(ptrs.iter().all(|ptr| *ptr == ptrs[0]));
        }
        running.shutdown();
        running.await
    });

    assert_eq!(report.outcome, Outcome::Ok);
    assert!(report.is_clean(), "{report}");
    assert_eq!(rt.tracked(), 0);
}

#[test]
fn startup_retry_counts_initialization_attempts_not_serving_episodes() {
    let tokio_rt = paused();
    let rt = adapter(&tokio_rt);
    let attempts = Arc::new(AtomicUsize::new(0));
    let episodes = Arc::new(Mutex::new(Vec::new()));
    let mut p = Plan::builder("startup retry");
    p.service("worker")
        .retry(Retry::attempts(2))
        .stop_within(secs(1))
        .initialize({
            let attempts = attempts.clone();
            move |cx, ()| {
                let attempts = attempts.clone();
                async move {
                    attempts.fetch_add(1, Ordering::SeqCst);
                    if cx.attempt() == 1 {
                        Err(Box::new(EpisodeFailed) as Error)
                    } else {
                        Ok(Handle {
                            value: AtomicUsize::new(7),
                        })
                    }
                }
            }
        })
        .serve({
            let episodes = episodes.clone();
            move |cx, _handle| {
                episodes.lock().expect("episodes").push(cx.episode());
                async move {
                    cx.stop().await;
                    Ok(())
                }
            }
        });
    let plan = p
        .build(Policy::FailFast, Shutdown::within(secs(5)), Mode::Resident)
        .expect("valid");

    let report = tokio_rt.block_on(async {
        let mut running = plan.start(rt.clone(), ());
        running.ready().await.expect("ready after retry");
        running.shutdown();
        running.await
    });
    assert_eq!(report.outcome, Outcome::Ok);
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    assert_eq!(*episodes.lock().expect("episodes"), [1]);
    assert_eq!(rt.tracked(), 0);
}

#[test]
fn readiness_stays_latched_while_serving_recovers() {
    let tokio_rt = paused();
    let rt = adapter(&tokio_rt);
    let episodes = Arc::new(AtomicUsize::new(0));
    let dependent_initializes = Arc::new(AtomicUsize::new(0));
    let mut p = Plan::builder("latched service readiness");
    let handle = p
        .service("worker")
        .idempotent()
        .restart(Restart::on_error(Backoff::fixed(secs(60))).max(1))
        .stop_within(secs(1))
        .initialize(|_cx, ()| async {
            Ok(Handle {
                value: AtomicUsize::new(9),
            })
        })
        .serve({
            let episodes = episodes.clone();
            move |cx, _handle| {
                episodes.fetch_add(1, Ordering::SeqCst);
                async move {
                    if cx.episode() == 1 {
                        Err(Box::new(EpisodeFailed) as Error)
                    } else {
                        cx.stop().await;
                        Ok(())
                    }
                }
            }
        });
    let gate = p.step("late gate").run(|cx, ()| async move {
        cx.sleep(secs(1)).await;
        Ok(())
    });
    p.step("late dependent").needs((handle, gate)).run({
        let dependent_initializes = dependent_initializes.clone();
        move |_cx, (handle, _gate)| {
            dependent_initializes.fetch_add(1, Ordering::SeqCst);
            async move {
                assert_eq!(handle.value.load(Ordering::SeqCst), 9);
                Ok(())
            }
        }
    });
    let plan = p
        .build(Policy::FailFast, Shutdown::within(secs(5)), Mode::Resident)
        .expect("valid");

    let report = tokio_rt.block_on(async {
        let mut running = plan.start(rt.clone(), ());
        running.ready().await.expect("latched readiness");
        assert_eq!(episodes.load(Ordering::SeqCst), 1);
        assert_eq!(dependent_initializes.load(Ordering::SeqCst), 1);
        running.shutdown();
        running.await
    });
    assert_eq!(report.outcome, Outcome::Failed);
    assert_eq!(report.faults[0].phase, Phase::Serve);
    assert_eq!(report.faults[0].order.attempt, 1);
    assert_eq!(rt.tracked(), 0);
}

#[test]
fn restart_exhaustion_reports_the_last_episode_and_does_not_reinitialize() {
    let tokio_rt = paused();
    let rt = adapter(&tokio_rt);
    let initializes = Arc::new(AtomicUsize::new(0));
    let episodes = Arc::new(Mutex::new(Vec::new()));
    let mut p = Plan::builder("restart exhaustion");
    p.service("worker")
        .idempotent()
        .restart(Restart::on_error(Backoff::fixed(secs(1))).max(1))
        .stop_within(secs(1))
        .initialize({
            let initializes = initializes.clone();
            move |_cx, ()| {
                initializes.fetch_add(1, Ordering::SeqCst);
                async { Ok(()) }
            }
        })
        .serve({
            let episodes = episodes.clone();
            move |cx, _handle: Arc<()>| {
                episodes.lock().expect("episodes").push(cx.episode());
                async { Err(Box::new(EpisodeFailed) as Error) }
            }
        });
    let plan = p
        .build(Policy::FailFast, Shutdown::within(secs(5)), Mode::Resident)
        .expect("valid");

    let report = tokio_rt.block_on(plan.start(rt.clone(), ()));
    assert_eq!(report.outcome, Outcome::Failed);
    assert_eq!(initializes.load(Ordering::SeqCst), 1);
    assert_eq!(*episodes.lock().expect("episodes"), [1, 2]);
    assert_eq!(
        report.faults.len(),
        1,
        "the recovered episode is historical"
    );
    assert_eq!(report.faults[0].phase, Phase::Serve);
    assert_eq!(report.faults[0].order.attempt, 2, "episode number");
    assert_eq!(rt.tracked(), 0);
}

#[test]
fn shutdown_interrupts_restart_backoff_without_starting_another_episode() {
    let tokio_rt = paused();
    let rt = adapter(&tokio_rt);
    let episodes = Arc::new(AtomicUsize::new(0));
    let (failed_tx, failed_rx) = tokio::sync::oneshot::channel();
    let failed_tx = Arc::new(Mutex::new(Some(failed_tx)));
    let mut p = Plan::builder("interrupt restart backoff");
    p.service("worker")
        .idempotent()
        .restart(Restart::on_error(Backoff::fixed(secs(60))))
        .stop_within(secs(1))
        .initialize(|_cx, ()| async { Ok(()) })
        .serve({
            let episodes = episodes.clone();
            move |_cx, _handle: Arc<()>| {
                episodes.fetch_add(1, Ordering::SeqCst);
                if let Some(tx) = failed_tx.lock().expect("failure signal").take() {
                    let _ = tx.send(());
                }
                async { Err(Box::new(EpisodeFailed) as Error) }
            }
        });
    let plan = p
        .build(Policy::FailFast, Shutdown::within(secs(5)), Mode::Resident)
        .expect("valid");

    let report = tokio_rt.block_on(async {
        let mut running = plan.start(rt.clone(), ());
        running.ready().await.expect("initial readiness");
        failed_rx.await.expect("first episode invoked");
        running.shutdown();
        running.await
    });
    assert_eq!(episodes.load(Ordering::SeqCst), 1);
    assert_eq!(report.outcome, Outcome::Failed);
    assert_eq!(rt.tracked(), 0);
}

#[test]
fn terminal_service_ends_after_its_serving_episode_completes() {
    let tokio_rt = paused();
    let rt = adapter(&tokio_rt);
    let mut p = Plan::builder("terminal service");
    p.service("once")
        .terminal()
        .stop_within(secs(1))
        .initialize(|_cx, ()| async { Ok(()) })
        .serve(|cx, _handle: Arc<()>| async move {
            assert_eq!(cx.episode(), 1);
            Ok(())
        });
    let plan = p
        .build(Policy::FailFast, Shutdown::within(secs(5)), Mode::Resident)
        .expect("valid");

    let report = tokio_rt.block_on(plan.start(rt.clone(), ()));
    assert_eq!(report.outcome, Outcome::Ok);
    assert!(report.is_clean());
    assert_eq!(rt.tracked(), 0);
}

#[test]
fn synchronous_initialize_and_serve_factory_panics_are_reported() {
    let tokio_rt = paused();
    let rt = adapter(&tokio_rt);

    let mut init = Plan::builder("initialize factory panic");
    init.service("worker")
        .stop_within(secs(1))
        .initialize(|_cx, ()| {
            panic!("initialize factory panic");
            #[allow(unreachable_code)]
            std::future::ready(Ok(()))
        })
        .serve(|cx, _handle: Arc<()>| async move {
            cx.stop().await;
            Ok(())
        });
    let init = init
        .build(Policy::FailFast, Shutdown::within(secs(5)), Mode::Resident)
        .expect("valid");
    let init_report = tokio_rt.block_on(init.start(rt.clone(), ()));
    assert_eq!(init_report.outcome, Outcome::Failed);
    assert!(matches!(init_report.faults[0].kind, FaultKind::Panic(_)));

    let mut serve = Plan::builder("serve factory panic");
    serve
        .service("worker")
        .stop_within(secs(1))
        .initialize(|_cx, ()| async { Ok(()) })
        .serve(|_cx, _handle: Arc<()>| {
            panic!("serve factory panic");
            #[allow(unreachable_code)]
            std::future::ready(Ok(()))
        });
    let serve = serve
        .build(Policy::FailFast, Shutdown::within(secs(5)), Mode::Resident)
        .expect("valid");
    let serve_report = tokio_rt.block_on(serve.start(rt.clone(), ()));
    assert_eq!(serve_report.outcome, Outcome::Failed);
    assert_eq!(serve_report.faults[0].phase, Phase::Serve);
    assert!(matches!(serve_report.faults[0].kind, FaultKind::Panic(_)));
    assert_eq!(rt.tracked(), 0);
}
