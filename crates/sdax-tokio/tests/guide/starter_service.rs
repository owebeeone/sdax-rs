//! Complete service starter using the shared in-memory backend.
//! Keep the ownership declarations when adapting the operations to application I/O.

use crate::starter_support::{Environment, Lease};
use sdax::prelude::*;
use std::sync::Arc;
use std::time::Duration;
/// Declare the complete plan, including resource ownership and completed output.
pub fn build(env: Environment) -> Plan<u32, u32> {
    let mut p = Plan::with_input::<u32>("dispatch");
    let n = p.input();
    let a = env.clone();
    let z = env.clone();
    let spool = p
        .resource("spool")
        .needs(n)
        .acquire(move |cx, n: Arc<u32>| {
            let e = a.clone();
            async move { cx.hold(move || e.acquire("spool", *n + 9, None)).await }
        })
        .release(move |_, r: Arc<Lease>| {
            let e = z.clone();
            async move { e.release(&r) }
        });

    let a = env.clone();
    let s = env.clone();
    let o = env.clone();
    let handle = p
        .service("dispatcher")
        .needs((n, spool))
        .idempotent()
        .restart(Restart::on_error(Backoff::fixed(Duration::from_millis(3))).max(1))
        .stop_within(Duration::from_millis(30))
        .initialize(move |_, (n, _): (Arc<u32>, Arc<Lease>)| {
            let e = a.clone();
            async move {
                e.record("initialized");
                Ok(*n * 4 + 1)
            }
        })
        .serve(move |cx, h: Arc<u32>| {
            let e = s.clone();
            async move {
                e.record(format!(
                    "episode:{}:{}:{}",
                    cx.episode(),
                    *h,
                    Arc::as_ptr(&h) as usize
                ));
                if cx.episode() == 1 {
                    return Err(crate::starter_support::fault("serving retry", "channel 2"));
                }
                e.begin_work();
                cx.stop().await;
                e.drain_work();
                e.record("stopped");
                Ok(())
            }
        });
    let out = p.step("observer").needs(handle).run(move |_, h: Arc<u32>| {
        let e = o.clone();
        async move {
            e.record(format!("observed:{}:{}", *h, Arc::as_ptr(&h) as usize));
            Ok(*h)
        }
    });
    p.export(out)
        .build(
            Policy::Isolate,
            Shutdown::within(Duration::from_millis(200)),
            Mode::Resident,
        )
        .unwrap()
}
/// Return completed output or full failure text, including absent output.
pub fn boundary(report: Report<u32>) -> Result<u32, String> {
    report
        .into_required_output()
        .map(|output| *output)
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod checks {
    use super as candidate;
    use crate::starter_support::Environment;
    use sdax::Report;
    use sdax_tokio::{PlanStart, TokioRuntime};
    use std::{sync::Arc, time::Duration};
    fn run(f: impl std::future::Future<Output = ()>) {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .start_paused(true)
            .build()
            .unwrap()
            .block_on(f)
    }
    fn rt() -> Arc<TokioRuntime> {
        Arc::new(TokioRuntime::current_thread_no_background_drain(
            tokio::runtime::Handle::current(),
        ))
    }
    fn position(events: &[String], s: &str) -> usize {
        events
            .iter()
            .position(|e| e == s)
            .unwrap_or_else(|| panic!("missing {s}: {events:?}"))
    }
    fn count(events: &[String], prefix: &str) -> usize {
        events.iter().filter(|e| e.starts_with(prefix)).count()
    }

    async fn resident(n: u32) -> (Environment, Report<u32>) {
        let e = Environment::default();
        let r = rt();
        let mut running = candidate::build(e.clone()).start(r.clone(), n);
        tokio::time::timeout(Duration::from_secs(1), running.ready())
            .await
            .unwrap()
            .unwrap();
        tokio::time::timeout(Duration::from_millis(100), async {
            while e.work() == 0 {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("second episode must start");
        running.shutdown();
        let report = tokio::time::timeout(Duration::from_secs(1), running)
            .await
            .unwrap();
        r.shutdown(Duration::from_millis(100)).await.unwrap();
        assert_eq!(r.tracked(), 0);
        (e, report)
    }
    #[test]
    fn behavior_restart_same_handle() {
        run(async {
            let (e, r) = resident(6).await;
            let x = e.events();
            assert_eq!(count(&x, "initialized"), 1);
            assert_eq!(count(&x, "episode:"), 2);
            assert_eq!(count(&x, "observed:"), 1);
            let a = x
                .iter()
                .find(|s| s.starts_with("episode:1:"))
                .unwrap()
                .strip_prefix("episode:1:")
                .unwrap();
            let b = x
                .iter()
                .find(|s| s.starts_with("episode:2:"))
                .unwrap()
                .strip_prefix("episode:2:")
                .unwrap();
            let o = x
                .iter()
                .find(|s| s.starts_with("observed:"))
                .unwrap()
                .strip_prefix("observed:")
                .unwrap();
            assert_eq!(a, b);
            assert_eq!(a, o);
            assert_eq!(candidate::boundary(r).unwrap(), 25);
        })
    }
    #[test]
    fn behavior_drained_before_release() {
        run(async {
            let (e, r) = resident(9).await;
            let x = e.events();
            assert_eq!(e.work(), 0);
            assert_eq!(e.live(), 0);
            assert_eq!(count(&x, "work-start"), 1);
            assert_eq!(count(&x, "work-drained"), 1);
            assert_eq!(count(&x, "stopped"), 1);
            assert!(position(&x, "work-drained") < position(&x, "close:spool"));
            assert_eq!(candidate::boundary(r).unwrap(), 37);
        })
    }
    #[test]
    fn diagnostic_recovered_service_is_clean() {
        run(async {
            let (_, r) = resident(1).await;
            assert!(r.faults.is_empty());
            assert!(r.cleanup_failures.is_empty());
            assert!(r.ambiguous.is_empty());
            assert!(r.incomplete.is_empty());
            assert_eq!(candidate::boundary(r).unwrap(), 5);
        })
    }

    #[test]
    fn diagnostic_missing_output_returns_error() {
        let report = sdax::Report::<u32>::empty(sdax::Outcome::Ok);
        let outcome =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| candidate::boundary(report)));
        assert!(
            outcome.is_ok(),
            "missing completed output must return Err(String), never panic"
        );
        assert!(
            outcome.unwrap().is_err(),
            "missing completed output must not return an invented success"
        );
    }
}
