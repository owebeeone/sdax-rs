//! `R-01` proper: the same plan and script on both drivers, and the traces
//! compared.
//!
//! Every other test in this binary asserts the *expectations* of suite (c)
//! against the adapter. This module asserts the stronger thing: that the
//! adapter and the pure machine produce the **same** trace and the same
//! report, event for event, once genuinely unordered pairs are normalised.
//!
//! What is normalised, and why: the contract's "valid schedules" paragraph
//! promises nothing about the relative order of unordered starts and releases.
//! Two events stamped at the same instant on the engine clock are exactly that
//! pair, so each instant's events are compared as a sorted multiset. Anything
//! else — a different event, a different node, a different time, a different
//! count — fails.

use crate::corpus::*;
use crate::Drv;
use sdax::host::Time;
use sdax::*;
use sdax_testkit::ScriptedDriver;

/// One instant's events, in a canonical order.
fn normalise(t: &Trace) -> Vec<(Time, Vec<String>)> {
    let mut out: Vec<(Time, Vec<String>)> = Vec::new();
    for e in &t.events {
        let line = format!(
            "{} {:?}",
            e.node.as_ref().map(|n| n.to_string()).unwrap_or_default(),
            e.kind
        );
        match out.last_mut() {
            Some((at, group)) if *at == e.at => group.push(line),
            _ => out.push((e.at, vec![line])),
        }
    }
    for (_, group) in &mut out {
        group.sort();
    }
    out
}

fn summary<Out>(r: &Report<Out>) -> String {
    let f = |v: &[Fault]| {
        v.iter()
            .map(|f| format!("{}/{:?}/{:?}", f.node, f.phase, f.kind.label()))
            .collect::<Vec<_>>()
            .join(",")
    };
    let n = |v: &[NodeRecord]| {
        v.iter()
            .map(|r| r.node.to_string())
            .collect::<Vec<_>>()
            .join(",")
    };
    format!(
        "{:?} faults[{}] cleanup[{}] incomplete[{}] ambiguous[{}]",
        r.outcome,
        f(&r.faults),
        f(&r.cleanup_failures),
        n(&r.incomplete),
        n(&r.ambiguous)
    )
}

/// The differential check for one case.
fn same<Out: Send + Sync + 'static>(what: &str, plan: &Plan<Out>, script: &Script) {
    let pure = ScriptedDriver::run(plan, script).expect("runs");
    let real = Drv::run(plan, script).expect("runs");
    pure.check();
    real.check();
    assert_eq!(
        summary(&pure.report),
        summary(&real.report),
        "{what}: the reports differ"
    );
    let (a, b) = (normalise(&pure.trace), normalise(&real.trace));
    if a != b {
        panic!(
            "{what}: the traces differ\npure:\n{}\nadapter:\n{}",
            pure.eol().render(),
            real.eol().render()
        );
    }
}

fn every_body(plan: &Plan, secs: f64) -> Script {
    let mut s = Script::new();
    for n in &plan.inspect().nodes {
        s = s.prepare(&n.path.to_string(), Body::ok(At::plus(secs)));
    }
    s
}

#[test]
fn r01_the_adapter_and_the_machine_agree_on_the_startup_programs() {
    same(
        "i01 finite",
        &i01(Mode::Finite),
        &every_body(&i01(Mode::Finite), 1.0),
    );
    same("i02", &i02(), &every_body(&i02(), 0.5));
    same("i04", &i04(), &Script::new());
    same(
        "i05 shutdown",
        &i05(),
        &every_body(&i05(), 1.0).at(5.0, Request::Shutdown),
    );
    same(
        "i07 restart",
        &i07(true),
        &Script::new()
            .serve(
                "Exporter",
                [Serve::Err(At::tick(2.0), "partition".to_string())],
            )
            .at(9.0, Request::Shutdown),
    );
}

#[test]
fn r01_the_adapter_and_the_machine_agree_on_the_fault_programs() {
    same(
        "i08 fail-fast",
        &i08(Policy::FailFast),
        &Script::new()
            .prepare("B", Body::fail(At::plus(1.0), "boom"))
            .prepare("A", Body::ok(At::plus(3.0)))
            .prepare("C", Body::ok(At::plus(4.0))),
    );
    same(
        "i08 isolate",
        &i08(Policy::Isolate),
        &Script::new()
            .prepare("B", Body::fail(At::plus(1.0), "boom"))
            .prepare("A", Body::ok(At::plus(3.0))),
    );
    same(
        "i09 panic",
        &i09(),
        &Script::new().prepare("Check", Body::panic(At::plus(1.0))),
    );
    same(
        "i12 cancel mid-acquire",
        &i12(),
        &Script::new()
            .prepare("A", Body::ok(At::plus(1.0)))
            .prepare("B", Body::ok(At::plus(4.0)).held(At::plus(3.0)))
            .at(2.0, Request::Cancel),
    );
    same(
        "i24 retry with backoff",
        &i24(std::time::Duration::from_secs(1)),
        &Script::new().body(
            "Conn",
            [
                Body::fail(At::plus(1.0), "1"),
                Body::fail(At::plus(1.0), "2"),
                Body::ok(At::plus(1.0)),
            ],
        ),
    );
}

#[test]
fn r01_the_adapter_and_the_machine_agree_on_cleanup_and_components() {
    same(
        "i15 timeout to ambiguity",
        &i15(
            Mode::Finite,
            Some(std::time::Duration::from_secs(2)),
            Ambiguity::Recover,
        ),
        &Script::new().prepare("Registration", Body::pending()),
    );
    same(
        "i16 budget expiry",
        &i16(
            Mode::Resident,
            Shutdown::within(std::time::Duration::from_secs(3)),
        ),
        &Script::new()
            .cleanup("PeerStore", Cleanup::IgnoreStop)
            .at(2.0, Request::Shutdown),
    );
    same(
        "i32 component",
        &i32(),
        &every_body(&i32(), 1.0).at(8.0, Request::Shutdown),
    );
    same(
        "i29 pool",
        &i29(2),
        &every_body(&i29(2), 1.0).at(8.0, Request::Shutdown),
    );
    same(
        "i27 locks",
        &i27(true),
        &every_body(&i27(true), 1.0).at(9.0, Request::Shutdown),
    );
}
