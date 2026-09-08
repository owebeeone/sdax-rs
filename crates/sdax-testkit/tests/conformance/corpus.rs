//! The training programs suite (c) runs, written in the author surface.
//! Ids are `sdax-v1/corpus/Intents.md`. Bodies are the smallest thing that
//! type-checks: the scripted driver never runs them.
//!
//! `Mode` is explicit in the adopted contract. Where B's row keeps a
//! resources-only plan up until `@t shutdown`, the program takes a `Mode`
//! argument and the test passes `Resident` (INV-19 allows it).

#![allow(dead_code)]

use sdax::*;
use std::sync::Arc;
use std::time::Duration;

pub struct Transport;
pub struct PeerStore;
pub struct RoutingTable;
pub struct Receipt;
pub struct Endpoint;
pub struct Db;
pub struct Snapshot;
pub struct Conn;
pub struct NetHandle;
pub struct Unit;

pub fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

pub fn shutdown10() -> Shutdown {
    Shutdown::within(secs(10))
}

fn res(p: &mut PlanBuilder, name: &str) -> Key<Unit> {
    p.resource(name)
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Unit)) })
        .release(|_cx, _u| async move { Ok(()) })
}

fn res_needs<D: Deps>(p: &mut PlanBuilder, name: &str, deps: D) -> Key<Unit> {
    p.resource(name)
        .needs(deps)
        .acquire(|cx, _d| async move { Ok(cx.hold_value(Unit)) })
        .release(|_cx, _u| async move { Ok(()) })
}

fn step_needs<D: Deps>(p: &mut PlanBuilder, name: &str, deps: D) -> Key<()> {
    p.step(name)
        .needs(deps)
        .run(|_cx, _d| async move { Ok(()) })
}

/// I-01 / I-33 — dependency-ordered startup with a compensated effect.
pub fn i01(mode: Mode) -> Plan {
    let mut p = Plan::builder("Startup");
    let transport = res(&mut p, "Transport");
    let peers = res_needs(&mut p, "PeerStore", transport);
    let routes = p
        .resource("RoutingTable")
        .needs(transport)
        .acquire(|cx, _t: Arc<Unit>| async move { Ok(cx.hold_value(RoutingTable)) })
        .release(release::by_drop());
    p.effect("Registration")
        .needs((peers, routes))
        .on_ambiguous(Ambiguity::Report)
        .perform(|cx, _d: (Arc<Unit>, Arc<RoutingTable>)| async move { Ok(cx.hold_value(Receipt)) })
        .compensate(|_cx, _r| async move { Ok(()) });
    p.build(Policy::FailFast, shutdown10(), mode)
        .expect("valid")
}

/// I-02 — two unrelated chains A→B→C and X→Y.
pub fn i02() -> Plan {
    let mut p = Plan::builder("Two chains");
    let a = res(&mut p, "A");
    let b = res_needs(&mut p, "B", a);
    step_needs(&mut p, "C", b);
    let x = res(&mut p, "X");
    step_needs(&mut p, "Y", x);
    p.build(Policy::FailFast, shutdown10(), Mode::Finite)
        .expect("valid")
}

/// I-04 — the per-request plan with a typed output.
pub fn i04() -> Plan<u32> {
    let mut p = Plan::builder("Request");
    let flags = res(&mut p, "Flags");
    let db = res(&mut p, "Db");
    let a = p
        .step("SvcA")
        .needs((flags, db))
        .run(|_cx, _d: (Arc<Unit>, Arc<Unit>)| async move { Ok(1u32) });
    let b = p
        .step("SvcB")
        .needs(flags)
        .run(|_cx, _f: Arc<Unit>| async move { Ok(2u32) });
    let agg = p
        .step("Aggregate")
        .needs((a, b))
        .run(|_cx, d: (Arc<u32>, Arc<u32>)| async move { Ok(*d.0 + *d.1) });
    p.export(agg)
        .build(Policy::FailFast, shutdown10(), Mode::Finite)
        .expect("valid")
}

/// I-05 — endpoint, accept loop (service), published address (effect).
pub fn i05() -> Plan {
    let mut p = Plan::builder("Mesh enable");
    let endpoint = res(&mut p, "Endpoint");
    let accept = p
        .service("AcceptLoop")
        .needs(endpoint)
        .stop_within(secs(2))
        .initialize(|_cx, _e: Arc<Unit>| async move { Ok(()) })
        .serve(|_cx, _handle| async move { Ok(()) });
    p.effect("PublishAddr")
        .needs(accept)
        .on_ambiguous(Ambiguity::Report)
        .perform(|cx, _a: Arc<()>| async move { Ok(cx.hold_value(Receipt)) })
        .compensate(|_cx, _r| async move { Ok(()) });
    p.build(Policy::FailFast, shutdown10(), Mode::Resident)
        .expect("valid")
}

/// I-07 — a finite migration, a long-lived exporter, an API that needs both.
/// `restart` turns on C-57's idempotent restart of the exporter.
pub fn i07(restart: bool) -> Plan {
    let mut p = Plan::builder("Migrate and serve");
    let migrate = p.step("Migrate").run(|_cx, ()| async move { Ok(()) });
    let mut exporter = p.service("Exporter").stop_within(secs(2));
    if restart {
        exporter = exporter
            .idempotent()
            .restart(Restart::on_error(Backoff::fixed(secs(1))));
    }
    let exporter = exporter
        .initialize(|_cx, ()| async move { Ok(()) })
        .serve(|_cx, _handle| async move { Ok(()) });
    p.service("Api")
        .needs((migrate, exporter))
        .stop_within(secs(1))
        .initialize(|_cx, _d: (Arc<()>, Arc<()>)| async move { Ok(()) })
        .serve(|_cx, _handle| async move { Ok(()) });
    p.build(Policy::FailFast, shutdown10(), Mode::Resident)
        .expect("valid")
}

/// I-08 / I-38 — three resources after a base, then a step needing all.
pub fn i08(policy: Policy) -> Plan {
    let mut p = Plan::builder("Fail fast");
    let base = res(&mut p, "Base");
    let a = res_needs(&mut p, "A", base);
    let b = res_needs(&mut p, "B", base);
    let c = res_needs(&mut p, "C", base);
    step_needs(&mut p, "Down", (a, b, c));
    p.build(policy, shutdown10(), Mode::Finite).expect("valid")
}

/// I-09 — a port bound inside `hold`, then checked.
pub fn i09() -> Plan {
    let mut p = Plan::builder("Partial init");
    let port = res(&mut p, "Port");
    step_needs(&mut p, "Check", port);
    p.build(Policy::FailFast, shutdown10(), Mode::Finite)
        .expect("valid")
}

/// I-11 — two resources; cancelled before start.
pub fn i11() -> Plan {
    let mut p = Plan::builder("Cancel before start");
    let a = res(&mut p, "A");
    res_needs(&mut p, "B", a);
    p.build(Policy::FailFast, shutdown10(), Mode::Resident)
        .expect("valid")
}

/// I-12 — A, B needs A, C needs B; cancelled during B.
pub fn i12() -> Plan {
    let mut p = Plan::builder("Cancel during");
    let a = res(&mut p, "A");
    let b = res_needs(&mut p, "B", a);
    step_needs(&mut p, "C", b);
    p.build(Policy::FailFast, shutdown10(), Mode::Resident)
        .expect("valid")
}

/// I-15 — an effect compensated while its transport is alive.
pub fn i15(mode: Mode, within: Option<Duration>, ambiguity: Ambiguity) -> Plan {
    let mut p = Plan::builder("Deregister last");
    let transport = res(&mut p, "Transport");
    let mut eff = p.effect("Registration").needs(transport);
    if let Some(d) = within {
        eff = eff.within(d);
    }
    if ambiguity != Ambiguity::Report {
        eff = eff.idempotent();
    }
    eff.on_ambiguous(ambiguity)
        .identified_by(transport)
        .perform(|cx, (_t, _id)| async move { Ok(cx.hold_value(Receipt)) })
        .recover_unknown(|_, _| async { Ok(Recovery::Resolved) })
        .compensate(|_cx, _r| async move { Ok(()) });
    p.build(Policy::FailFast, shutdown10(), mode)
        .expect("valid")
}

/// I-16 — two independent resources over one transport.
pub fn i16(mode: Mode, shutdown: Shutdown) -> Plan {
    let mut p = Plan::builder("Concurrent cleanup");
    let transport = res(&mut p, "Transport");
    res_needs(&mut p, "PeerStore", transport);
    res_needs(&mut p, "RoutingTable", transport);
    p.build(Policy::FailFast, shutdown, mode).expect("valid")
}

/// I-18 — a database and a worker with a 3-tick stop budget.
pub fn i18() -> Plan {
    let mut p = Plan::builder("Bounded shutdown");
    let db = res(&mut p, "Db");
    p.service("Worker")
        .needs(db)
        .stop_within(secs(3))
        .initialize(|_cx, _d: Arc<Unit>| async move { Ok(()) })
        .serve(|_cx, _handle| async move { Ok(()) });
    p.build(Policy::FailFast, shutdown10(), Mode::Resident)
        .expect("valid")
}

/// I-20 — two payload steps under `Isolate`.
pub fn i20() -> Plan {
    let mut p = Plan::builder("Isolate");
    let a = res(&mut p, "A");
    step_needs(&mut p, "P1", a);
    step_needs(&mut p, "P2", a);
    p.build(Policy::Isolate, shutdown10(), Mode::Finite)
        .expect("valid")
}

/// I-21 — two acquisitions that fail in the same tick.
pub fn i21() -> Plan {
    let mut p = Plan::builder("Two faults");
    let base = res(&mut p, "Base");
    res_needs(&mut p, "B", base);
    res_needs(&mut p, "C", base);
    p.build(Policy::FailFast, shutdown10(), Mode::Finite)
        .expect("valid")
}

/// I-23 — five probes whose failures are values, then a decision.
pub fn i23() -> Plan<u8> {
    let mut p = Plan::builder("Collect all");
    let transport = res(&mut p, "Transport");
    let mut probes = Vec::new();
    for name in ["Probe1", "Probe2", "Probe3", "Probe4", "Probe5"] {
        probes.push(
            p.try_step(name)
                .needs(transport)
                .run(|_cx, _t: Arc<Unit>| async move { Ok(1u8) }),
        );
    }
    let d = (probes[0], probes[1], probes[2], probes[3], probes[4]);
    let decide = p
        .step("Decide")
        .needs(d)
        .run(|_cx, _r| async move { Ok(7u8) });
    p.export(decide)
        .build(Policy::FailFast, shutdown10(), Mode::Finite)
        .expect("valid")
}

/// I-24 / I-26 — a retried connection with exponential backoff.
pub fn i24(initial: Duration) -> Plan {
    let mut p = Plan::builder("Retry with backoff");
    p.resource("Conn")
        .retry(Retry::attempts(3).backoff(Backoff::exponential(initial, 2, secs(8))))
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Conn)) })
        .release(|_cx, _c| async move { Ok(()) });
    p.build(Policy::FailFast, shutdown10(), Mode::Finite)
        .expect("valid")
}

/// I-27 — two migrators serialised by an exclusive lock; `readers` adds two
/// `.shared(db)` readers (C-62).
pub fn i27(readers: bool) -> Plan {
    let mut p = Plan::builder("Exclusive migrations");
    let db = res(&mut p, "Db");
    let miga = p
        .step("MigA")
        .needs(db)
        .exclusive(db)
        .run(|_cx, _d: Arc<Unit>| async move { Ok(()) });
    let migb = p
        .step("MigB")
        .needs(db)
        .exclusive(db)
        .run(|_cx, _d: Arc<Unit>| async move { Ok(()) });
    if readers {
        for name in ["R1", "R2"] {
            p.step(name)
                .needs(db)
                .shared(db)
                .run(|_cx, _d: Arc<Unit>| async move { Ok(()) });
        }
    }
    p.service("App")
        .needs((miga, migb))
        .stop_within(secs(1))
        .initialize(|_cx, _d: (Arc<()>, Arc<()>)| async move { Ok(()) })
        .serve(|_cx, _handle| async move { Ok(()) });
    p.build(Policy::FailFast, shutdown10(), Mode::Resident)
        .expect("valid")
}

/// I-29 — bounded blocking verification; `extra` adds more blocking steps
/// on the same pool (C-29's variant).
pub fn i29(extra: usize) -> Plan {
    let mut p = Plan::builder("Bounded blocking");
    let cpu = p.pool("cpu", 2);
    let snapshot = res(&mut p, "Snapshot");
    let verify = p
        .blocking_step("Verify")
        .needs(snapshot)
        .on(cpu)
        .run(|_cx, _s: Arc<Unit>| Ok(()));
    for i in 0..extra {
        p.blocking_step(&format!("Verify{}", i + 2))
            .needs(snapshot)
            .on(cpu)
            .run(|_cx, _s: Arc<Unit>| Ok(()));
    }
    p.service("Serve")
        .needs(verify)
        .stop_within(secs(1))
        .initialize(|_cx, _v: Arc<()>| async move { Ok(()) })
        .serve(|_cx, _handle| async move { Ok(()) });
    p.build(Policy::FailFast, shutdown10(), Mode::Resident)
        .expect("valid")
}

/// I-32 — networking as a component with a typed output.
pub fn i32() -> Plan {
    let mut inner = Plan::builder("Networking");
    let transport = res(&mut inner, "Transport");
    res_needs(&mut inner, "PeerStore", transport);
    let routes = res_needs(&mut inner, "RoutingTable", transport);
    let handle = inner
        .step("Handle")
        .needs(routes)
        .run(|_cx, _r: Arc<Unit>| async move { Ok(NetHandle) });
    let net = inner
        .export(handle)
        .build(Policy::FailFast, Shutdown::within(secs(5)), Mode::Finite)
        .expect("valid");
    let mut p = Plan::builder("Process");
    let net_key = p.component("Net", &net, ());
    p.effect("Registration")
        .needs(net_key)
        .on_ambiguous(Ambiguity::Report)
        .perform(|cx, _n: Arc<NetHandle>| async move { Ok(cx.hold_value(Receipt)) })
        .compensate(|_cx, _r| async move { Ok(()) });
    p.build(Policy::FailFast, shutdown10(), Mode::Resident)
        .expect("valid")
}

/// I-40 — a renewal service restarting idempotently across a partition.
pub fn i40() -> Plan {
    let mut p = Plan::builder("Renewal");
    let transport = res(&mut p, "Transport");
    let local = res_needs(&mut p, "LocalReg", transport);
    p.service("Renew")
        .needs(local)
        .idempotent()
        .restart(Restart::on_error(Backoff::exponential(secs(1), 2, secs(8))))
        .stop_within(secs(2))
        .initialize(|_cx, _l: Arc<Unit>| async move { Ok(()) })
        .serve(|_cx, _handle| async move { Ok(()) });
    p.build(Policy::FailFast, shutdown10(), Mode::Resident)
        .expect("valid")
}

/// I-41 — four adapters behind ports, and a discovery service.
pub fn i41() -> Plan {
    let mut p = Plan::builder("Node");
    let clock = res(&mut p, "Clock");
    let storage = res(&mut p, "Storage");
    let signer = res(&mut p, "Signer");
    let transport = res(&mut p, "Transport");
    p.service("Discovery")
        .needs((storage, signer, transport, clock))
        .stop_within(secs(1))
        .initialize(|_cx, _d: (Arc<Unit>, Arc<Unit>, Arc<Unit>, Arc<Unit>)| async move { Ok(()) })
        .serve(|_cx, _handle| async move { Ok(()) });
    p.build(Policy::FailFast, shutdown10(), Mode::Resident)
        .expect("valid")
}

/// C-59 — a chain A → B → C → D under `Isolate`, plus an unrelated E.
pub fn chain_isolate() -> Plan {
    let mut p = Plan::builder("Chain");
    let a = res(&mut p, "A");
    let b = res_needs(&mut p, "B", a);
    let c = res_needs(&mut p, "C", b);
    step_needs(&mut p, "D", c);
    res(&mut p, "E");
    p.build(Policy::Isolate, shutdown10(), Mode::Finite)
        .expect("valid")
}

/// C-60 — the cooperative effect of `Proposal.md` § B.2.
pub fn cooperative_effect() -> Plan {
    let mut p = Plan::builder("Cooperative");
    let net = res(&mut p, "Net");
    p.effect("Registration")
        .needs(net)
        .on_ambiguous(Ambiguity::Recover)
        .idempotent()
        .cooperative(secs(1))
        .identified_by(net)
        .perform(|cx, (_n, _id)| async move { Ok(cx.hold_value(Receipt)) })
        .recover_unknown(|_, _| async { Ok(Recovery::Resolved) })
        .compensate(|_cx, _r| async move { Ok(()) });
    p.build(Policy::FailFast, shutdown10(), Mode::Resident)
        .expect("valid")
}

/// C-63 — three async steps `.limit(cpu)` on a pool of two.
pub fn three_limited_steps() -> Plan {
    let mut p = Plan::builder("Limited");
    let cpu = p.pool("cpu", 2);
    for name in ["S1", "S2", "S3"] {
        p.step(name).limit(cpu).run(|_cx, ()| async move { Ok(()) });
    }
    p.build(Policy::FailFast, shutdown10(), Mode::Finite)
        .expect("valid")
}
