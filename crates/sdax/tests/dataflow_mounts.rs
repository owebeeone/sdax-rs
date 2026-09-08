//! Data-provenance regression guards for independently mounted definitions.
use sdax::*;
use std::collections::BTreeSet;
use std::sync::Arc;

#[test]
fn repeated_mounts_keep_distinct_input_and_formal_import_provenance() {
    let mut child = Plan::with_input::<u32>("child");
    let input = child.input();
    let rate = child.port::<u32>("rate");
    let output = child
        .step("multiply")
        .needs((input, rate))
        .run(|_, values: (Arc<u32>, Arc<u32>)| async move { Ok(*values.0 * *values.1) });
    let child = child
        .export(output)
        .build(Policy::FailFast, Shutdown::unbounded(), Mode::Finite)
        .unwrap();
    let mut parent = Plan::with_input::<u32>("parent");
    let input = parent.input();
    let alternate = parent
        .step("alternate")
        .needs(input)
        .run(|_, input: Arc<u32>| async move { Ok(*input + 1) });
    let rate_a = parent.step("rate_a").run(|_, ()| async { Ok(2u32) });
    let rate_b = parent.step("rate_b").run(|_, ()| async { Ok(3u32) });
    parent.component("left", &child.bind(rate, rate_a).unwrap(), input);
    parent.component("right", &child.bind(rate, rate_b).unwrap(), alternate);
    let parent = parent
        .build(Policy::FailFast, Shutdown::unbounded(), Mode::Finite)
        .unwrap();
    let view = parent.inspect_dataflow();
    assert_eq!(view, parent.inspect_dataflow());
    let paths: BTreeSet<_> = view
        .nodes
        .iter()
        .map(|node| node.path.to_string())
        .collect();
    assert_eq!(paths.len(), view.nodes.len());
    let inputs: Vec<_> = view
        .nodes
        .iter()
        .filter(|node| node.kind == Kind::Input)
        .map(|node| node.path.to_string())
        .collect();
    assert_eq!(inputs, ["input", "left/input", "right/input"]);
    let sources: Vec<_> = view
        .edges
        .iter()
        .filter(|edge| edge.reason == Reason::Import)
        .map(|edge| (edge.from.to_string(), edge.to.to_string()))
        .collect();
    assert_eq!(
        sources,
        [
            ("left/input".into(), "input".into()),
            ("left/rate".into(), "rate_a".into()),
            ("right/input".into(), "alternate".into()),
            ("right/rate".into(), "rate_b".into()),
        ]
    );
    for mount in ["left", "right"] {
        for dependency in ["input", "rate"] {
            assert!(view.edges.iter().any(|edge| {
                edge.from.to_string() == format!("{mount}/multiply")
                    && edge.to.to_string() == format!("{mount}/{dependency}")
            }));
        }
    }
    let lifecycle = parent.inspect();
    assert!(lifecycle.node("input").is_none());
    assert!(lifecycle.node("left/input").is_none());
    assert_eq!(
        lifecycle.node("left/multiply").unwrap().needs,
        [NodePath::root("rate_a")]
    );
    assert_eq!(
        lifecycle.node("right/multiply").unwrap().needs,
        [NodePath::root("alternate"), NodePath::root("rate_b")]
    );
}
