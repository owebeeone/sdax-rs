use sdax::host::sim::Simulator;
use sdax::{At, Body, Cleanup, Request, Script, Serve, SpawnSpec};
use sdax::{Mode, Plan, Policy, Shutdown};
use std::sync::Arc;
use std::time::Duration;

struct Endpoint;
struct Conn;

fn plan() -> Plan {
    let mut p = Plan::builder("Churn");
    let endpoint = p
        .resource("Endpoint")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Endpoint)) })
        .release(|_cx, _e| async move { Ok(()) });
    let mut link_builder = Plan::with_input::<u8>("Link");
    let imported = link_builder.import(endpoint);
    link_builder
        .resource("Sock")
        .needs(imported)
        .acquire(|cx, _e: Arc<Endpoint>| async move { Ok(cx.hold_value(Conn)) })
        .release(|_cx, _c| async move { Ok(()) });
    let link = link_builder
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(5)),
            Mode::Resident,
        )
        .expect("template is valid");
    let template = p.template("Link", &link);
    p.service("Acceptor")
        .needs(endpoint)
        .spawns(&template)
        .initialize(|_cx, _e: Arc<Endpoint>| async move { Ok(()) })
        .serve(|_cx, _handle| async move { Ok(()) });
    p.build(
        Policy::FailFast,
        Shutdown::within(Duration::from_secs(30_000)),
        Mode::Resident,
    )
    .expect("diagnostic plan is valid")
}

#[test]
fn active_instance_states_exclude_history_without_removing_it() {
    let script = Script::new()
        .prepare("Endpoint", Body::ok(At::plus(0.0)))
        .cleanup("Endpoint", Cleanup::Ok(Duration::ZERO))
        .prepare("Link/Sock", Body::ok(At::plus(0.0)))
        .cleanup("Link/Sock", Cleanup::Ok(Duration::ZERO))
        .prepare("Acceptor", Body::ok(At::plus(0.0)))
        .spawns(
            "Acceptor",
            [
                SpawnSpec::new("Link", At::plus(0.0)).stopped(At::tick(1.0)),
                SpawnSpec::new("Link", At::plus(0.0)).stopped(At::tick(3.0)),
            ],
        )
        .serve("Acceptor", [Serve::StopsAfter(Duration::ZERO)])
        .at(5.0, Request::Shutdown);
    let mut sim = Simulator::new(&plan(), &script).unwrap();
    let mut mixed = false;
    while sim.step().is_some() {
        let all = sim.machine().instances();
        let expected: Vec<_> = all
            .iter()
            .copied()
            .filter(|(_, state)| *state != sdax::host::engine::RunState::Ended)
            .collect();
        let active: Vec<_> = sim.machine().active_instance_states().collect();
        assert_eq!(active, expected);
        mixed |= !active.is_empty() && active.len() < all.len();
    }
    assert!(
        mixed,
        "fixture must observe a live sibling alongside ended history"
    );
    assert_eq!(sim.machine().instances().len(), 2);
    assert_eq!(sim.machine().active_instance_states().count(), 0);
    assert_eq!(sim.machine().execution_node_count(), 3);
    assert!(sim.ended());
    assert!(sim.rejections().is_empty());
    let report = sim.take_report::<()>().unwrap();
    assert_eq!(report.outcome, sdax::Outcome::Ok);
    assert!(report.is_clean());
}
