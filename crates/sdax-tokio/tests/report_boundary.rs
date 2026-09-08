//! Public-API consumer regression guards for the required String boundary.
use sdax::*;
use sdax_tokio::{PlanStart, TokioRuntime};
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug)]
struct ContextError {
    message: &'static str,
    source: Error,
}
impl fmt::Display for ContextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message)
    }
}
impl std::error::Error for ContextError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.as_ref())
    }
}

fn cause(message: &'static str, middle: &'static str, leaf: &'static str) -> Error {
    Box::new(ContextError {
        message,
        source: Box::new(ContextError {
            message: middle,
            source: std::io::Error::other(leaf).into(),
        }),
    })
}

#[derive(Debug)]
struct UnresolvedOperation(u64);
impl fmt::Display for UnresolvedOperation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "operation {} remains unresolved", self.0)
    }
}
impl std::error::Error for UnresolvedOperation {}

fn build<O, I>(p: PlanBuilder<O, I>) -> Plan<O, I> {
    p.build(
        Policy::FailFast,
        Shutdown::within(Duration::from_millis(20)),
        Mode::Finite,
    )
    .expect("valid plan")
}

fn nested<O: Send + Sync + 'static>(leaf: Plan<O, u64>) -> Plan<O, u64> {
    let mut middle = Plan::with_input::<u64>("Middle");
    let input = middle.input();
    let output = middle.component("reservation", &leaf, input);
    let middle = build(middle.export(output));
    let mut root = Plan::with_input::<u64>("Request");
    let input = root.input();
    let output = root.component("region", &middle, input);
    build(root.export(output))
}

fn execute<O: Send + Sync + 'static>(plan: &Plan<O, u64>, input: u64) -> Report<O> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .expect("runtime");
    let adapter = Arc::new(TokioRuntime::current_thread_no_background_drain(
        runtime.handle().clone(),
    ));
    let report = runtime.block_on(plan.start(adapter.clone(), input));
    assert_eq!(adapter.tracked(), 0, "all tasks joined before return");
    report
}

// Keep Report internally. Only this consumer's declared signature requires text.
fn string_boundary<O>(report: Report<O>) -> Result<Arc<O>, String> {
    report
        .into_result()
        .map_err(|report| report.to_string())?
        .ok_or_else(|| "clean run did not produce its declared output".to_owned())
}

fn body_and_cleanup_report(fail: bool) -> Report<u32> {
    let mut p = Plan::with_input::<u64>("Leaf");
    let lease = p
        .resource("Lease")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(17u32)) })
        .release(move |_, _| async move {
            if fail {
                Err(cause("close lease", "storage flush", "disk offline"))
            } else {
                Ok(())
            }
        });
    let answer = p.step("Run").needs(lease).run(move |_, lease| async move {
        if fail {
            Err(cause(
                "submit request",
                "decode response",
                "invalid checksum",
            ))
        } else {
            Ok(*lease + 1)
        }
    });
    execute(&nested(build(p.export(answer))), 1)
}

const BODY_DETAILS: [&str; 5] = [
    "outcome: failed",
    "faults: region/reservation/Run (run)",
    "submit request: decode response: invalid checksum",
    "cleanup_failures: region/reservation/Lease (release)",
    "close lease: storage flush: disk offline",
];

fn assert_body_and_cleanup_details(text: &str) {
    for detail in BODY_DETAILS {
        assert!(text.contains(detail), "missing {detail:?} in {text:?}");
    }
}

#[test]
fn string_consumer_extracts_output_and_preserves_mounted_body_and_cleanup_causes() {
    let success: Result<Arc<u32>, String> = string_boundary(body_and_cleanup_report(false));
    assert_eq!(*success.expect("successful output"), 18);
    let failure: Result<Arc<u32>, String> = string_boundary(body_and_cleanup_report(true));
    assert_body_and_cleanup_details(&failure.expect_err("both bodies failed"));
}

#[test]
fn detail_assertions_reject_a_lossy_first_fault_summary() {
    let report = body_and_cleanup_report(true);
    // A plausible but incomplete consumer: returns the outer body error only.
    let body = report
        .faults
        .iter()
        .find(|fault| fault.phase == Phase::Run)
        .unwrap();
    let lossy = match &body.kind {
        FaultKind::Error(error) => error.to_string(),
        other => panic!("expected typed body error: {other:?}"),
    };
    assert_eq!(lossy, "submit request");
    assert!(std::panic::catch_unwind(|| assert_body_and_cleanup_details(&lossy)).is_err());
    let complete = report.to_string();
    for detail in BODY_DETAILS {
        let incomplete = complete.replace(detail, "[detail omitted]");
        assert!(
            std::panic::catch_unwind(|| assert_body_and_cleanup_details(&incomplete)).is_err(),
            "the diagnostic oracle must reject omission of {detail:?}"
        );
    }
}

#[test]
fn typed_boundary_preserves_original_errors_and_downcastable_nested_sources() {
    let report = body_and_cleanup_report(true)
        .into_result()
        .expect_err("structured failure");
    assert_eq!(
        report.faults.len(),
        3,
        "two mount propagation faults plus the body"
    );
    let body = report
        .faults
        .iter()
        .find(|fault| fault.phase == Phase::Run)
        .unwrap();
    assert_eq!(report.cleanup_failures.len(), 1);
    for (fault, outer, leaf) in [
        (body, "submit request", "invalid checksum"),
        (&report.cleanup_failures[0], "close lease", "disk offline"),
    ] {
        let error = match &fault.kind {
            FaultKind::Error(error) => error,
            other => panic!("original error lost: {other:?}"),
        };
        let original = error.downcast_ref::<ContextError>().expect("outer type");
        assert_eq!(original.message, outer);
        let middle = original
            .source
            .downcast_ref::<ContextError>()
            .expect("middle type");
        let original_leaf = middle
            .source
            .downcast_ref::<std::io::Error>()
            .expect("leaf type");
        assert_eq!(original_leaf.kind(), std::io::ErrorKind::Other);
        assert_eq!(original_leaf.to_string(), leaf);
    }
}

#[test]
fn abandoned_cleanup_cannot_be_mistaken_for_success_at_string_boundary() {
    let mut p = Plan::with_input::<u64>("Leaf");
    let lease = p
        .resource("Lease")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(17u32)) })
        .release(|_, _| async move { std::future::pending().await });
    let done = p
        .step("Done")
        .needs(lease)
        .run(|_, lease| async move { Ok(*lease) });
    let report = execute(&nested(build(p.export(done))), 1);
    assert_eq!(report.outcome, Outcome::Ok);
    assert_eq!(report.incomplete.len(), 1);
    let text = string_boundary(report).expect_err("abandoned obligation is not clean");
    assert!(text.contains("outcome: ok"), "{text}");
    assert!(
        text.contains("incomplete: {region/reservation/Lease}"),
        "{text}"
    );
}

fn unresolved_report(identity: u64) -> Report<()> {
    let mut p = Plan::with_input::<u64>("Leaf");
    let operation = p.input();
    p.effect("Reserve")
        .idempotent()
        .within(Duration::from_millis(1))
        .on_ambiguous(Ambiguity::Recover)
        .identified_by(operation)
        .perform(|cx, ((), _operation)| async move {
            cx.hold(std::future::pending::<Result<u32, Error>>).await
        })
        .recover_unknown(|_, operation| async move {
            Err(Box::new(UnresolvedOperation(*operation)) as Error)
        })
        .persistent();
    execute(&nested(build(p)), identity)
}

#[test]
fn unresolved_operation_identity_and_phase_survive_the_string_boundary() {
    // Domain errors carry printable identity; arbitrary identity types need not
    // implement Display, and StillUnknown itself does not stringify their value.
    for identity in [417, 918] {
        let report = unresolved_report(identity);
        assert_eq!(report.ambiguous.len(), 1);
        let error = match &report.cleanup_failures[0].kind {
            FaultKind::Error(error) => error,
            other => panic!("recovery error lost: {other:?}"),
        };
        assert_eq!(
            error.downcast_ref::<UnresolvedOperation>().unwrap().0,
            identity
        );
        let text = string_boundary(report).expect_err("unknown operation");
        for detail in [
            "faults: region/reservation/Reserve (prepare) timeout".to_owned(),
            "cleanup_failures: region/reservation/Reserve (recover)".to_owned(),
            format!("operation {identity} remains unresolved"),
            "ambiguous: {region/reservation/Reserve}".to_owned(),
        ] {
            assert!(text.contains(&detail), "missing {detail:?} in {text}");
        }
    }
}
