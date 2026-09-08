use sdax::*;
use sdax_tokio::{PlanStart, TokioRuntime};
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::Poll;
use std::time::Duration;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn paused() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .expect("runtime")
}

fn host(rt: &tokio::runtime::Runtime) -> Arc<TokioRuntime> {
    Arc::new(TokioRuntime::current_thread_no_background_drain(
        rt.handle().clone(),
    ))
}

fn remaining<P>(cx: &Cx<P>) -> Option<Duration> {
    cx.deadline()
        .and_then(|deadline| deadline.checked_duration_since(cx.now()))
}

#[test]
fn release_uses_shutdown_budget_instead_of_prepare_within() {
    let observed = Arc::new(Mutex::new(None));
    let mut p = Plan::builder("cleanup deadline");
    p.resource("lease")
        .within(ms(1))
        .acquire(|cx, ()| async move { Ok(cx.hold_value(1_u64)) })
        .release({
            let observed = observed.clone();
            move |cx, _| {
                *observed.lock().expect("observed") = remaining(&cx);
                async { Ok(()) }
            }
        });
    let plan = p
        .build(Policy::FailFast, Shutdown::within(ms(100)), Mode::Finite)
        .expect("valid");
    let tokio_rt = paused();
    let adapter = host(&tokio_rt);
    let report = tokio_rt.block_on(plan.start(adapter.clone(), ()));

    assert!(report.is_clean(), "{report}");
    assert_eq!(*observed.lock().expect("observed"), Some(ms(100)));
    assert_eq!(adapter.tracked(), 0);
}

#[test]
fn serving_context_is_refreshed_with_its_stop_deadline() {
    let observed = Arc::new(Mutex::new(None));
    let mut p = Plan::builder("serving deadline");
    p.service("worker")
        .stop_within(ms(5))
        .initialize(|_, ()| async { Ok(()) })
        .serve({
            let observed = observed.clone();
            move |cx, _| {
                let observed = observed.clone();
                async move {
                    cx.stop().await;
                    *observed.lock().expect("observed") = remaining(&cx);
                    Ok(())
                }
            }
        });
    let plan = p
        .build(Policy::FailFast, Shutdown::within(ms(100)), Mode::Resident)
        .expect("valid");
    let tokio_rt = paused();
    let adapter = host(&tokio_rt);
    let report = tokio_rt.block_on(async {
        let mut running = plan.start(adapter.clone(), ());
        running.ready().await.expect("ready");
        running.shutdown();
        running.await
    });

    assert!(report.is_clean(), "{report}");
    assert_eq!(*observed.lock().expect("observed"), Some(ms(5)));
    assert_eq!(adapter.tracked(), 0);
}

#[test]
fn recovered_serving_episode_gets_the_same_stop_deadline() {
    let observed = Arc::new(Mutex::new(None));
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let started_tx = Arc::new(Mutex::new(Some(started_tx)));
    let mut p = Plan::builder("recovered serving deadline");
    p.service("worker")
        .idempotent()
        .restart(Restart::on_error(Backoff::fixed(ms(1))).max(1))
        .stop_within(ms(5))
        .initialize(|_, ()| async { Ok(()) })
        .serve({
            let observed = observed.clone();
            move |cx, _| {
                let observed = observed.clone();
                let started_tx = started_tx.clone();
                async move {
                    if cx.episode() == 1 {
                        return Err("restart".into());
                    }
                    if let Some(tx) = started_tx.lock().expect("started").take() {
                        let _ = tx.send(());
                    }
                    cx.stop().await;
                    *observed.lock().expect("observed") = remaining(&cx);
                    Ok(())
                }
            }
        });
    let plan = p
        .build(Policy::FailFast, Shutdown::within(ms(100)), Mode::Resident)
        .expect("valid");
    let tokio_rt = paused();
    let adapter = host(&tokio_rt);
    let report = tokio_rt.block_on(async {
        let mut running = std::pin::pin!(plan.start(adapter.clone(), ()));
        let control = running.handle();
        let mut started_rx = std::pin::pin!(started_rx);
        std::future::poll_fn(|cx| {
            assert!(running.as_mut().poll(cx).is_pending());
            match started_rx.as_mut().poll(cx) {
                Poll::Ready(Ok(())) => Poll::Ready(()),
                Poll::Ready(Err(_)) => panic!("episode 2 sender dropped"),
                Poll::Pending => Poll::Pending,
            }
        })
        .await;
        control.shutdown();
        running.await
    });

    assert!(report.is_clean(), "{report}");
    assert_eq!(*observed.lock().expect("observed"), Some(ms(5)));
    assert_eq!(adapter.tracked(), 0);
}

#[test]
fn retry_release_has_no_prepare_deadline_and_final_release_has_the_budget() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let observed = Arc::new(Mutex::new(Vec::new()));
    let mut p = Plan::builder("retry cleanup deadline");
    p.resource("lease")
        .within(ms(1))
        .retry(Retry::attempts(2))
        .acquire({
            let attempts = attempts.clone();
            move |cx, ()| {
                let attempt = attempts.fetch_add(1, Ordering::SeqCst) + 1;
                async move {
                    let held = cx.hold_value(attempt);
                    if attempt == 1 {
                        Err("retry".into())
                    } else {
                        Ok(held)
                    }
                }
            }
        })
        .release({
            let observed = observed.clone();
            move |cx, _| {
                observed.lock().expect("observed").push(remaining(&cx));
                async { Ok(()) }
            }
        });
    let plan = p
        .build(Policy::FailFast, Shutdown::within(ms(100)), Mode::Finite)
        .expect("valid");
    let tokio_rt = paused();
    let adapter = host(&tokio_rt);
    let report = tokio_rt.block_on(plan.start(adapter.clone(), ()));

    assert!(report.is_clean(), "{report}");
    assert_eq!(*observed.lock().expect("observed"), [None, Some(ms(100))]);
    assert_eq!(adapter.tracked(), 0);
}

#[test]
fn retry_release_observes_a_shutdown_budget_activated_after_it_started() {
    let observed = Arc::new(Mutex::new(None));
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let started_tx = Arc::new(Mutex::new(Some(started_tx)));
    let mut p = Plan::builder("active retry cleanup deadline");
    p.resource("lease")
        .retry(Retry::attempts(2))
        .acquire(|cx, ()| async move {
            let _held = cx.hold_value(());
            Err::<Held<()>, Error>("retry".into())
        })
        .release({
            let observed = observed.clone();
            move |cx, _| {
                let observed = observed.clone();
                let started_tx = started_tx.clone();
                async move {
                    assert_eq!(remaining(&cx), None, "budget is not active at entry");
                    if let Some(tx) = started_tx.lock().expect("started").take() {
                        let _ = tx.send(());
                    }
                    cx.sleep(ms(5)).await;
                    *observed.lock().expect("observed") = remaining(&cx);
                    Ok(())
                }
            }
        });
    let plan = p
        .build(Policy::FailFast, Shutdown::within(ms(10)), Mode::Resident)
        .expect("valid");
    let tokio_rt = paused();
    let adapter = host(&tokio_rt);
    let report = tokio_rt.block_on(async {
        let mut running = std::pin::pin!(plan.start(adapter.clone(), ()));
        let control = running.handle();
        let mut started_rx = std::pin::pin!(started_rx);
        std::future::poll_fn(|cx| {
            assert!(running.as_mut().poll(cx).is_pending());
            match started_rx.as_mut().poll(cx) {
                Poll::Ready(Ok(())) => Poll::Ready(()),
                Poll::Ready(Err(_)) => panic!("retry release sender dropped"),
                Poll::Pending => Poll::Pending,
            }
        })
        .await;
        control.shutdown();
        running.await
    });

    assert_eq!(report.outcome, Outcome::Failed);
    assert_eq!(*observed.lock().expect("observed"), Some(ms(5)));
    assert_eq!(adapter.tracked(), 0);
}

#[test]
fn nested_release_deadline_is_capped_by_the_parent_budget() {
    let observed = Arc::new(Mutex::new(None));
    let mut child = Plan::builder("child");
    child
        .resource("lease")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(())) })
        .release({
            let observed = observed.clone();
            move |cx, _| {
                *observed.lock().expect("observed") = remaining(&cx);
                async { Ok(()) }
            }
        });
    let child = child
        .build(Policy::FailFast, Shutdown::within(ms(20)), Mode::Finite)
        .expect("valid child");
    let mut parent = Plan::builder("parent");
    let component = parent.component("child", &child, ());
    parent
        .resource("dependent")
        .needs(component)
        .acquire(|cx, _| async move { Ok(cx.hold_value(())) })
        .release(|cx, _| async move {
            cx.sleep(ms(7)).await;
            Ok(())
        });
    let parent = parent
        .build(Policy::FailFast, Shutdown::within(ms(20)), Mode::Finite)
        .expect("valid parent");
    let tokio_rt = paused();
    let adapter = host(&tokio_rt);
    let report = tokio_rt.block_on(parent.start(adapter.clone(), ()));

    assert!(report.is_clean(), "{report}");
    assert_eq!(*observed.lock().expect("observed"), Some(ms(13)));
    assert_eq!(adapter.tracked(), 0);
}

#[test]
fn interrupted_initializer_replaces_its_within_deadline_with_stop_grace() {
    let observed = Arc::new(Mutex::new(None));
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let started_tx = Arc::new(Mutex::new(Some(started_tx)));
    let mut p = Plan::builder("initializer stop grace");
    p.service("worker")
        .within(ms(1))
        .stop_within(ms(5))
        .initialize({
            let observed = observed.clone();
            move |cx, ()| {
                let observed = observed.clone();
                let started_tx = started_tx.clone();
                async move {
                    if let Some(tx) = started_tx.lock().expect("started").take() {
                        let _ = tx.send(());
                    }
                    cx.stop().await;
                    cx.sleep(ms(3)).await;
                    *observed.lock().expect("observed") = remaining(&cx);
                    Ok(())
                }
            }
        })
        .serve(|cx, _| async move {
            cx.stop().await;
            Ok(())
        });
    let plan = p
        .build(Policy::FailFast, Shutdown::within(ms(100)), Mode::Resident)
        .expect("valid");
    let tokio_rt = paused();
    let adapter = host(&tokio_rt);
    let report = tokio_rt.block_on(async {
        let mut running = std::pin::pin!(plan.start(adapter.clone(), ()));
        let control = running.handle();
        let mut started_rx = std::pin::pin!(started_rx);
        std::future::poll_fn(|cx| {
            assert!(running.as_mut().poll(cx).is_pending());
            match started_rx.as_mut().poll(cx) {
                Poll::Ready(Ok(())) => Poll::Ready(()),
                Poll::Ready(Err(_)) => panic!("initializer sender dropped"),
                Poll::Pending => Poll::Pending,
            }
        })
        .await;
        control.shutdown();
        running.await
    });

    assert!(report.is_clean(), "{report}");
    assert_eq!(*observed.lock().expect("observed"), Some(ms(2)));
    assert_eq!(adapter.tracked(), 0);
}
