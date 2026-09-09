//! Complete composition starter using the shared in-memory backend.
//! Keep the ownership declarations when adapting the operations to application I/O.

use crate::starter_support::{Environment, Lease};
use sdax::prelude::*;
use std::sync::Arc;
use std::time::Duration;
/// Declare the complete plan, including resource ownership and completed output.
pub fn build(env: Environment) -> Plan<u32, u32> {
    let mut c = Plan::with_input::<u32>("batch");
    let multiplier = c.input();
    let bin = c.port::<Lease>("bin");
    let a = env.clone();
    let z = env.clone();
    let batch = c
        .resource("batch")
        .needs((multiplier, bin))
        .acquire(move |cx, (m, bin): (Arc<u32>, Arc<Lease>)| {
            let e = a.clone();
            async move {
                cx.hold(move || e.acquire(&format!("batch-{}", *m), bin.value * *m + 4, Some(&bin)))
                    .await
            }
        })
        .release(move |_, r: Arc<Lease>| {
            let e = z.clone();
            async move { e.release(&r) }
        });

    let v = c
        .step("value")
        .needs(batch)
        .run(|_, r: Arc<Lease>| async move { Ok(r.value) });
    let child = c
        .export(v)
        .build(
            Policy::Isolate,
            Shutdown::within(Duration::from_millis(200)),
            Mode::Finite,
        )
        .unwrap();
    let mut p = Plan::with_input::<u32>("bins");
    let n = p.input();
    let a = env.clone();
    let z = env.clone();
    let north = p
        .resource("north-bin")
        .needs(n)
        .acquire(move |cx, n: Arc<u32>| {
            let e = a.clone();
            async move { cx.hold(move || e.acquire("north-bin", *n + 8, None)).await }
        })
        .release(move |_, r: Arc<Lease>| {
            let e = z.clone();
            async move { e.release(&r) }
        });
    let a = env.clone();
    let z = env.clone();
    let south = p
        .resource("south-bin")
        .needs(n)
        .acquire(move |cx, n: Arc<u32>| {
            let e = a.clone();
            async move {
                cx.hold(move || e.acquire("south-bin", 2 * *n + 3, None))
                    .await
            }
        })
        .release(move |_, r: Arc<Lease>| {
            let e = z.clone();
            async move { e.release(&r) }
        });
    let m3 = p.step("three").run(|_, ()| async { Ok(3u32) });
    let m5 = p.step("five").run(|_, ()| async { Ok(5u32) });
    let amber = p.component("amber", &child.bind(bin, north).unwrap(), m3);
    let violet = p.component("violet", &child.bind(bin, south).unwrap(), m5);
    let total = p
        .step("total")
        .needs((amber, violet))
        .run(|_, (a, v): (Arc<u32>, Arc<u32>)| async move { Ok(*a * 100 + *v) });
    p.export(total)
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

    fn verify(e: &Environment, n: u32) {
        let x = e.events();
        for s in [
            "close:batch-3",
            "close:batch-5",
            "close:north-bin",
            "close:south-bin",
        ] {
            assert_eq!(x.iter().filter(|a| a.as_str() == s).count(), 1);
        }
        assert!(position(&x, "close:batch-3") < position(&x, "close:north-bin"));
        assert!(position(&x, "close:batch-5") < position(&x, "close:south-bin"));
        assert!(x.contains(&format!("open:batch-3:{}", (n + 8) * 3 + 4)));
        assert!(x.contains(&format!("open:batch-5:{}", (2 * n + 3) * 5 + 4)));
        assert_eq!(e.live(), 0);
    }
    #[test]
    fn behavior_binding_outputs() {
        run(async {
            for n in [2, 17] {
                let e = Environment::default();
                let r = finite(e.clone(), n).await;
                assert_eq!(
                    candidate::boundary(r).unwrap(),
                    ((n + 8) * 3 + 4) * 100 + (2 * n + 3) * 5 + 4
                );
                verify(&e, n);
            }
        })
    }
    #[test]
    fn diagnostic_cleanup_continues() {
        run(async {
            let e = Environment::default().with_cleanup_failure("batch-3");
            let r = finite(e.clone(), 6).await;
            assert_eq!(r.cleanup_failures.len(), 1);
            let text = candidate::boundary(r).unwrap_err();
            for s in ["amber", "batch", "flush seal", "disk rack 7"] {
                assert!(text.contains(s), "{text}");
            }
            verify(&e, 6);
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
