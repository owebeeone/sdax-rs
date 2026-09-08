//! Cross-feature regression guards: mounted recovery keeps identities and lifetimes.
use sdax::*;
use sdax_tokio::{PlanStart, TokioRuntime};
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::task::Poll;
use std::time::Duration;

type Events = Arc<Mutex<Vec<(u64, &'static str)>>>;

struct Interrupted {
    operation: u64,
    events: Events,
}

impl Drop for Interrupted {
    fn drop(&mut self) {
        self.events.lock().unwrap().push((self.operation, "joined"));
    }
}

#[test]
fn mounted_unknown_recovery_keeps_each_identity_and_parent_lifetime_across_runs() {
    let events: Events = Arc::new(Mutex::new(Vec::new()));
    let mut child = Plan::with_input::<u64>("operation");
    let identity = child.input();
    let lease_port = child.port::<u64>("lease");
    let performing = events.clone();
    let recovering = events.clone();
    let compensating = events.clone();
    child
        .effect("reserve")
        .needs(lease_port)
        .idempotent()
        .on_ambiguous(Ambiguity::Recover)
        .identified_by(identity)
        .perform(move |cx, (lease, operation): (Arc<u64>, Arc<u64>)| {
            let events = performing.clone();
            async move {
                assert_eq!(*lease, *operation / 10);
                cx.hold(|| async move {
                    let _interrupted = Interrupted {
                        operation: *operation,
                        events: events.clone(),
                    };
                    events.lock().unwrap().push((*operation, "started"));
                    std::future::pending::<Result<u64, Error>>().await
                })
                .await
            }
        })
        .recover_unknown(move |cx, operation| {
            let events = recovering.clone();
            async move {
                assert_eq!(cx.attempt(), 1);
                let mut events = events.lock().unwrap();
                assert!(events.contains(&(*operation, "joined")));
                assert!(!events.contains(&(*operation / 10, "released")));
                events.push((*operation, "recovered"));
                Ok(Recovery::Resolved)
            }
        })
        .compensate(move |_, receipt| {
            let events = compensating.clone();
            async move {
                events.lock().unwrap().push((*receipt, "compensated"));
                Ok(())
            }
        });
    let child = child
        .build(
            Policy::Isolate,
            Shutdown::within(Duration::from_millis(500)),
            Mode::Finite,
        )
        .unwrap();
    let mut parent = Plan::with_input::<u64>("parent");
    let run_id = parent.input();
    let released = events.clone();
    let lease = parent
        .resource("lease")
        .needs(run_id)
        .acquire(|cx, run: Arc<u64>| async move { Ok(cx.hold_value(*run)) })
        .release(move |_, run| {
            let events = released.clone();
            async move {
                events.lock().unwrap().push((*run, "released"));
                Ok(())
            }
        });
    let left = parent
        .step("left_identity")
        .needs(run_id)
        .run(|_, run: Arc<u64>| async move { Ok(*run * 10 + 1) });
    let right = parent
        .step("right_identity")
        .needs(run_id)
        .run(|_, run: Arc<u64>| async move { Ok(*run * 10 + 2) });
    let bound = child.bind(lease_port, lease).unwrap();
    parent.component("left", &bound, left);
    parent.component("right", &bound, right);
    let parent = parent
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(1)),
            Mode::Resident,
        )
        .unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .unwrap();
    let host = Arc::new(TokioRuntime::current_thread_no_background_drain(
        runtime.handle().clone(),
    ));
    let reports = runtime.block_on(async {
        let mut first = std::pin::pin!(parent.start(host.clone(), 1));
        let mut second = std::pin::pin!(parent.start(host.clone(), 2));
        let handles = [first.handle(), second.handle()];
        let mut first_report = None;
        let mut second_report = None;
        std::future::poll_fn(|cx| {
            if first_report.is_none() {
                if let Poll::Ready(report) = first.as_mut().poll(cx) {
                    first_report = Some(report);
                }
            }
            if second_report.is_none() {
                if let Poll::Ready(report) = second.as_mut().poll(cx) {
                    second_report = Some(report);
                }
            }
            let started = events
                .lock()
                .unwrap()
                .iter()
                .filter(|(_, event)| *event == "started")
                .count();
            if started == 4 {
                for handle in &handles {
                    handle.cancel();
                }
            } else {
                cx.waker().wake_by_ref();
            }
            if first_report.is_some() && second_report.is_some() {
                Poll::Ready([first_report.take().unwrap(), second_report.take().unwrap()])
            } else {
                Poll::Pending
            }
        })
        .await
    });
    for report in reports {
        assert_eq!(report.outcome, Outcome::Cancelled);
        assert!(report.ambiguous.is_empty(), "{report:?}");
        assert!(report.cleanup_failures.is_empty(), "{report:?}");
        assert!(report.incomplete.is_empty(), "{report:?}");
        let violations = sdax_testkit::invariants::check_report(
            report.trace.as_ref().unwrap(),
            &parent.inspect(),
            &report,
        );
        assert!(violations.is_empty(), "{violations:?}");
    }
    let events = events.lock().unwrap();
    for operation in [11, 12, 21, 22] {
        for phase in ["started", "joined", "recovered"] {
            assert_eq!(
                events.iter().filter(|e| **e == (operation, phase)).count(),
                1
            );
        }
        let recovered = events
            .iter()
            .position(|e| *e == (operation, "recovered"))
            .unwrap();
        let released = events
            .iter()
            .position(|e| *e == (operation / 10, "released"))
            .unwrap();
        assert!(recovered < released, "{events:?}");
    }
    assert_eq!(
        events
            .iter()
            .filter(|(_, event)| *event == "released")
            .count(),
        2
    );
    assert!(!events.iter().any(|(_, event)| *event == "compensated"));
    assert_eq!(host.tracked(), 0);
}
