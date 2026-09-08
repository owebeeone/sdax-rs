//! Suite (b), the inspection rows: `CanonicalTests.md` § 3 P-03, P-04, P-13,
//! P-14, plus `Plan::effects()` (adoption A3).

use super::corpus::*;
use crate::view::Reason;
use crate::*;

fn names(v: &[NodePath]) -> Vec<String> {
    v.iter().map(|p| p.to_string()).collect()
}

/// P-03 — I-33: nodes, edges, layers, release order, `why`, mode, rendering.
#[test]
fn p03_the_view_of_i33_is_the_declaration_and_nothing_else() {
    let plan = i01().expect("valid");
    let v = plan.inspect();

    assert_eq!(
        names(&v.nodes.iter().map(|n| n.path.clone()).collect::<Vec<_>>()),
        ["Transport", "PeerStore", "RoutingTable", "Registration"]
    );
    let edges: Vec<(String, String)> = v
        .edges
        .iter()
        .map(|e| (e.from.to_string(), e.to.to_string()))
        .collect();
    assert_eq!(
        edges,
        [
            ("PeerStore".to_string(), "Transport".to_string()),
            ("RoutingTable".to_string(), "Transport".to_string()),
            ("Registration".to_string(), "PeerStore".to_string()),
            ("Registration".to_string(), "RoutingTable".to_string()),
        ],
        "exactly the declared edges, and no other"
    );

    let layers: Vec<Vec<String>> = v.layers().iter().map(|l| names(l)).collect();
    assert_eq!(
        layers,
        [
            vec!["Transport"],
            vec!["PeerStore", "RoutingTable"],
            vec!["Registration"]
        ]
    );

    let order = v.release_order();
    assert!(order.before("Registration", "PeerStore"));
    assert!(order.before("Registration", "RoutingTable"));
    assert!(order.before("PeerStore", "Transport"));
    assert!(order.unordered("PeerStore", "RoutingTable"));
    assert!(!order.unordered("Registration", "Transport"));

    let why = v.why("Registration").expect("a declared node");
    assert_eq!(
        why.waits_on
            .iter()
            .map(|(n, r)| (n.to_string(), *r))
            .collect::<Vec<_>>(),
        [
            ("PeerStore".to_string(), Reason::DeclaredNeed),
            ("RoutingTable".to_string(), Reason::DeclaredNeed)
        ]
    );

    assert_eq!(v.mode, Mode::Finite);
    let text = v.to_string();
    for line in [
        "plan Startup",
        "engine-semantics sdax/2",
        "mode: finite",
        "policy: fail-fast",
        "shutdown: within 10s",
        "Registration",
        "ambiguous: report",
        "release graph",
    ] {
        assert!(
            text.contains(line),
            "rendering is missing {line:?}:\n{text}"
        );
    }
}

/// P-04 — release order for I-16, I-32 (nested, addressable) and I-30
/// (instances placed before every key they import).
#[test]
fn p04_release_order_for_flat_nested_and_dynamic_plans() {
    let order = i16().expect("valid").inspect().release_order();
    assert!(order.unordered("PeerStore", "RoutingTable"));
    assert!(order.before("PeerStore", "Transport"));
    assert!(order.before("RoutingTable", "Transport"));

    let nested = i32(Shutdown::within(secs(5))).expect("valid").inspect();
    let paths = names(
        &nested
            .nodes
            .iter()
            .map(|n| n.path.clone())
            .collect::<Vec<_>>(),
    );
    assert!(
        paths.contains(&"Net/Transport".to_string()),
        "inner nodes are addressable: {paths:?}"
    );
    let order = nested.release_order();
    assert!(order.before("Registration", "Net"));
    assert!(order.before("Net/PeerStore", "Net/Transport"));
    assert!(order.unordered("Net/PeerStore", "Net/RoutingTable"));

    let mesh = i30().expect("valid").inspect();
    let order = mesh.release_order();
    assert!(order.before("Link/Dispatcher", "Link/LinkEntry"));
    assert!(order.unordered("Link/Dispatcher", "Link/UnlinkOnClose"));
    assert!(
        order.before("Link", "Endpoint"),
        "instances end before an imported key is released"
    );
    assert!(order.before("AcceptLoop", "Endpoint"));
    assert!(order.unordered("AcceptLoop", "Link"));
}

/// F1 — a service's `spawns` declaration is visible in the view.
#[test]
fn spawns_is_visible_in_the_view() {
    let mesh = i30().expect("valid").inspect();
    let accept = mesh.node("AcceptLoop").expect("AcceptLoop");
    assert_eq!(names(&accept.spawns), ["Link"]);
    assert!(mesh.to_string().contains("spawns Link"), "{}", mesh);
}

/// F1 — `spawns` is a service's declaration, so the view shows it only there.
///
/// White box: a plan whose resource carries a `spawns` cannot be built at all
/// (`V-SPAWN-KIND` rejects it), so the view is checked over a hand-made IR.
#[test]
fn spawns_is_shown_only_on_a_service() {
    let mesh = i30().expect("valid");
    let mut ir = (*mesh.ir).clone();
    let tpl_key = ir
        .nodes
        .iter()
        .find(|n| n.kind == Kind::Template)
        .expect("template")
        .key;
    assert_eq!(ir.nodes[1].kind, Kind::Resource, "node 0 is the endpoint");
    ir.nodes[1].spawns.push(tpl_key);
    let v = crate::view::PlanView::of(&ir);
    assert!(
        v.node("Endpoint").expect("Endpoint").spawns.is_empty(),
        "a resource never shows spawns"
    );
    assert_eq!(
        names(&v.node("AcceptLoop").expect("AcceptLoop").spawns),
        ["Link"]
    );
    assert_eq!(
        v.to_string().matches("spawns Link").count(),
        1,
        "only the service's line carries it:\n{v}"
    );
}

/// A3 — `effects()` lists what is at or past the ship boundary, in declaration
/// order, with the earliest layer that contains one.
#[test]
fn effects_lists_the_ship_boundary_nodes_and_the_first_layer_that_has_one() {
    let plan = i01().expect("valid");
    let e = plan.effects();
    assert_eq!(
        names(&e.nodes),
        ["Transport", "PeerStore", "RoutingTable", "Registration"]
    );
    assert_eq!(e.first_layer, Some(0), "Transport acquires in layer 0");

    // A persistent effect is still an effect and is still listed.
    let persistent = i15_with(I15Opts {
        persistent: true,
        ..I15Opts::default()
    })
    .expect("valid");
    assert_eq!(
        names(&persistent.effects().nodes),
        ["Transport", "Registration"]
    );
    assert!(
        persistent
            .inspect()
            .to_string()
            .contains("effect (persistent)"),
        "{}",
        persistent.inspect()
    );

    // A plan of steps ships nothing.
    let mut p = Plan::builder("Pure");
    p.step("Compute").run(|_cx, ()| async move { Ok(()) });
    let pure = p
        .build(Policy::FailFast, Shutdown::within(secs(1)), Mode::Finite)
        .expect("valid");
    assert!(pure.effects().nodes.is_empty());
    assert_eq!(pure.effects().first_layer, None);
}

/// P-13 — a diff between two identical builds is empty; a changed resolved
/// value is listed, and a change that came from a default is marked.
#[test]
fn p13_the_diff_lists_resolved_values_and_marks_default_changes() {
    let a = i01().expect("valid");
    let b = i01().expect("valid");
    assert!(
        a.inspect().diff(&b.inspect()).is_empty(),
        "same program, empty diff"
    );

    let cooperative = {
        let mut p = Plan::builder("Startup");
        let t = p
            .resource("Transport")
            .acquire(|cx, ()| async move { Ok(cx.hold_value(Transport { addr: "t" })) })
            .release(|_cx, _t| async move { Ok(()) });
        p.effect("Registration")
            .needs(t)
            .cooperative(secs(1))
            .on_ambiguous(Ambiguity::Report)
            .perform(
                |cx, _t: std::sync::Arc<Transport>| async move { Ok(cx.hold_value(Receipt("r"))) },
            )
            .compensate(|_cx, _r| async move { Ok(()) });
        p.build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Finite)
            .expect("valid")
    };
    let base = i15().expect("valid");
    let d = base.inspect().diff(&cooperative.inspect());
    let change = d
        .changed
        .iter()
        .find(|c| c.node.to_string() == "Registration" && c.attr == "cancel")
        .expect("the cancel mode changed");
    assert_eq!(change.from, "drop");
    assert_eq!(change.to, "cooperative(1s)");
    assert!(
        !change.default_changed,
        "the author wrote it, so it is not a default change"
    );

    // The same source under a simulated later crate version: the default moved.
    let mut ir = (*base.ir).clone();
    ir.nodes[1].attrs.cancel = CancelMode::Cooperative(secs(1));
    let simulated = crate::view::PlanView::of(&ir);
    let d = base.inspect().diff(&simulated);
    let change = d
        .changed
        .iter()
        .find(|c| c.attr == "cancel")
        .expect("cancel changed");
    assert!(
        change.default_changed,
        "neither build set it, so the engine default moved"
    );
    assert!(d.added_nodes.is_empty() && d.removed_nodes.is_empty() && d.added_edges.is_empty());
}

/// P-14 — mode is what the author declared, never derived from structure.
#[test]
fn p14_mode_is_declared_and_printed_never_derived() {
    assert_eq!(i16().expect("valid").inspect().mode, Mode::Finite);
    let mut p = Plan::builder("Resident resources");
    p.resource("Db")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Db)) })
        .release(|_cx, _d| async move { Ok(()) });
    let resident = p
        .build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Resident)
        .expect("valid");
    assert_eq!(resident.inspect().mode, Mode::Resident);
    assert!(resident.inspect().to_string().contains("mode: resident"));
    assert_eq!(i05().expect("valid").inspect().mode, Mode::Resident);
}

/// I-27 — `why` names an exclusivity conflict distinctly from a dependency.
#[test]
fn why_separates_a_lock_conflict_from_a_declared_need() {
    let plan = i27(true).expect("valid");
    let v = plan.inspect();
    let why = v.why("MigB").expect("MigB");
    let rendered: Vec<(String, Reason)> = why
        .waits_on
        .iter()
        .map(|(n, r)| (n.to_string(), *r))
        .collect();
    assert!(rendered.contains(&("Db".to_string(), Reason::DeclaredNeed)));
    assert!(
        rendered
            .iter()
            .any(|(n, r)| n == "MigA" && matches!(r, Reason::Exclusive)),
        "{rendered:?}"
    );
}

/// I-29 — a pool user's `why` names the pool.
#[test]
fn why_names_a_pool_a_node_must_be_granted() {
    let plan = i29(2, 0).expect("valid");
    let v = plan.inspect();
    let why = v.why("Verify").expect("Verify");
    assert!(why
        .waits_on
        .iter()
        .any(|(n, r)| *n == *"cpu" && *r == Reason::Pool));
}

#[test]
#[ignore = "prints the rendering for eyeballing; not an assertion"]
fn show_renderings() {
    println!("{}", i01().expect("valid").inspect());
    println!("{}", i30().expect("valid").inspect());
}
