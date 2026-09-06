//! Suite (c), a root plan's per-run input: `C-70`, `C-71`.
//!
//! `I-04` is the headline use case — build the per-request orchestration once,
//! run it for thousands of concurrent requests, each with its own typed state.
//! The input node is the same node a template declares; what changed is that a
//! **root** run may declare one too, and the driver seeds its slot before any
//! body is built. A node that `needs` it is therefore satisfied from the first
//! step, exactly as an instance's is from the moment of the spawn.
//!
//! `C-71` is the other half: a plan that declares an input and is offered to a
//! driver that supplies **no** value is still refused, with the error it has
//! always been refused with.

use crate::Drv as ScriptedDriver;
use sdax::host::engine::EngineError;
use sdax::host::sim::ScriptError;
use sdax::*;
use sdax_testkit::eol::*;
use std::sync::Arc;
use std::time::Duration;

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

/// A root plan taking a per-run `u8`: `Cfg` needs it, `Work` needs `Cfg`.
fn per_run() -> Plan<(), u8> {
    let mut p = Plan::with_input::<u8>("Request");
    let input = p.input();
    let cfg = p
        .step("Cfg")
        .needs(input)
        .run(|_cx, _v: Arc<u8>| async move { Ok(()) });
    p.step("Work")
        .needs(cfg)
        .run(|_cx, _c: Arc<()>| async move { Ok(()) });
    p.build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Finite)
        .expect("a valid root plan with an input")
}

/// `C-70` — a root input is a satisfied need from the first step, and the
/// input node is not a node of the run.
#[test]
fn c70_a_root_input_satisfies_the_node_that_needs_it() {
    let plan = per_run();
    let d = ScriptedDriver::run_with_input(&plan, 7u8, &Script::new()).expect("runs");
    d.check();
    let t = d.eol();
    assert!(
        t.pos(is_start, "Cfg") < t.pos(is_start, "Work"),
        "Cfg needs only the input, so it starts before Work"
    );
    assert_eq!(d.report.outcome, Outcome::Ok);

    // The input is not a node of the run: `inspect()` does not show it, and
    // nothing in the trace names it.
    let view = plan.inspect();
    assert!(
        view.nodes.iter().all(|n| n.path.to_string() != "input"),
        "the input node is not shown: {view}"
    );
    assert!(
        d.trace
            .events
            .iter()
            .all(|e| e.node.as_ref().map(|p| p.to_string()).as_deref() != Some("input")),
        "no event names the input node"
    );
    // And the node that needs it waits for nothing at all.
    assert!(
        view.why("Cfg").expect("Cfg").waits_on.is_empty(),
        "the input is satisfied before the run begins"
    );
}

/// `C-71` — a plan that declares an input, offered to a driver that supplies
/// no value, is refused before any effect, naming the input node.
///
/// Named driver rather than `Drv`: `start(rt, input)` cannot *reach* this case
/// — the type system makes a root run supply its input — so the only entry
/// point that can offer a plan with nothing for its input is one that takes no
/// input at all. In the scripted binary this is `Drv`; in the adapter's binary
/// it is the same row run against the machine both drivers refuse through.
#[test]
fn c71_a_plan_with_an_unsatisfied_input_is_refused() {
    let plan = per_run();
    match sdax_testkit::ScriptedDriver::run(&plan, &Script::new()) {
        Err(ScriptError::Engine(EngineError::TemplateAsScope(p))) => {
            assert_eq!(p.to_string(), "input");
        }
        Err(e) => panic!("refused, but not as an unsatisfied input: {e:?}"),
        Ok(_) => panic!("a plan whose input has no value must not run"),
    }
}
