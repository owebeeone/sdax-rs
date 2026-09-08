use sdax::{Fault, FaultKind, NodePath, NodeRecord, Outcome, Phase, RecordOrder, Report};

#[test]
fn result_and_display_preserve_actual_causes_and_unresolved_work() {
    let mut report: Report = Report::empty(Outcome::Failed);
    report.faults.push(Fault {
        node: NodePath::root("Pipeline").child("Input"),
        order: RecordOrder::default(),
        phase: Phase::Prepare,
        kind: FaultKind::Error(
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "missing tenant binding; bind the component input",
            )
            .into(),
        ),
    });
    report.cleanup_failures.push(Fault {
        node: NodePath::root("Lease"),
        order: RecordOrder::default(),
        phase: Phase::ReleaseBody,
        kind: FaultKind::Error("close failed".into()),
    });
    report.ambiguous.push(NodeRecord {
        node: NodePath::root("Reservation"),
        order: RecordOrder::default(),
    });
    let error = report.into_result().expect_err("preserve failed report");
    let rendered = error.to_string();
    assert!(rendered.contains("Pipeline/Input"));
    assert!(rendered.contains("prepare"));
    assert!(
        rendered.contains("missing tenant binding; bind the component input"),
        "{rendered}"
    );
    assert!(rendered.contains("close failed"), "{rendered}");
    assert!(rendered.contains("ambiguous: {Reservation}"));
    if let FaultKind::Error(cause) = &error.faults[0].kind {
        assert!(cause.downcast_ref::<std::io::Error>().is_some());
    } else {
        panic!("typed error lost");
    }
}
