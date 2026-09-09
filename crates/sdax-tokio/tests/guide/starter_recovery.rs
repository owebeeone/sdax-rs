//! Complete recovery starter using the shared in-memory backend.
//! Keep the ownership declarations when adapting the operations to application I/O.

use crate::starter_support::Environment;
use sdax::prelude::*;
use std::sync::Arc;
use std::time::Duration;
/// Declare the complete plan, including resource ownership and completed output.
pub fn build(env: Environment) -> Plan<u32, u32> {
    let mut p = Plan::with_input::<u32>("dispatch");
    let n = p.input();
    let id = p
        .step("identity")
        .needs(n)
        .run(|_, n: Arc<u32>| async move { Ok(*n * 13 + 29) });
    let a = env.clone();
    let r = env.clone();
    let z = env.clone();
    let _send: Key<u32> = p
        .effect("transmit")
        .idempotent()
        .within(Duration::from_millis(7))
        .on_ambiguous(Ambiguity::Recover)
        .identified_by(id)
        .perform(move |cx, ((), id): ((), Arc<u32>)| {
            let e = a.clone();
            async move { cx.hold(move || e.request(&id)).await }
        })
        .recover_unknown(move |_, id: Arc<u32>| {
            let e = r.clone();
            async move {
                e.reconcile(&id);
                Ok(if e.resolve {
                    Recovery::Resolved
                } else {
                    Recovery::StillUnknown
                })
            }
        })
        .compensate(move |_, _: Arc<u32>| {
            let e = z.clone();
            async move {
                e.record("unexpected-compensation");
                Ok(())
            }
        });
    let out = p
        .step("independent")
        .needs(n)
        .run(|_, n: Arc<u32>| async move { Ok(*n + 12) });
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
    use crate::starter_support::Environment;
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
    fn count(events: &[String], prefix: &str) -> usize {
        events.iter().filter(|e| e.starts_with(prefix)).count()
    }

    fn identity(e: &Environment, n: u32, recovery: bool) {
        let x = e.events();
        let req = x
            .iter()
            .filter(|s| s.starts_with("request:"))
            .collect::<Vec<_>>();
        assert_eq!(req.len(), 1);
        let r = req[0].split(':').collect::<Vec<_>>();
        assert_eq!(r[1], (n * 13 + 29).to_string());
        let rec = x
            .iter()
            .filter(|s| s.starts_with("reconcile:"))
            .collect::<Vec<_>>();
        assert_eq!(rec.len(), usize::from(recovery));
        if recovery {
            assert_eq!(
                req[0].strip_prefix("request:"),
                rec[0].strip_prefix("reconcile:")
            );
        }
        assert_eq!(count(&x, "unexpected-compensation"), 0);
    }
    #[test]
    fn behavior_resolved_identity() {
        run(async {
            let e = Environment::default().with_resolved_recovery();
            let r = finite(e.clone(), 4).await;
            identity(&e, 4, true);
            assert_eq!(r.output.as_deref(), Some(&16));
            assert!(r.ambiguous.is_empty());
            assert!(r.cleanup_failures.is_empty());
            assert!(r
                .faults
                .iter()
                .any(|f| matches!(f.kind, FaultKind::Timeout)));
            assert!(candidate::boundary(r).is_err());
        })
    }
    #[test]
    fn diagnostic_unresolved_identity() {
        run(async {
            let e = Environment::default();
            let r = finite(e.clone(), 11).await;
            identity(&e, 11, true);
            assert_eq!(r.output.as_deref(), Some(&23));
            assert_eq!(r.ambiguous.len(), 1);
            assert_eq!(r.cleanup_failures.len(), 1);
            let text = candidate::boundary(r).unwrap_err();
            assert!(text.contains("transmit"), "{text}");
            assert!(text.to_lowercase().contains("timeout"), "{text}");
        })
    }
    #[test]
    fn diagnostic_known_failure_is_not_unknown() {
        run(async {
            let e = Environment::default().with_request_failure();
            let r = finite(e.clone(), 7).await;
            identity(&e, 7, false);
            assert!(r.ambiguous.is_empty());
            assert!(r.cleanup_failures.is_empty());
            assert_eq!(r.output.as_deref(), Some(&19));
            let text = candidate::boundary(r).unwrap_err();
            for s in ["request rejected", "schema 6"] {
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
