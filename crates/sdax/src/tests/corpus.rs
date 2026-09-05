//! The training programs the planner tests run on, written in this crate's
//! surface. Ids are `sdax-v1/corpus/Intents.md`.
//!
//! Bodies are the smallest thing that type-checks: Stage 0 never runs them.

use crate::*;
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug)]
pub struct Transport {
    #[allow(dead_code)]
    pub addr: &'static str,
}
pub struct PeerStore;
pub struct RoutingTable;
pub struct Receipt(#[allow(dead_code)] pub &'static str);
pub struct Endpoint;
pub struct Db;
pub struct Snapshot;
pub struct Conn;
pub struct LinkEntry;
pub struct PeerLink(#[allow(dead_code)] pub u32);
pub struct NetHandle;

pub fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn shutdown10() -> Shutdown {
    Shutdown::within(secs(10))
}

/// I-01 / I-33 — dependency-ordered startup with a compensated effect.
pub fn i01() -> Result<Plan, Invalid> {
    let mut p = Plan::builder("Startup");
    let transport = p
        .resource("Transport")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Transport { addr: "t" })) })
        .release(|_cx, _t| async move { Ok(()) });
    let peers = p
        .resource("PeerStore")
        .needs(transport)
        .acquire(|cx, _t: Arc<Transport>| async move { Ok(cx.hold_value(PeerStore)) })
        .release(|_cx, _s| async move { Ok(()) });
    let routes = p
        .resource("RoutingTable")
        .needs(transport)
        .acquire(|cx, _t: Arc<Transport>| async move { Ok(cx.hold_value(RoutingTable)) })
        .release(release::by_drop());
    p.effect("Registration")
        .needs((peers, routes))
        .on_ambiguous(Ambiguity::Report)
        .perform(|cx, _d: (Arc<PeerStore>, Arc<RoutingTable>)| async move {
            Ok(cx.hold_value(Receipt("r")))
        })
        .compensate(|_cx, _r| async move { Ok(()) });
    p.build(Policy::FailFast, shutdown10(), Mode::Finite)
}

/// I-16 — two independent resources over one transport; no effect.
pub fn i16() -> Result<Plan, Invalid> {
    let mut p = Plan::builder("Concurrent cleanup");
    let transport = p
        .resource("Transport")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Transport { addr: "t" })) })
        .release(|_cx, _t| async move { Ok(()) });
    p.resource("PeerStore")
        .needs(transport)
        .acquire(|cx, _t: Arc<Transport>| async move { Ok(cx.hold_value(PeerStore)) })
        .release(|_cx, _s| async move { Ok(()) });
    p.resource("RoutingTable")
        .needs(transport)
        .acquire(|cx, _t: Arc<Transport>| async move { Ok(cx.hold_value(RoutingTable)) })
        .release(|_cx, _r| async move { Ok(()) });
    p.build(Policy::FailFast, shutdown10(), Mode::Finite)
}

/// I-15 — an effect that must be compensated while its transport is alive.
pub fn i15() -> Result<Plan, Invalid> {
    i15_with(I15Opts::default())
}

/// Knobs for the I-15 mutants (P-06, C-61).
#[derive(Clone, Copy)]
pub struct I15Opts {
    pub retry: bool,
    pub idempotent: bool,
    pub ambiguity: Ambiguity,
    pub persistent: bool,
}

impl Default for I15Opts {
    fn default() -> Self {
        I15Opts {
            retry: false,
            idempotent: false,
            ambiguity: Ambiguity::Report,
            persistent: false,
        }
    }
}

/// I-15 with the effect node's declaration adjusted, for the mutant tests.
pub fn i15_with(o: I15Opts) -> Result<Plan, Invalid> {
    let mut p = Plan::builder("Deregister last");
    let transport = p
        .resource("Transport")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Transport { addr: "t" })) })
        .release(|_cx, _t| async move { Ok(()) });
    let mut eff = p.effect("Registration").needs(transport);
    if o.retry {
        eff = eff.retry(Retry::attempts(2));
    }
    if o.idempotent {
        eff = eff.idempotent();
    }
    let performed = eff
        .on_ambiguous(o.ambiguity)
        .perform(|cx, _t: Arc<Transport>| async move { Ok(cx.hold_value(Receipt("r"))) });
    if o.persistent {
        performed.persistent();
    } else {
        performed.compensate(|_cx, _r| async move { Ok(()) });
    }
    p.build(Policy::FailFast, shutdown10(), Mode::Finite)
}

/// I-05 — endpoint, accept loop (service), published address (effect).
pub fn i05() -> Result<Plan, Invalid> {
    let mut p = Plan::builder("Mesh enable");
    let endpoint = p
        .resource("Endpoint")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Endpoint)) })
        .release(|_cx, _e| async move { Ok(()) });
    let accept = p
        .service("AcceptLoop")
        .needs(endpoint)
        .stop_within(secs(2))
        .start(|cx, _e: Arc<Endpoint>| async move {
            Ok(Serving::new((), async move {
                cx.until_stop(std::future::pending::<()>()).await;
                Ok(())
            }))
        });
    p.effect("PublishAddr")
        .needs(accept)
        .on_ambiguous(Ambiguity::Report)
        .perform(|cx, _a: Arc<()>| async move { Ok(cx.hold_value(Receipt("addr"))) })
        .compensate(|_cx, _r| async move { Ok(()) });
    p.build(Policy::FailFast, shutdown10(), Mode::Resident)
}

/// I-07 — a finite migration, a long-lived exporter, and an API that needs both.
pub fn i07(shutdown: Shutdown, exporter_stop: Option<Duration>) -> Result<Plan, Invalid> {
    let mut p = Plan::builder("Migrate and serve");
    let migrate = p.step("Migrate").run(|_cx, ()| async move { Ok(()) });
    let mut exporter = p.service("Exporter");
    if let Some(d) = exporter_stop {
        exporter = exporter.stop_within(d);
    }
    let exporter = exporter.start(|_cx, ()| async move { Ok(Serving::new((), async { Ok(()) })) });
    p.service("Api")
        .needs((migrate, exporter))
        .stop_within(secs(1))
        .start(|_cx, _d: (Arc<()>, Arc<()>)| async move { Ok(Serving::new((), async { Ok(()) })) });
    p.build(Policy::FailFast, shutdown, Mode::Resident)
}

/// I-18 — a database and a worker that ignores its stop signal.
pub fn i18(worker_stop: Duration, dup_db: bool) -> Result<Plan, Invalid> {
    let mut p = Plan::builder("Bounded shutdown");
    let db = p
        .resource("Db")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Db)) })
        .release(|_cx, _d| async move { Ok(()) });
    if dup_db {
        p.resource("Db")
            .acquire(|cx, ()| async move { Ok(cx.hold_value(Db)) })
            .release(|_cx, _d| async move { Ok(()) });
    }
    p.service("Worker")
        .needs(db)
        .stop_within(worker_stop)
        .start(|_cx, _d: Arc<Db>| async move { Ok(Serving::new((), async { Ok(()) })) });
    p.build(Policy::FailFast, shutdown10(), Mode::Resident)
}

/// I-23 — five probes whose failures are values, then a decision.
pub fn i23(consume: bool) -> Result<Plan, Invalid> {
    let mut p = Plan::builder("Collect all");
    let transport = p
        .resource("Transport")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Transport { addr: "t" })) })
        .release(|_cx, _t| async move { Ok(()) });
    let mut probes = Vec::new();
    for name in ["Probe1", "Probe2", "Probe3", "Probe4", "Probe5"] {
        probes.push(
            p.try_step(name)
                .needs(transport)
                .run(|_cx, _t: Arc<Transport>| async move { Ok(1u8) }),
        );
    }
    if consume {
        let d = (probes[0], probes[1], probes[2], probes[3], probes[4]);
        p.step("Decide")
            .needs(d)
            .run(|_cx, _r| async move { Ok(()) });
    }
    p.build(Policy::FailFast, shutdown10(), Mode::Finite)
}

/// I-24 — a retried connection. `twice` sets `.retry` twice (V-DUP-ATTR).
pub fn i24(twice: bool) -> Result<Plan, Invalid> {
    let mut p = Plan::builder("Retry with backoff");
    let mut conn = p
        .resource("Conn")
        .retry(Retry::attempts(3).backoff(Backoff::exponential(secs(1), 2, secs(8))));
    if twice {
        conn = conn.retry(Retry::attempts(2));
    }
    conn.acquire(|cx, ()| async move { Ok(cx.hold_value(Conn)) })
        .release(|_cx, _c| async move { Ok(()) });
    p.build(Policy::FailFast, shutdown10(), Mode::Finite)
}

/// I-27 — two migrators serialised by an exclusive lock on the database.
pub fn i27(migb_needs_db: bool) -> Result<Plan, Invalid> {
    let mut p = Plan::builder("Exclusive migrations");
    let db = p
        .resource("Db")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Db)) })
        .release(|_cx, _d| async move { Ok(()) });
    let miga = p
        .step("MigA")
        .needs(db)
        .exclusive(db)
        .run(|_cx, _d: Arc<Db>| async move { Ok(()) });
    let migb = if migb_needs_db {
        p.step("MigB")
            .needs(db)
            .exclusive(db)
            .run(|_cx, _d: Arc<Db>| async move { Ok(()) })
    } else {
        p.step("MigB")
            .exclusive(db)
            .run(|_cx, ()| async move { Ok(()) })
    };
    p.service("App")
        .needs((miga, migb))
        .stop_within(secs(1))
        .start(|_cx, _d: (Arc<()>, Arc<()>)| async move { Ok(Serving::new((), async { Ok(()) })) });
    p.build(Policy::FailFast, shutdown10(), Mode::Resident)
}

/// I-29 — a bounded blocking verification. `resident_holders` services also
/// take the pool, which is the starvation mutant (P-09).
pub fn i29(pool_limit: usize, resident_holders: usize) -> Result<Plan, Invalid> {
    let mut p = Plan::builder("Bounded blocking");
    let cpu = p.pool("cpu", pool_limit);
    let snapshot = p
        .resource("Snapshot")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Snapshot)) })
        .release(|_cx, _s| async move { Ok(()) });
    for i in 0..resident_holders {
        p.service(if i == 0 { "Hold0" } else { "Hold1" })
            .limit(cpu)
            .stop_within(secs(1))
            .start(|_cx, ()| async move { Ok(Serving::new((), async { Ok(()) })) });
    }
    let verify = p
        .blocking_step("Verify")
        .needs(snapshot)
        .on(cpu)
        .run(|_cx, _s: Arc<Snapshot>| Ok(()));
    p.service("Serve")
        .needs(verify)
        .stop_within(secs(1))
        .start(|_cx, _v: Arc<()>| async move { Ok(Serving::new((), async { Ok(()) })) });
    p.build(Policy::FailFast, shutdown10(), Mode::Resident)
}

/// I-32 — "networking" as a reusable component with a typed output.
pub fn networking() -> Result<Plan<NetHandle>, Invalid> {
    let mut p = Plan::builder("Networking");
    let transport = p
        .resource("Transport")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Transport { addr: "t" })) })
        .release(|_cx, _t| async move { Ok(()) });
    p.resource("PeerStore")
        .needs(transport)
        .acquire(|cx, _t: Arc<Transport>| async move { Ok(cx.hold_value(PeerStore)) })
        .release(|_cx, _s| async move { Ok(()) });
    let routes = p
        .resource("RoutingTable")
        .needs(transport)
        .acquire(|cx, _t: Arc<Transport>| async move { Ok(cx.hold_value(RoutingTable)) })
        .release(|_cx, _r| async move { Ok(()) });
    let handle = p
        .step("Handle")
        .needs(routes)
        .run(|_cx, _r: Arc<RoutingTable>| async move { Ok(NetHandle) });
    p.export(handle)
        .build(Policy::FailFast, shutdown10(), Mode::Finite)
}

/// I-32 — the component used as one node of a larger plan.
pub fn i32(inner: Shutdown) -> Result<Plan, Invalid> {
    let mut inner_plan = Plan::builder("Networking");
    let transport = inner_plan
        .resource("Transport")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Transport { addr: "t" })) })
        .release(|_cx, _t| async move { Ok(()) });
    inner_plan
        .resource("PeerStore")
        .needs(transport)
        .acquire(|cx, _t: Arc<Transport>| async move { Ok(cx.hold_value(PeerStore)) })
        .release(|_cx, _s| async move { Ok(()) });
    let routes = inner_plan
        .resource("RoutingTable")
        .needs(transport)
        .acquire(|cx, _t: Arc<Transport>| async move { Ok(cx.hold_value(RoutingTable)) })
        .release(|_cx, _r| async move { Ok(()) });
    let handle = inner_plan
        .step("Handle")
        .needs(routes)
        .run(|_cx, _r: Arc<RoutingTable>| async move { Ok(NetHandle) });
    let net = inner_plan
        .export(handle)
        .build(Policy::FailFast, inner, Mode::Finite)?;

    let mut p = Plan::builder("Process");
    let net_key = p.component("Net", &net);
    p.effect("Registration")
        .needs(net_key)
        .on_ambiguous(Ambiguity::Report)
        .perform(|cx, _n: Arc<NetHandle>| async move { Ok(cx.hold_value(Receipt("r"))) })
        .compensate(|_cx, _r| async move { Ok(()) });
    p.build(Policy::FailFast, shutdown10(), Mode::Finite)
}

/// I-30's per-connection template, importing the mesh endpoint.
pub fn link_plan(endpoint: Key<Endpoint>) -> Result<Plan<(), PeerLink>, Invalid> {
    let mut t = Plan::template::<PeerLink>("Link");
    let ep = t.import(endpoint);
    let conn = t.input();
    let entry = t
        .resource("LinkEntry")
        .needs((conn, ep))
        .acquire(
            |cx, _d: (Arc<PeerLink>, Arc<Endpoint>)| async move { Ok(cx.hold_value(LinkEntry)) },
        )
        .release(|_cx, _e| async move { Ok(()) });
    t.service("Dispatcher")
        .needs(entry)
        .stop_within(secs(1))
        .start(|_cx, _e: Arc<LinkEntry>| async move { Ok(Serving::new((), async { Ok(()) })) });
    t.service("UnlinkOnClose")
        .needs(entry)
        .terminal()
        .stop_within(secs(1))
        .start(|_cx, _e: Arc<LinkEntry>| async move { Ok(Serving::new((), async { Ok(()) })) });
    t.build(Policy::Isolate, Shutdown::within(secs(2)), Mode::Resident)
}

/// A template with no imports at all, for the tests that need one registered
/// without tying it to any particular key.
pub fn plain_link() -> Result<Plan<(), PeerLink>, Invalid> {
    let mut t = Plan::template::<PeerLink>("Link");
    let conn = t.input();
    t.step("Inner")
        .needs(conn)
        .run(|_cx, _c: Arc<PeerLink>| async move { Ok(()) });
    t.build(Policy::Isolate, Shutdown::within(secs(2)), Mode::Finite)
}

/// I-30 — a mesh whose accept loop spawns per-connection links.
pub fn i30() -> Result<Plan, Invalid> {
    let mut p = Plan::builder("Mesh");
    let endpoint = p
        .resource("Endpoint")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Endpoint)) })
        .release(|_cx, _e| async move { Ok(()) });
    let links = p.template("Link", &link_plan(endpoint)?);
    p.service("AcceptLoop")
        .needs(endpoint)
        .spawns(&links)
        .stop_within(secs(2))
        .start(move |cx, _e: Arc<Endpoint>| async move {
            let child = cx.spawn(&links, PeerLink(1))?;
            child.ready().await?;
            Ok(Serving::new((), async { Ok(()) }))
        });
    p.build(Policy::FailFast, shutdown10(), Mode::Resident)
}
