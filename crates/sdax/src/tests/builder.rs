//! What the builder records (`W-14` A1/A4/A5 of `B/experiments/typed_keys`,
//! restated against this crate).

use super::corpus::*;
use crate::plan::{Kind, ReleaseStyle};
use crate::*;
use std::sync::Arc;

fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn i01_is_recorded_exactly_as_written() {
    let plan = i01().expect("valid");
    assert_send_sync::<Plan>();
    let ir = plan.ir();
    assert_eq!(ir.name, "Startup");
    assert_eq!(ir.semantics, "sdax/1");
    let names: Vec<&str> = ir.nodes.iter().map(|n| n.name.as_str()).collect();
    assert_eq!(
        names,
        ["Transport", "PeerStore", "RoutingTable", "Registration"]
    );
    let kinds: Vec<Kind> = ir.nodes.iter().map(|n| n.kind).collect();
    assert_eq!(
        kinds,
        [Kind::Resource, Kind::Resource, Kind::Resource, Kind::Effect]
    );
    assert_eq!(ir.nodes[0].needs, vec![]);
    assert_eq!(ir.nodes[1].needs, vec![ir.nodes[0].key]);
    assert_eq!(ir.nodes[2].needs, vec![ir.nodes[0].key]);
    assert_eq!(ir.nodes[3].needs, vec![ir.nodes[1].key, ir.nodes[2].key]);
    assert_eq!(ir.nodes[3].attrs.on_ambiguous, Some(Ambiguity::Report));
    assert_eq!(ir.policy, Policy::FailFast);
    assert_eq!(ir.mode, Mode::Finite);
    assert_eq!(ir.shutdown, Shutdown::within(secs(10)));
}

#[test]
fn release_by_drop_is_recorded_as_drop_not_as_a_body() {
    let plan = i01().expect("valid");
    let ir = plan.ir();
    assert_eq!(
        ir.nodes[0].attrs.release,
        ReleaseStyle::Async,
        "Transport has a release body"
    );
    assert_eq!(
        ir.nodes[2].attrs.release,
        ReleaseStyle::Drop,
        "RoutingTable is release::by_drop()"
    );
    assert_eq!(ir.nodes[3].attrs.release, ReleaseStyle::Compensate);
}

#[test]
fn an_effect_may_be_persistent_instead_of_compensated() {
    let plan = i15_with(I15Opts {
        persistent: true,
        ..I15Opts::default()
    })
    .expect("valid");
    assert_eq!(plan.ir().nodes[1].attrs.release, ReleaseStyle::Persistent);
}

#[test]
fn attribute_order_is_free_and_the_recorded_declaration_is_identical() {
    fn build(within_first: bool) -> Plan {
        let mut p = Plan::builder("Order");
        let t = p
            .resource("Transport")
            .acquire(|cx, ()| async move { Ok(cx.hold_value(Transport { addr: "t" })) })
            .release(|_cx, _t| async move { Ok(()) });
        let node = p.resource("PeerStore");
        let node = if within_first {
            node.within(secs(3)).needs(t)
        } else {
            node.needs(t).within(secs(3))
        };
        node.acquire(|cx, _t: Arc<Transport>| async move { Ok(cx.hold_value(PeerStore)) })
            .release(|_cx, _s| async move { Ok(()) });
        p.build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Finite)
            .expect("valid")
    }
    // Two builders get two plan identities, so compare with the identity elided.
    fn normalised(p: &Plan) -> String {
        format!("{:?}", p.ir())
            .replace(&format!("plan: {}", p.ir().id), "plan: N")
            .replace(&format!("id: {},", p.ir().id), "id: N,")
    }
    assert_eq!(normalised(&build(true)), normalised(&build(false)));
}

#[test]
fn a_component_carries_its_child_plan_and_a_template_records_its_imports() {
    let composed = i32(Shutdown::within(secs(5))).expect("valid");
    let net = &composed.ir().nodes[0];
    assert_eq!(net.kind, Kind::Component);
    assert_eq!(net.child.as_ref().expect("inner plan").name, "Networking");

    let mesh = i30().expect("valid");
    let tpl = mesh
        .ir()
        .nodes
        .iter()
        .find(|n| n.kind == Kind::Template)
        .expect("template node");
    assert_eq!(tpl.name, "Link");
    let inner = tpl.child.as_ref().expect("inner plan");
    // The child's `import(endpoint)` is a node of the child naming a parent key.
    let import = inner
        .nodes
        .iter()
        .find(|n| n.kind == Kind::Import)
        .expect("import node");
    assert_eq!(
        import.source,
        Some(mesh.ir().nodes[0].key),
        "imports the mesh Endpoint"
    );
    assert!(
        inner.nodes.iter().any(|n| n.kind == Kind::Input),
        "template has an input node"
    );
    let accept = mesh
        .ir()
        .nodes
        .iter()
        .find(|n| n.name == "AcceptLoop")
        .expect("accept loop");
    assert_eq!(
        accept.spawns,
        vec![tpl.key],
        "spawns is recorded on the service"
    );
}

#[test]
fn a_step_and_a_try_step_differ_in_their_recorded_kind() {
    let plan = i23(true).expect("valid");
    let kinds: Vec<Kind> = plan.ir().nodes.iter().map(|n| n.kind).collect();
    assert_eq!(kinds[1], Kind::TryStep);
    assert_eq!(kinds[6], Kind::Step);
}

#[test]
fn a_blocking_step_records_its_required_pool() {
    let plan = i29(2, 0).expect("valid");
    let verify = plan
        .ir()
        .nodes
        .iter()
        .find(|n| n.name == "Verify")
        .expect("Verify");
    assert_eq!(verify.kind, Kind::BlockingStep);
    let pool = verify.attrs.pool.expect("pool required by the typestate");
    assert_eq!(plan.ir().pools[pool.idx as usize].name, "cpu");
    assert_eq!(plan.ir().pools[pool.idx as usize].limit, 2);
}

#[test]
fn locks_and_exports_are_recorded() {
    let plan = i27(true).expect("valid");
    let miga = plan
        .ir()
        .nodes
        .iter()
        .find(|n| n.name == "MigA")
        .expect("MigA");
    assert_eq!(miga.attrs.exclusive, vec![plan.ir().nodes[0].key]);

    let net = networking().expect("valid");
    assert_eq!(net.ir().export, Some(net.ir().nodes[3].key));
}
