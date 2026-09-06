//! Suite (b) instance tests: the machine's side of templates and dynamic
//! instances (Stage 3). These drive [`Machine::step`] by hand, one event at a
//! time, so the instance table, the spawn gate and the instance's own release
//! graph are exercised without a driver.

use crate::host::engine::{Effect, EngineError, Event, Machine, NodeState, RunState};
use crate::host::{InstanceId, RawKey};
use crate::*;
use std::sync::Arc;
use std::time::Duration;

struct Endpoint;
struct Conn;

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

/// The `HC` shape of `C-30`: an endpoint resource, a template that imports it,
/// and an acceptor service that may instantiate the template per connection.
pub(crate) fn mesh() -> Plan {
    let mut root = Plan::builder("Mesh");
    let endpoint = root
        .resource("Endpoint")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Endpoint)) })
        .release(|_cx, _e| async move { Ok(()) });

    let mut t = Plan::template::<u8>("Link");
    let imported = t.import(endpoint);
    t.resource("Sock")
        .needs(imported)
        .acquire(|cx, _e: Arc<Endpoint>| async move { Ok(cx.hold_value(Conn)) })
        .release(|_cx, _c| async move { Ok(()) });
    let child = t
        .build(Policy::FailFast, Shutdown::within(secs(2)), Mode::Resident)
        .expect("valid template");

    let link = root.template("Link", &child);
    root.service("Acceptor")
        .needs(endpoint)
        .spawns(&link)
        .stop_within(secs(1))
        .start(|_cx, _e: Arc<Endpoint>| async move { Ok(Serving::new((), async { Ok(()) })) });
    root.build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Resident)
        .expect("valid plan")
}

fn key(m: &Machine, path: &str) -> RawKey {
    m.key_of(path).unwrap_or_else(|| panic!("no node {path}"))
}

/// `Machine::new` no longer refuses a plan that declares a template: Stage 3
/// runs them, so the refusal that named the stage is gone.
#[test]
fn a_plan_with_a_template_is_accepted() {
    let plan = mesh();
    let m = Machine::new(&plan).expect("the machine runs templates from Stage 3");
    assert_eq!(m.run_state(), RunState::Planned);
}

/// A plan that declares an input runs as a root when the caller supplies the
/// value, and is refused when nobody does. Both answers come from the same
/// plan value: what decides is the entry point, not the declaration.
#[test]
fn an_input_bearing_plan_runs_only_when_its_value_is_supplied() {
    let mut p = Plan::with_input::<u8>("Request");
    let input = p.input();
    p.step("Cfg")
        .needs(input)
        .run(|_cx, _v: Arc<u8>| async move { Ok(()) });
    let plan = p
        .build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Finite)
        .expect("valid");

    let err = Machine::new(&plan).expect_err("no value, no run");
    match &err {
        EngineError::TemplateAsScope(node) => assert_eq!(*node, NodePath::root("input")),
        other => panic!("refused, but not as an unsatisfied input: {other}"),
    }
    let m = Machine::with_input(&plan).expect("the caller supplies the input");
    assert_eq!(m.run_state(), RunState::Planned);
    // The input is not a node of the run: `Cfg` is, and its need on the input
    // is satisfied before the first step.
    assert!(m.key_of("input").is_none());
    assert!(m.key_of("Cfg").is_some());

    // The counterfactual splits the same way, so § 9's pre-ship answer is
    // available for exactly the plans that can be run.
    assert!(plan.simulate(&Script::new()).is_err());
    let trace = plan
        .simulate_with_input(1u8, &Script::new())
        .expect("the counterfactual for start(rt, input)");
    assert!(trace
        .events
        .iter()
        .any(|e| e.node.as_ref() == Some(&NodePath::root("Cfg"))));
    assert!(trace
        .events
        .iter()
        .all(|e| e.node.as_ref() != Some(&NodePath::root("input"))));
}

/// The template node comes up `Live` — it has no body and never becomes
/// `Ready` (contract § 1) — and the scope reaches steady state with it.
#[test]
fn a_template_node_becomes_live_and_does_not_block_steady_state() {
    let plan = mesh();
    let mut m = Machine::new(&plan).expect("runs");
    m.begin();
    let endpoint = key(&m, "Endpoint");
    m.step(Event::Started(endpoint));
    m.step(Event::Held(endpoint));
    m.step(Event::NodeOk(endpoint));
    assert!(matches!(m.state_of("Link"), Some(NodeState::Live)));
    let acceptor = key(&m, "Acceptor");
    m.step(Event::Started(acceptor));
    m.step(Event::NodeOk(acceptor));
    assert_eq!(m.run_state(), RunState::Steady);
}

/// A spawn creates the instance's nodes, tells the host to open its slots and
/// admits them: `Effect::SpawnInstance` is live.
#[test]
fn a_spawn_opens_an_instance_and_admits_its_nodes() {
    let plan = mesh();
    let mut m = Machine::new(&plan).expect("runs");
    m.begin();
    let endpoint = key(&m, "Endpoint");
    m.step(Event::Started(endpoint));
    m.step(Event::Held(endpoint));
    m.step(Event::NodeOk(endpoint));
    let acceptor = key(&m, "Acceptor");
    let link = key(&m, "Link");
    m.step(Event::Started(acceptor));
    let id = InstanceId(1);
    let fx = m.step(Event::InstanceSpawned {
        spawner: acceptor,
        template: link,
        id,
    });
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::SpawnInstance { id: i, .. } if *i == id)),
        "expected a SpawnInstance effect, got {fx:?}"
    );
    let nodes = m.instance_nodes(id);
    assert_eq!(nodes.len(), 1, "the instance has one node: {nodes:?}");
    assert_eq!(nodes[0].2.to_string(), "Link/Sock");
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Spawn { node, .. } if *node == nodes[0].0)),
        "the instance's resource was admitted: {fx:?}"
    );
}

/// `V-SPAWN-*` at run time: the three refusals `cx.spawn` can answer with,
/// decided by the machine so both drivers answer alike.
#[test]
fn the_spawn_gate_refuses_a_foreign_or_undeclared_template() {
    let plan = mesh();
    let mut m = Machine::new(&plan).expect("runs");
    m.begin();
    let endpoint = key(&m, "Endpoint");
    let acceptor = key(&m, "Acceptor");
    let link = key(&m, "Link");
    let foreign = RawKey {
        plan: u64::MAX,
        idx: 0,
    };
    assert_eq!(
        m.spawn_check(acceptor, foreign),
        Err(SpawnError::ForeignTemplate)
    );
    assert_eq!(
        m.spawn_check(endpoint, link),
        Err(SpawnError::UndeclaredTemplate)
    );
    assert_eq!(m.spawn_check(acceptor, link), Ok(()));
    m.step(Event::ShutdownRequested);
    assert_eq!(
        m.spawn_check(acceptor, link),
        Err(SpawnError::ScopeStopping)
    );
}

/// Drive the mesh to steady state and return the machine with its three keys.
fn steady() -> (Machine, RawKey, RawKey, RawKey) {
    let plan = mesh();
    let mut m = Machine::new(&plan).expect("runs");
    m.begin();
    let endpoint = key(&m, "Endpoint");
    let acceptor = key(&m, "Acceptor");
    let link = key(&m, "Link");
    m.step(Event::Started(endpoint));
    m.step(Event::Held(endpoint));
    m.step(Event::NodeOk(endpoint));
    m.step(Event::Started(acceptor));
    m.step(Event::NodeOk(acceptor));
    (m, endpoint, acceptor, link)
}

/// Spawn one instance and drive its resource to `Ready`.
fn spawn(m: &mut Machine, acceptor: RawKey, link: RawKey, id: InstanceId) -> RawKey {
    m.step(Event::InstanceSpawned {
        spawner: acceptor,
        template: link,
        id,
    });
    let sock = m.instance_nodes(id)[0].0;
    m.step(Event::Started(sock));
    m.step(Event::Held(sock));
    m.step(Event::NodeOk(sock));
    sock
}

/// INV-16's release clause and `C-14`'s instance half: the endpoint the
/// template imports is not released while an instance of it is live, and the
/// instance's own release runs first.
#[test]
fn a_live_instance_holds_the_key_it_imports_until_it_ends() {
    let (mut m, endpoint, acceptor, link) = steady();
    let sock = spawn(&mut m, acceptor, link, InstanceId(1));
    assert_eq!(m.state(sock), Some(NodeState::Ready));

    let fx = m.step(Event::ShutdownRequested);
    // The endpoint's own release cannot be among the first effects: the
    // instance is live and holds the gate shut.
    assert!(
        !fx.iter()
            .any(|e| matches!(e, Effect::Release(k) if *k == endpoint)),
        "the endpoint released with a live instance: {fx:?}"
    );
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Release(k) if *k == sock)),
        "the instance's own release runs first: {fx:?}"
    );
    let fx = m.step(Event::NodeOk(sock));
    assert!(
        !fx.iter()
            .any(|e| matches!(e, Effect::Release(k) if *k == endpoint)),
        "the acceptor still holds the endpoint: {fx:?}"
    );
    assert!(matches!(m.state(link), Some(NodeState::Stopped)));
    // The acceptor is the endpoint's other dependent; once it has stopped too,
    // the gate opens.
    let fx = m.step(Event::ServeEnded {
        node: acceptor,
        fault: None,
    });
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Release(k) if *k == endpoint)),
        "the endpoint releases once every dependent has: {fx:?}"
    );
    m.step(Event::NodeOk(endpoint));
    assert!(m.ended(), "{:?}", m.run_state());
    let report = m.take_report().expect("a report");
    assert!(report.is_clean(), "{report}");
}

/// `C-30`'s shape: two instances, one closed while the run keeps going, the
/// other stopped by the shutdown.
#[test]
fn one_instance_can_be_closed_while_the_other_keeps_running() {
    let (mut m, _endpoint, acceptor, link) = steady();
    let a = spawn(&mut m, acceptor, link, InstanceId(1));
    let b = spawn(&mut m, acceptor, link, InstanceId(2));
    assert_ne!(a, b, "two instances have distinct node keys");

    let fx = m.step(Event::StopInstance(InstanceId(1)));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Release(k) if *k == a)),
        "{fx:?}"
    );
    m.step(Event::NodeOk(a));
    assert_eq!(
        m.instances(),
        vec![
            (InstanceId(1), RunState::Ended),
            (InstanceId(2), RunState::Steady)
        ]
    );
    assert_eq!(m.state(b), Some(NodeState::Ready));
    assert_eq!(m.run_state(), RunState::Steady);

    m.step(Event::ShutdownRequested);
    m.step(Event::NodeOk(b));
    m.step(Event::ServeEnded {
        node: acceptor,
        fault: None,
    });
    m.step(Event::NodeOk(_endpoint));
    assert!(m.ended());
    let report = m.take_report().expect("a report");
    assert!(report.is_clean(), "{report}");
}

/// The instance's records are ordered after the template's own and by id
/// (INV-20, F4).
#[test]
fn instance_records_carry_the_instance_in_their_order() {
    let (mut m, _e, acceptor, link) = steady();
    m.step(Event::InstanceSpawned {
        spawner: acceptor,
        template: link,
        id: InstanceId(2),
    });
    m.step(Event::InstanceSpawned {
        spawner: acceptor,
        template: link,
        id: InstanceId(1),
    });
    let a = m.instance_nodes(InstanceId(2))[0].0;
    let b = m.instance_nodes(InstanceId(1))[0].0;
    m.step(Event::Started(a));
    m.step(Event::NodeErr(a, crate::FaultKind::Timeout));
    m.step(Event::Started(b));
    m.step(Event::NodeErr(b, crate::FaultKind::Timeout));
    m.step(Event::CancelRequested);
    m.step(Event::ServeEnded {
        node: acceptor,
        fault: None,
    });
    m.step(Event::NodeOk(_e));
    assert!(m.ended(), "{:?}", m.run_state());
    let report = m.take_report().expect("a report");
    let orders: Vec<Vec<(u32, Option<InstanceId>)>> = report
        .faults
        .iter()
        .map(|f| f.order.steps.clone())
        .collect();
    assert_eq!(
        orders,
        vec![
            vec![(1, Some(InstanceId(1))), (0, None)],
            vec![(1, Some(InstanceId(2))), (0, None)],
        ],
        "instances sort by id, under the template's own step"
    );
}

/// The spawn/settle race, which only a real driver can lose: a body's
/// `cx.spawn` passed the gate and the scope settled before the event reached
/// the machine. The instance is created and ended at once, so the `Child` the
/// body is holding answers instead of hanging — and the run still ends.
///
/// Found by the adapter's Monte Carlo walk (`SDAX_MC_SEED=6746427589533237250`,
/// index 281): the machine left the instance's scope `Planned` for ever and
/// the run never reached `End`.
#[test]
fn a_spawn_that_lands_after_the_settle_ends_at_once() {
    let (mut m, endpoint, acceptor, link) = steady();
    m.step(Event::ShutdownRequested);
    assert_eq!(
        m.spawn_check(acceptor, link),
        Err(SpawnError::ScopeStopping),
        "the gate is shut by now"
    );
    // The body checked the gate before the settle and is holding a `Child`.
    let id = InstanceId(7);
    m.step(Event::InstanceSpawned {
        spawner: acceptor,
        template: link,
        id,
    });
    assert_eq!(
        m.instances(),
        vec![(id, RunState::Ended)],
        "an instance that arrives too late ends at once"
    );
    m.step(Event::ServeEnded {
        node: acceptor,
        fault: None,
    });
    m.step(Event::NodeOk(endpoint));
    assert!(m.ended(), "the run still ends: {:?}", m.run_state());
    let report = m.take_report().expect("a report");
    assert!(report.is_clean(), "{report}");
}
