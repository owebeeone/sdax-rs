use sdax::*;
use std::collections::HashSet;
use std::sync::Arc;

fn valid_nodes(v: &PlanView) -> HashSet<NodePath> {
    v.nodes.iter().map(|n| n.path.clone()).collect()
}

fn assert_no_missing_targets(v: &PlanView) {
    let nodes = valid_nodes(v);
    let missing: Vec<_> = v
        .edges
        .iter()
        .filter(|e| !nodes.contains(&e.to))
        .map(|e| (e.from.to_string(), e.to.to_string()))
        .collect();
    assert!(
        missing.is_empty(),
        "all declared targets must resolve to declared nodes; found invalid targets: {missing:?}"
    );
}

#[test]
fn direct_child_import_of_root_input_omits_only_the_root_input_wait() {
    let mut root = Plan::with_input::<u32>("Request");
    let request = root.input();
    let token = root
        .resource("Token")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(7u32)) })
        .release(|_cx, _t| async move { Ok(()) });

    let mut child = Plan::builder("Child");
    let request_from_parent = child.import(request);
    let token_from_parent = child.import(token);
    child
        .step("NeedsRequest")
        .needs(request_from_parent)
        .run(|_cx, _v: Arc<u32>| async move { Ok(()) });
    child
        .step("NeedsToken")
        .needs(token_from_parent)
        .run(|_cx, _t: Arc<u32>| async move { Ok(()) });
    let child = child
        .build(
            Policy::Isolate,
            Shutdown::within(std::time::Duration::from_secs(1)),
            Mode::Finite,
        )
        .expect("valid child");
    let _ = root.component("Child", &child, ());
    let root = root
        .build(
            Policy::FailFast,
            Shutdown::within(std::time::Duration::from_secs(10)),
            Mode::Finite,
        )
        .expect("valid root");
    let v = root.inspect();

    assert_no_missing_targets(&v);
    assert!(!v.nodes.is_empty(), "view contains nodes");

    let needs_request = v.node("Child/NeedsRequest").expect("NeedsRequest");
    assert!(
        needs_request.needs.is_empty(),
        "root input is supplied, not a wait edge; got {:?}",
        needs_request.needs
    );
    assert!(
        v.why("Child/NeedsRequest")
            .expect("why")
            .waits_on
            .is_empty(),
        "no waits for supplied root input"
    );

    let needs_token = v.node("Child/NeedsToken").expect("NeedsToken");
    assert_eq!(needs_token.needs, vec![NodePath::root("Token")]);
    assert_eq!(
        v.why("Child/NeedsToken").expect("why").waits_on,
        vec![(NodePath::root("Token"), Reason::Import)]
    );

    let order = v.release_order();
    assert!(order.before("Child/NeedsToken", "Token"));
}

#[test]
fn nested_component_imports_preserve_supplied_root_input_within_chains() {
    let mut root = Plan::with_input::<u32>("Request");
    let request = root.input();
    let token = root
        .resource("Token")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(1u32)) })
        .release(|_cx, _t| async move { Ok(()) });

    let mut child = Plan::builder("Component");
    let from_root_request = child.import(request);
    let from_root_token = child.import(token);

    let mut nested = Plan::builder("Nested");
    let from_nested_request = nested.import(from_root_request);
    let from_nested_token = nested.import(from_root_token);
    nested
        .step("NeedsRequest")
        .needs(from_nested_request)
        .run(|_cx, _v: Arc<u32>| async move { Ok(()) });
    nested
        .step("NeedsToken")
        .needs(from_nested_token)
        .run(|_cx, _t: Arc<u32>| async move { Ok(()) });
    let nested = nested
        .build(
            Policy::Isolate,
            Shutdown::within(std::time::Duration::from_secs(1)),
            Mode::Finite,
        )
        .expect("valid nested");
    let _ = child.component("Nested", &nested, ());

    let child = child
        .build(
            Policy::Isolate,
            Shutdown::within(std::time::Duration::from_secs(1)),
            Mode::Finite,
        )
        .expect("valid child");
    let _ = root.component("Component", &child, ());

    let root = root
        .build(
            Policy::FailFast,
            Shutdown::within(std::time::Duration::from_secs(10)),
            Mode::Finite,
        )
        .expect("valid root");

    let v = root.inspect();
    assert_no_missing_targets(&v);

    let nested_request = v
        .node("Component/Nested/NeedsRequest")
        .expect("Component/Nested/NeedsRequest");
    assert!(nested_request.needs.is_empty());
    assert!(v
        .why("Component/Nested/NeedsRequest")
        .expect("why")
        .waits_on
        .is_empty());

    let nested_token = v
        .node("Component/Nested/NeedsToken")
        .expect("Component/Nested/NeedsToken");
    assert_eq!(nested_token.needs, vec![NodePath::root("Token")]);
    assert_eq!(
        v.why("Component/Nested/NeedsToken").expect("why").waits_on,
        vec![(NodePath::root("Token"), Reason::Import)]
    );

    let order = v.release_order();
    assert!(order.before("Component/Nested/NeedsRequest", "Component"));
}

#[test]
fn template_imports_root_input_without_phantom_waits() {
    let mut root = Plan::with_input::<u32>("Request");
    let request = root.input();
    let token = root
        .resource("Token")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(3u32)) })
        .release(|_cx, _t| async move { Ok(()) });

    let mut template = Plan::with_input::<u8>("Template");
    let template_input = template.input();
    let from_root_request = template.import(request);
    let from_root_token = template.import(token);

    template
        .step("NeedsTemplateInput")
        .needs(template_input)
        .run(|_cx, _in: Arc<u8>| async move { Ok(()) });
    template
        .step("NeedsRequest")
        .needs(from_root_request)
        .run(|_cx, _r: Arc<u32>| async move { Ok(()) });
    template
        .step("NeedsToken")
        .needs(from_root_token)
        .run(|_cx, _t: Arc<u32>| async move { Ok(()) });

    let template = template
        .build(
            Policy::Isolate,
            Shutdown::within(std::time::Duration::from_secs(2)),
            Mode::Resident,
        )
        .expect("valid template");
    let _ = root.template("Template", &template);

    let root = root
        .build(
            Policy::FailFast,
            Shutdown::within(std::time::Duration::from_secs(10)),
            Mode::Resident,
        )
        .expect("valid root");

    let v = root.inspect();
    assert_no_missing_targets(&v);

    let needs_request = v
        .node("Template/NeedsRequest")
        .expect("Template/NeedsRequest");
    assert!(needs_request.needs.is_empty());
    assert!(v
        .why("Template/NeedsRequest")
        .expect("why")
        .waits_on
        .is_empty());

    let needs_input = v
        .node("Template/NeedsTemplateInput")
        .expect("Template/NeedsTemplateInput");
    assert_eq!(needs_input.needs, vec![NodePath::root("Template")]);
    assert_eq!(
        v.why("Template/NeedsTemplateInput").expect("why").waits_on,
        vec![(NodePath::root("Template"), Reason::DeclaredNeed)]
    );

    let needs_token = v.node("Template/NeedsToken").expect("Template/NeedsToken");
    assert_eq!(needs_token.needs, vec![NodePath::root("Token")]);
    assert_eq!(
        v.why("Template/NeedsToken").expect("why").waits_on,
        vec![(NodePath::root("Token"), Reason::Import)]
    );

    let order = v.release_order();
    assert!(order.before("Template/NeedsTemplateInput", "Template"));
    assert!(order.before("Template/NeedsRequest", "Template"));
    assert!(order.before("Template/NeedsToken", "Template"));

    // The template declaration does not block execution; it is still a valid plan
    // when supplied input is present.
    let trace = root
        .simulate_with_input(99u32, &Script::new().at(1.0, Request::Shutdown))
        .expect("valid simulated run");
    assert!(trace
        .events
        .iter()
        .any(|e| e.node.as_ref() == Some(&NodePath::root("Token"))));
}

#[test]
fn unrelated_input_import_is_not_silently_hidden_or_admitted() {
    use sdax::host::engine::{EngineError, Machine};
    let external = Plan::with_input::<u32>("external");
    let mut root = Plan::builder("root");
    let imported = root.import(external.input());
    root.step("Read")
        .needs(imported)
        .run(|_, _: Arc<u32>| async { Ok(()) });
    let root = root
        .build(Policy::FailFast, Shutdown::unbounded(), Mode::Finite)
        .unwrap();
    let view = root.inspect();
    assert_eq!(
        view.edges.len(),
        1,
        "unresolved import must retain its diagnostic edge"
    );
    assert_eq!(view.why("Read").unwrap().waits_on.len(), 1);
    assert!(matches!(
        Machine::new(&root),
        Err(EngineError::UnresolvedImports(_))
    ));
}

#[test]
fn template_inside_component_preserves_own_input_boundary_through_child_imports() {
    let mut root = Plan::with_input::<u32>("root");
    let input = root.input();
    let mut middle = Plan::builder("middle");
    let from_root = middle.import(input);
    let mut factory = Plan::with_input::<u8>("factory");
    let own = factory.input();
    let imported = factory.import(from_root);
    let mut child = Plan::builder("child");
    let from_root = child.import(imported);
    let from_factory = child.import(own);
    child
        .step("Read")
        .needs((from_root, from_factory))
        .run(|_, _: (Arc<u32>, Arc<u8>)| async { Ok(()) });
    let child = child
        .build(Policy::FailFast, Shutdown::unbounded(), Mode::Finite)
        .unwrap();
    factory.component("Child", &child, ());
    let factory = factory
        .build(Policy::FailFast, Shutdown::unbounded(), Mode::Finite)
        .unwrap();
    middle.template("Factory", &factory);
    let middle = middle
        .build(Policy::FailFast, Shutdown::unbounded(), Mode::Resident)
        .unwrap();
    root.component("Middle", &middle, ());
    let root = root
        .build(Policy::FailFast, Shutdown::unbounded(), Mode::Finite)
        .unwrap();
    let view = root.inspect();
    assert_no_missing_targets(&view);
    let boundary = NodePath::root("Middle").child("Factory");
    assert_eq!(
        view.node("Middle/Factory/Child/Read").unwrap().needs,
        vec![boundary.clone()]
    );
    assert_eq!(
        view.why("Middle/Factory/Child/Read").unwrap().waits_on,
        vec![(boundary, Reason::Import)]
    );
}
