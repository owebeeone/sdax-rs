//! Immutable lookup regressions: skipped declarations, mounts and instance growth.
use super::*;
use crate::{Plan, Policy, Shutdown};
use std::time::Duration;

#[test]
fn cached_run_index_distinguishes_holes_unknown_plans_and_equal_declaration_indices() {
    let mut index = RunIndex::default();
    index.add_scope(11, 4);
    index.add_scope(22, 4);
    index.insert(RawKey { plan: 11, idx: 1 }, 0);
    index.insert(RawKey { plan: 11, idx: 3 }, 1);
    index.insert(RawKey { plan: 22, idx: 1 }, 2);
    assert_eq!(index.get(RawKey { plan: 11, idx: 0 }), None);
    assert_eq!(index.get(RawKey { plan: 11, idx: 1 }), Some(0));
    assert_eq!(index.get(RawKey { plan: 11, idx: 3 }), Some(1));
    assert_eq!(index.get(RawKey { plan: 22, idx: 1 }), Some(2));
    assert_eq!(
        index.get(RawKey {
            plan: 11,
            idx: u32::MAX
        }),
        None
    );
    assert_eq!(
        index.get(RawKey {
            plan: u64::MAX,
            idx: 1
        }),
        None
    );
}

fn budget() -> Shutdown {
    Shutdown::within(Duration::from_secs(1))
}

#[test]
fn cached_index_matches_node_keys_for_reused_components_and_input_holes() {
    let mut child = Plan::with_input::<u64>("Child");
    let input = child.input();
    let value = child
        .step("Value")
        .needs(input)
        .run(|_, value| async move { Ok(*value) });
    let child = child
        .export(value)
        .build(Policy::FailFast, budget(), Mode::Finite)
        .unwrap();
    let mut root = Plan::with_input::<u64>("Root");
    let input = root.input();
    root.component("Left", &child, input);
    root.component("Right", &child, input);
    let plan = root
        .build(Policy::FailFast, budget(), Mode::Finite)
        .unwrap();
    let table = plan.layout.as_ref().unwrap();
    assert_eq!(
        table.index_of(input.raw()),
        None,
        "input is not an executable node"
    );
    for (i, node) in table.nodes.iter().enumerate() {
        assert_eq!(table.index_of(node.key), Some(i));
    }
    assert_eq!(
        table.index_of(value.raw()),
        None,
        "unmounted child declaration is not a run identity"
    );
}

#[test]
fn dynamic_index_growth_preserves_static_and_previous_instance_keys() {
    let mut child = Plan::with_input::<u64>("Child");
    let input = child.input();
    child
        .step("Value")
        .needs(input)
        .run(|_, value| async move { Ok(*value) });
    let child = child
        .build(Policy::FailFast, budget(), Mode::Finite)
        .unwrap();
    let mut root = Plan::builder("Root");
    let template = root.template("Children", &child);
    let root = root
        .build(Policy::FailFast, budget(), Mode::Resident)
        .unwrap();
    let mut table = root.layout.as_ref().unwrap().as_ref().clone();
    let template = table.index_of(template.node()).unwrap();
    let cached_static = table.clone();
    let (_, first) = table.instantiate(template, InstanceId(100)).unwrap();
    let first_key = table.nodes[first[0]].key;
    let (_, second) = table.instantiate(template, InstanceId(200)).unwrap();
    let second_key = table.nodes[second[0]].key;
    assert_ne!(first_key, second_key);
    assert_eq!(
        cached_static.index_of(first_key),
        None,
        "a run's growth cannot alter cached static topology"
    );
    assert_eq!(table.index_of(first_key), Some(first[0]));
    assert_eq!(table.index_of(second_key), Some(second[0]));
    for (i, node) in table.nodes.iter().enumerate() {
        assert_eq!(table.index_of(node.key), Some(i));
    }
}
