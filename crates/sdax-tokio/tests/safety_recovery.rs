use sdax::*;
use sdax_tokio::{PlanStart, TokioRuntime};
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn run_case(occurred: bool, recovery: u8, known: bool) -> (Report<()>, Vec<u64>) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let seen = calls.clone();
    let external = Arc::new(Mutex::new(Vec::<u64>::new()));
    let effect_external = external.clone();
    let recover_external = external.clone();
    let mut p = Plan::with_input::<u64>("Recovery");
    let operation = p.input();
    p.effect("Reserve")
        .idempotent()
        .within(Duration::from_millis(5))
        .on_ambiguous(Ambiguity::Recover)
        .identified_by(operation)
        .perform(move |cx, ((), id)| {
            let effect_external = effect_external.clone();
            async move {
                assert_eq!(*id, 42);
                cx.hold(|| async move {
                    if known {
                        return Ok::<u64, Error>(*id + 1);
                    }
                    if occurred {
                        effect_external.lock().unwrap().push(*id);
                    }
                    std::future::pending::<Result<u64, Error>>().await
                })
                .await
            }
        })
        .recover_unknown(move |cx, id| {
            let seen = seen.clone();
            let external = recover_external.clone();
            if recovery == 5 {
                seen.lock().unwrap().push(*id);
                panic!("recovery factory panic");
            }
            async move {
                assert_eq!(cx.attempt(), 1);
                seen.lock().unwrap().push(*id);
                match recovery {
                    0 => {
                        external.lock().unwrap().retain(|stored| stored != &*id);
                        Ok(Recovery::Resolved)
                    }
                    1 => Ok(Recovery::StillUnknown),
                    2 => Err("reconciliation failed".into()),
                    3 => panic!("reconciliation panicked"),
                    _ => std::future::pending().await,
                }
            }
        })
        .compensate(|_, receipt| async move {
            assert_eq!(*receipt, 43);
            Ok(())
        });
    let p = p
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_millis(20)),
            Mode::Finite,
        )
        .unwrap();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .unwrap();
    let adapter = Arc::new(TokioRuntime::current_thread_no_background_drain(
        rt.handle().clone(),
    ));
    let report = rt.block_on(p.start(adapter.clone(), 42));
    assert_eq!(adapter.tracked(), 0);
    if recovery == 0 {
        assert!(external.lock().unwrap().is_empty());
    }
    let violations = sdax_testkit::invariants::check_report(
        report.trace.as_ref().unwrap(),
        &p.inspect(),
        &report,
    );
    assert!(violations.is_empty(), "{violations:?}");
    if known {
        let mut invalid = report.trace.as_ref().unwrap().clone();
        for event in &mut invalid.events {
            if matches!(event.kind, TraceKind::CompensateStart) {
                event.kind = TraceKind::RecoveryStart;
            }
        }
        assert!(
            !sdax_testkit::invariants::check_trace_prefix(&invalid, &p.inspect()).is_empty(),
            "checker must reject recovery of an acknowledged effect"
        );
    }
    let seen = calls.lock().unwrap().clone();
    (report, seen)
}

#[test]
fn unknown_outcome_uses_identity_without_a_receipt() {
    for occurred in [false, true] {
        let (report, calls) = run_case(occurred, 0, false);
        assert_eq!(calls, [42]);
        assert!(
            report.ambiguous.is_empty(),
            "resolved recovery must discharge uncertainty: {report:?}"
        );
        assert!(report.cleanup_failures.is_empty());
        assert!(report
            .trace
            .as_ref()
            .unwrap()
            .events
            .iter()
            .any(|e| matches!(e.kind, TraceKind::Ambiguous)));
    }
}

#[test]
fn known_success_receives_receipt_and_never_runs_unknown_recovery() {
    let (report, calls) = run_case(true, 0, true);
    assert!(calls.is_empty());
    assert_eq!(report.outcome, Outcome::Ok);
    assert!(report.cleanup_failures.is_empty());
}

#[test]
fn unresolved_recovery_preserves_uncertainty_and_failure() {
    for failure in 1..=5 {
        let (report, calls) = run_case(true, failure, false);
        assert_eq!(calls, [42]);
        assert_eq!(report.ambiguous.len(), 1);
        if failure == 4 {
            assert_eq!(report.incomplete.len(), 1);
        }
        assert_eq!(report.cleanup_failures.len(), 1);
        assert_eq!(report.cleanup_failures[0].phase, Phase::Recover);
    }
}

#[test]
fn concurrent_cancellation_recovers_each_persistent_operation_after_join() {
    use std::future::Future;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::task::Poll;
    struct Joined(Arc<AtomicUsize>);
    impl Drop for Joined {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let live = Arc::new(Mutex::new(Vec::new()));
    let releases = Arc::new(Mutex::new(Vec::new()));
    let joined = Arc::new(AtomicUsize::new(0));
    let mut p = Plan::with_input::<u64>("ConcurrentRecover");
    let operation = p.input();
    let effects = live.clone();
    let observed = releases.clone();
    let joined_body = joined.clone();
    let joined_recovery = joined.clone();
    p.effect("RemoteOperation")
        .on_ambiguous(Ambiguity::Recover)
        .idempotent()
        .identified_by(operation)
        .perform(move |cx, ((), operation)| {
            let effects = effects.clone();
            let joined = joined_body.clone();
            async move {
                cx.hold(|| async move {
                    let _joined = Joined(joined);
                    effects.lock().unwrap().push(*operation);
                    std::future::pending::<Result<u8, Error>>().await
                })
                .await
            }
        })
        .recover_unknown(move |_, operation| {
            let observed = observed.clone();
            let joined = joined_recovery.clone();
            async move {
                assert!(
                    joined.load(Ordering::SeqCst) > 0,
                    "interrupted work must join before recovery"
                );
                observed.lock().unwrap().push(*operation);
                Ok(Recovery::Resolved)
            }
        })
        .persistent();
    let p = p
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(1)),
            Mode::Resident,
        )
        .unwrap();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .unwrap();
    let adapter = Arc::new(TokioRuntime::current_thread_no_background_drain(
        rt.handle().clone(),
    ));
    let (a, b) = rt.block_on(async {
        let mut a = std::pin::pin!(p.start(adapter.clone(), 101));
        let mut b = std::pin::pin!(p.start(adapter.clone(), 202));
        let ha = a.handle();
        let hb = b.handle();
        let mut ra = None;
        let mut rb = None;
        std::future::poll_fn(|cx| {
            if ra.is_none() {
                if let Poll::Ready(r) = a.as_mut().poll(cx) {
                    ra = Some(r);
                }
            }
            if rb.is_none() {
                if let Poll::Ready(r) = b.as_mut().poll(cx) {
                    rb = Some(r);
                }
            }
            if live.lock().unwrap().len() == 2 {
                ha.cancel();
                hb.cancel();
            } else {
                cx.waker().wake_by_ref();
            }
            if ra.is_some() && rb.is_some() {
                Poll::Ready((ra.take().unwrap(), rb.take().unwrap()))
            } else {
                Poll::Pending
            }
        })
        .await
    });
    let mut observed = releases.lock().unwrap().clone();
    observed.sort();
    assert_eq!(observed, [101, 202]);
    assert_eq!(joined.load(Ordering::SeqCst), 2);
    for report in [a, b] {
        assert_eq!(report.outcome, Outcome::Cancelled);
        assert!(report.ambiguous.is_empty() && report.cleanup_failures.is_empty());
        assert!(sdax_testkit::invariants::check_report(
            report.trace.as_ref().unwrap(),
            &p.inspect(),
            &report
        )
        .is_empty());
    }
    assert_eq!(adapter.tracked(), 0);
}

#[test]
fn identity_is_stable_across_retries_and_compensation_gets_the_real_receipt() {
    let observed = Arc::new(Mutex::new(Vec::new()));
    let attempts = observed.clone();
    let recovered = observed.clone();
    let mut p = Plan::with_input::<u64>("Retries");
    let id = p.input();
    p.effect("Operation")
        .idempotent()
        .retry(Retry::attempts(2))
        .within(Duration::from_millis(1))
        .on_ambiguous(Ambiguity::Retry)
        .identified_by(id)
        .perform(move |cx, ((), id)| {
            let attempts = attempts.clone();
            async move {
                let attempt = cx.attempt();
                attempts.lock().unwrap().push((*id, attempt));
                cx.hold(|| async move {
                    if attempt == 1 {
                        std::future::pending::<()>().await;
                    }
                    Ok::<_, Error>(*id + 1)
                })
                .await
            }
        })
        .recover_unknown(move |_, _| {
            let recovered = recovered.clone();
            async move {
                recovered.lock().unwrap().push((0, 0));
                Ok(Recovery::Resolved)
            }
        })
        .compensate(|_, receipt| async move {
            assert_eq!(*receipt, 78);
            Ok(())
        });
    let p = p
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(1)),
            Mode::Finite,
        )
        .unwrap();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .unwrap();
    let adapter = Arc::new(TokioRuntime::current_thread_no_background_drain(
        rt.handle().clone(),
    ));
    let report = rt.block_on(p.start(adapter.clone(), 77));
    assert_eq!(*observed.lock().unwrap(), [(77, 1), (77, 2)]);
    assert_eq!(report.outcome, Outcome::Ok);
    assert!(report.ambiguous.is_empty());
    assert_eq!(adapter.tracked(), 0);
}
