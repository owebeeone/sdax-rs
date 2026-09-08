use sdax::prelude::*;
use std::sync::Arc;

#[test]
fn dataflow_shows_supplied_input_without_inventing_cleanup() {
    let mut p = Plan::with_input::<u32>("Input");
    let input = p.input();
    p.step("Double")
        .needs(input)
        .run(|_, n: Arc<u32>| async move { Ok(*n * 2) });
    let plan = p
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(1)),
            Mode::Finite,
        )
        .unwrap();
    let data = plan.inspect_dataflow();
    assert!(data
        .nodes
        .iter()
        .any(|n| n.kind == Kind::Input && n.path == *"input"));
    assert!(data
        .edges
        .iter()
        .any(|e| e.from == *"Double" && e.to == *"input"));
    assert!(plan.inspect().node("input").is_none());
}
