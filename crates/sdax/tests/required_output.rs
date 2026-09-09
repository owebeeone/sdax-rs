use sdax::*;
use std::sync::Arc;

#[test]
fn required_output_preserves_the_original_arc_and_accepts_non_debug_output() {
    struct Value;
    let output = Arc::new(Value);
    let mut report = Report::empty(Outcome::Ok);
    report.output = Some(output.clone());
    let actual = report.into_required_output().unwrap();
    assert!(Arc::ptr_eq(&actual, &output));
    fn is_error<T: std::error::Error>() {}
    is_error::<RequiredOutputError<Value>>();
}

#[test]
fn clean_missing_output_is_typed_and_retains_trace() {
    let mut report: Report<u32> = Report::empty(Outcome::Ok);
    report.trace = Some(Trace::default());
    let error = report.into_required_output().unwrap_err();
    assert!(matches!(&error, RequiredOutputError::MissingOutput(_)));
    assert!(error.to_string().contains("missing completed output"));
    assert!(error.report().trace.is_some());
    assert!(error.into_report().is_clean());
    assert!(Report::<u32>::empty(Outcome::Ok)
        .into_result()
        .unwrap()
        .is_none());
}

#[test]
fn failed_run_takes_precedence_and_preserves_every_report_field() {
    for has_output in [false, true] {
        let mut report = Report::<u32>::empty(Outcome::Failed);
        report.output = has_output.then(|| Arc::new(7));
        report.trace = Some(Trace::default());
        report.faults.push(Fault {
            node: NodePath::root("run"),
            order: RecordOrder::default(),
            phase: Phase::Run,
            kind: FaultKind::Error(std::io::Error::other("original").into()),
        });
        report.cleanup_failures.push(Fault {
            node: NodePath::root("cleanup"),
            order: RecordOrder::default(),
            phase: Phase::ReleaseBody,
            kind: FaultKind::Error("cleanup cause".into()),
        });
        report.incomplete.push(NodeRecord {
            node: NodePath::root("unfinished"),
            order: RecordOrder::default(),
        });
        report.ambiguous.push(NodeRecord {
            node: NodePath::root("unknown"),
            order: RecordOrder::default(),
        });
        let error = report.into_required_output().unwrap_err();
        assert!(matches!(&error, RequiredOutputError::Failed(_)));
        let rendered = error.to_string();
        for expected in ["original", "cleanup cause", "unfinished", "unknown"] {
            assert!(rendered.contains(expected));
        }
        let report = error.into_report();
        assert_eq!(report.outcome, Outcome::Failed);
        assert_eq!(report.output.is_some(), has_output);
        assert!(report.trace.is_some());
        assert_eq!(
            (
                report.faults.len(),
                report.cleanup_failures.len(),
                report.incomplete.len(),
                report.ambiguous.len()
            ),
            (1, 1, 1, 1)
        );
        match &report.faults[0].kind {
            FaultKind::Error(e) => assert!(e.downcast_ref::<std::io::Error>().is_some()),
            _ => panic!("typed cause lost"),
        }
    }
}
