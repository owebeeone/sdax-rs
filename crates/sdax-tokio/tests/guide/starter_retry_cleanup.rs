//! Complete retry cleanup starter using the shared in-memory backend.
//! Keep the ownership declarations when adapting the operations to application I/O.

use crate::starter_support::{Environment, Lease};
use sdax::prelude::*;
use std::sync::Arc;
use std::time::Duration;
/// Declare the complete plan, including resource ownership and completed output.
pub fn build(env: Environment) -> Plan<u32, u32> {
    let mut p = Plan::with_input::<u32>("routing");
    let n = p.input();
    let a = env.clone();
    let z = env.clone();
    let foundation = p
        .resource("foundation")
        .needs(n)
        .acquire(move |cx, n: Arc<u32>| {
            let e = a.clone();
            async move { cx.hold(move || e.acquire("foundation", *n + 2, None)).await }
        })
        .release(move |_, r: Arc<Lease>| {
            let e = z.clone();
            async move { e.release(&r) }
        });

    let a = env.clone();
    let z = env.clone();
    let transfer = p
        .resource("transfer")
        .needs(foundation)
        .idempotent()
        .retry(Retry::attempts(2).backoff(Backoff::fixed(Duration::from_millis(2))))
        .acquire(move |cx, parent: Arc<Lease>| {
            let e = a.clone();
            let attempt = cx.attempt();
            async move { cx.hold(move || e.transfer(attempt, &parent)).await }
        })
        .release(move |_, r: Arc<Lease>| {
            let e = z.clone();
            async move { e.release(&r) }
        });
    let a = env.clone();
    let z = env.clone();
    let seal = p
        .resource("seal")
        .needs(transfer)
        .acquire(move |cx, parent: Arc<Lease>| {
            let e = a.clone();
            async move {
                cx.hold(move || e.acquire("seal", parent.value * 2, Some(&parent)))
                    .await
            }
        })
        .release(move |_, r: Arc<Lease>| {
            let e = z.clone();
            async move { e.release(&r) }
        });

    let out = p
        .step("value")
        .needs(seal)
        .run(|_, r: Arc<Lease>| async move { Ok(r.value + 1) });
    p.export(out)
        .build(
            Policy::Isolate,
            Shutdown::within(Duration::from_millis(200)),
            Mode::Finite,
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
    use crate::starter_support::{Environment, OperationError};
    use sdax::{FaultKind, Report};
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
    async fn finite(e: Environment, n: u32) -> Report<u32> {
        let r = rt();
        let report = tokio::time::timeout(
            Duration::from_secs(2),
            candidate::build(e).start(r.clone(), n),
        )
        .await
        .expect("run deadline");
        r.shutdown(Duration::from_millis(100)).await.unwrap();
        assert_eq!(r.tracked(), 0);
        report
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

    #[test]
    fn behavior_second_attempt_and_cleanup() {
        run(async {
            let e = Environment::default().with_cleanup_failure("seal");
            let r = finite(e.clone(), 8).await;
            assert_eq!(r.output.as_deref(), Some(&43));
            let x = e.events();
            assert_eq!(
                x.iter()
                    .filter(|s| s.starts_with("transfer-attempt:"))
                    .cloned()
                    .collect::<Vec<_>>(),
                ["transfer-attempt:1", "transfer-attempt:2"]
            );
            assert_eq!(count(&x, "open:transfer:"), 1);
            assert_eq!(count(&x, "open:seal:"), 1);
            assert!(position(&x, "close:seal") < position(&x, "close:transfer"));
            assert!(position(&x, "close:transfer") < position(&x, "close:foundation"));
            assert_eq!(e.live(), 0);
        })
    }
    #[test]
    fn diagnostic_typed_cleanup_sources() {
        run(async {
            let e = Environment::default().with_cleanup_failure("seal");
            let r = finite(e, 8).await;
            assert_eq!(r.cleanup_failures.len(), 1);
            let fault = &r.cleanup_failures[0];
            match &fault.kind {
                FaultKind::Error(e) => assert!(e.downcast_ref::<OperationError>().is_some()),
                _ => panic!("lost typed error"),
            };
            let text = candidate::boundary(r).unwrap_err();
            for s in ["seal", "flush seal", "disk rack 7"] {
                assert!(text.contains(s), "{text}");
            }
        })
    }
    #[test]
    fn diagnostic_retry_exhaustion() {
        run(async {
            let e = Environment::default().with_unavailable_transfer();
            let r = finite(e.clone(), 3).await;
            let x = e.events();
            assert_eq!(count(&x, "transfer-attempt:"), 2);
            assert_eq!(count(&x, "open:transfer:"), 0);
            assert_eq!(count(&x, "open:seal:"), 0);
            assert_eq!(e.live(), 0);
            assert!(x.contains(&"close:foundation".into()));
            let text = candidate::boundary(r).unwrap_err();
            for s in ["transfer", "gate warming", "thermostat"] {
                assert!(text.contains(s), "{text}");
            }
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
