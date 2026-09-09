use sdax::*;
use sdax_tokio::{PlanStart, TokioRuntime};
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn plan(after: bool, recover: bool, seen: Arc<Mutex<Vec<(u64, usize, u32)>>>) -> Plan<(), u64> {
    let mut p = Plan::with_input::<u64>("configured");
    let input = p.input();
    let id = p
        .step("identity")
        .needs(input)
        .run(|_, id| async move { Ok(*id) });
    let value = p.step("value").run(|_, ()| async { Ok(5u32) });
    let node = p.effect("operation").on_ambiguous(if recover {
        Ambiguity::Recover
    } else {
        Ambiguity::Retry
    });
    let node = if after {
        node.identified_by(id)
            .needs(())
            .needs((value, id))
            .within(Duration::from_millis(1))
            .retry(Retry::attempts(2))
            .idempotent()
    } else {
        node.needs((value, id))
            .within(Duration::from_millis(1))
            .retry(Retry::attempts(2))
            .idempotent()
            .identified_by(id)
    };
    let recovered = seen.clone();
    node.perform(move |cx, ((value, ordinary_id), id)| {
        let seen = seen.clone();
        async move {
            assert!(Arc::ptr_eq(&ordinary_id, &id));
            let pointer = Arc::as_ptr(&id) as usize;
            let attempt = cx.attempt();
            seen.lock().unwrap().push((*id, pointer, attempt));
            cx.hold(|| async move {
                if recover || attempt == 1 {
                    std::future::pending::<()>().await;
                }
                Ok::<_, Error>(*value)
            })
            .await
        }
    })
    .recover_unknown(move |_, id| {
        let recovered = recovered.clone();
        async move {
            recovered
                .lock()
                .unwrap()
                .push((*id, Arc::as_ptr(&id) as usize, 0));
            Ok(Recovery::Resolved)
        }
    })
    .compensate(|_, receipt| async move {
        assert_eq!(*receipt, 5);
        Ok(())
    });
    p.build(
        Policy::FailFast,
        Shutdown::within(Duration::from_secs(1)),
        Mode::Finite,
    )
    .unwrap()
}

#[test]
fn configuration_orders_have_identical_edges_and_runtime_behavior() {
    for recover in [false, true] {
        let before = plan(false, recover, Default::default()).inspect();
        let after = plan(true, recover, Default::default()).inspect();
        assert_eq!(before, after);
        let operation = after
            .nodes
            .iter()
            .find(|n| n.path.to_string().ends_with("operation"))
            .unwrap();
        assert_eq!(operation.needs.len(), 2, "identity must not be duplicated");
        for after in [false, true] {
            let seen = Arc::new(Mutex::new(Vec::new()));
            let p = plan(after, recover, seen.clone());
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .start_paused(true)
                .build()
                .unwrap();
            let adapter = Arc::new(TokioRuntime::current_thread_no_background_drain(
                rt.handle().clone(),
            ));
            let report = rt.block_on(p.start(adapter.clone(), 77));
            assert!(report.ambiguous.is_empty());
            assert!(report.cleanup_failures.is_empty());
            let events = seen.lock().unwrap();
            assert_eq!(events.len(), 2);
            assert_eq!(events[0].0, 77);
            assert_eq!(events[0].1, events[1].1);
            assert_eq!(events[0].2, 1);
            assert_eq!(events[1].2, if recover { 0 } else { 2 });
            assert_eq!(
                report.outcome,
                if recover {
                    Outcome::Failed
                } else {
                    Outcome::Ok
                }
            );
            assert_eq!(adapter.tracked(), 0);
        }
    }
}
