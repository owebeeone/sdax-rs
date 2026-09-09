use sdax::prelude::*;
use std::sync::Arc;

#[test]
fn required_output_distinguishes_all_three_cases() {
    let mut success = Report::empty(Outcome::Ok);
    success.output = Some(Arc::new(42));
    assert_eq!(
        *success.into_required_output().expect("completed value"),
        42
    );

    let missing = Report::<u32>::empty(Outcome::Ok)
        .into_required_output()
        .expect_err("no output");
    match missing {
        RequiredOutputError::MissingOutput(report) => assert!(report.is_clean()),
        RequiredOutputError::Failed(_) => panic!("the run was clean"),
    }

    let failed = Report::<u32>::empty(Outcome::Cancelled)
        .into_required_output()
        .expect_err("cancelled");
    assert!(matches!(&failed, RequiredOutputError::Failed(_)));
    assert_eq!(failed.report().outcome, Outcome::Cancelled);
    let original = failed.into_report();
    assert_eq!(original.outcome, Outcome::Cancelled);
}
