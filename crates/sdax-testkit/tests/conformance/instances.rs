//! Suite (c), templates and dynamic instances: `C-30`, `C-51`, `C-64`,
//! `C-65`, the INV-16 half of `C-14`, and `I-34`.
//!
//! `C-30` is the `HC` golden of `sdax-v1/B/CanonicalTests.md` § 4: an endpoint
//! resource, a per-connection template that imports it, and an acceptor
//! service that instantiates one link per connection. `I-34` — a node that
//! depends on *every* instance being ready — is the coverage failure the
//! design comparison held out; `spawns` plus `Child::ready()` is what closes
//! it, and `i34_*` below is the case.

use crate::Drv as ScriptedDriver;
use sdax::host::InstanceId;
use sdax::*;
use sdax_testkit::eol::*;
use std::sync::Arc;
use std::time::Duration;

struct Endpoint;
struct Conn;

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

/// The link template: it imports the endpoint and holds one socket.
fn link(endpoint: Key<Endpoint>, terminal: bool) -> Plan<(), u8> {
    let mut t = Plan::template::<u8>("Link");
    let imported = t.import(endpoint);
    let sock = t
        .resource("Sock")
        .needs(imported)
        .acquire(|cx, _e: Arc<Endpoint>| async move { Ok(cx.hold_value(Conn)) })
        .release(|_cx, _c| async move { Ok(()) });
    if terminal {
        t.service("Pump")
            .needs(sock)
            .terminal()
            .stop_within(secs(1))
            .start(|_cx, _s: Arc<Conn>| async move { Ok(Serving::new((), async { Ok(()) })) });
    }
    t.build(Policy::FailFast, Shutdown::within(secs(3)), Mode::Resident)
        .expect("a valid template")
}

/// `HC`: endpoint, link template, acceptor. `router` adds the I-34 dependent.
fn hc(terminal: bool, router: bool) -> Plan {
    let mut p = Plan::builder("HC");
    let endpoint = p
        .resource("Endpoint")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Endpoint)) })
        .release(|_cx, _e| async move { Ok(()) });
    let child = link(endpoint, terminal);
    let links = p.template("Link", &child);
    let acceptor = p
        .service("Acceptor")
        .needs(endpoint)
        .spawns(&links)
        .stop_within(secs(2))
        .start(|_cx, _e: Arc<Endpoint>| async move { Ok(Serving::new((), async { Ok(()) })) });
    if router {
        p.step("Router")
            .needs(acceptor)
            .run(|_cx, _a: Arc<()>| async move { Ok(()) });
    }
    p.build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Resident)
        .expect("a valid plan")
}

/// `C-30` — per-connection links: spawn two, close one, then shut down.
///
/// The endpoint the template imports is released only after **both**
/// instances have finished their own cleanup (INV-16, T6), and the one that
/// was closed early is gone long before that.
#[test]
fn c30_two_links_one_closed_early_then_a_shutdown() {
    let plan = hc(false, false);
    let script = Script::new()
        .prepare("Endpoint", Body::ok(At::plus(1.0)))
        .cleanup("Endpoint", Cleanup::Ok(secs(1)))
        .prepare("Link/Sock", Body::ok(At::plus(1.0)))
        .cleanup("Link/Sock", Cleanup::Ok(secs(1)))
        .prepare("Acceptor", Body::ok(At::plus(0.0)))
        .spawns(
            "Acceptor",
            [
                SpawnSpec::new("Link", At::plus(0.0)).stopped(At::plus(3.0)),
                SpawnSpec::new("Link", At::plus(0.0)),
            ],
        )
        .serve("Acceptor", [Serve::StopsAfter(secs(0))])
        .at(8.0, Request::Shutdown);
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    let t = d.eol();
    let ids = t.spawned("Link");
    assert_eq!(ids.len(), 2, "two instances: {:?}", t.render());
    let (first, second) = (ids[0], ids[1]);

    // The first is closed at t=3, while the run keeps going.
    let ended: Vec<(InstanceId, Outcome)> = t.instance_ends("Link");
    assert_eq!(ended.len(), 2, "both instances ended");
    // The acceptor is ready at t=1 (the endpoint takes a second), the serve
    // future stops the first child three seconds into its episode, and the
    // socket's own release takes one more.
    assert_eq!(
        t.instance_end_at("Link", first),
        Some(5.0),
        "closed at 1+3+1"
    );
    assert!(
        t.instance_end_at("Link", second).expect("ended") >= 8.0,
        "the second lives until the shutdown"
    );
    // INV-16: the endpoint is released only after both instances are gone.
    let release = t.cleanup_start("Endpoint").expect("the endpoint released");
    for id in [first, second] {
        assert!(
            t.instance_end_at("Link", id).expect("ended") <= release,
            "instance {id} outlived the endpoint's release"
        );
    }
    assert_eq!(d.report.outcome, Outcome::Ok);
    assert!(d.report.is_clean(), "{}", d.report);
}

/// `C-51` — a foreign template handed to `cx.spawn` is refused at the first
/// run, and the run is otherwise unaffected.
#[test]
fn c51_a_foreign_template_is_refused() {
    let plan = hc(false, false);
    let script = Script::new()
        .prepare("Endpoint", Body::ok(At::plus(1.0)))
        .cleanup("Endpoint", Cleanup::Ok(secs(0)))
        .prepare("Acceptor", Body::ok(At::plus(0.0)))
        .spawns("Acceptor", [SpawnSpec::new("Elsewhere", At::plus(0.0))])
        .serve("Acceptor", [Serve::StopsAfter(secs(0))])
        .at(4.0, Request::Shutdown);
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    assert_eq!(
        d.spawns.iter().map(|s| s.result).collect::<Vec<_>>(),
        vec![Err(SpawnError::ForeignTemplate)],
        "a template of another plan is refused, at the first run"
    );
    assert!(d.eol().spawned("Link").is_empty(), "nothing was created");
    assert!(d.report.is_clean(), "{}", d.report);
}

/// `C-64` — an instance whose `terminal` service finishes shuts that instance
/// down as a **normal** shutdown: `Outcome::Ok`, no fault, and the rest of the
/// run untouched.
#[test]
fn c64_a_terminal_service_inside_an_instance_ends_that_instance_cleanly() {
    let plan = hc(true, false);
    let script = Script::new()
        .prepare("Endpoint", Body::ok(At::plus(0.0)))
        .cleanup("Endpoint", Cleanup::Ok(secs(0)))
        .prepare("Link/Sock", Body::ok(At::plus(0.0)))
        .cleanup("Link/Sock", Cleanup::Ok(secs(0)))
        .prepare("Link/Pump", Body::ok(At::plus(0.0)))
        // The pump's own serve future returns at t=3: the instance's scope
        // stops admitting and cleans up, and the run does not.
        .serve("Link/Pump", [Serve::Ok(At::plus(3.0))])
        .prepare("Acceptor", Body::ok(At::plus(0.0)))
        .spawns("Acceptor", [SpawnSpec::new("Link", At::plus(0.0))])
        .serve("Acceptor", [Serve::StopsAfter(secs(0))])
        .at(9.0, Request::Shutdown);
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    let t = d.eol();
    let ids = t.spawned("Link");
    assert_eq!(ids.len(), 1);
    assert_eq!(
        t.instance_ends("Link"),
        vec![(ids[0], Outcome::Ok)],
        "a terminal service finishing is a normal shutdown, not a fault"
    );
    assert!(
        t.instance_end_at("Link", ids[0]).expect("ended") < 9.0,
        "the instance ended on its own, before the run's shutdown"
    );
    assert_eq!(d.report.outcome, Outcome::Ok);
    assert!(d.report.is_clean(), "{}", d.report);
}

/// `C-65` — `cx.spawn` while the scope is stopping is refused.
#[test]
fn c65_a_spawn_while_the_scope_is_stopping_is_refused() {
    let plan = hc(false, false);
    let script = Script::new()
        .prepare("Endpoint", Body::ok(At::plus(0.0)))
        .cleanup("Endpoint", Cleanup::Ok(secs(0)))
        // A slow start body, still in flight when the shutdown lands.
        .prepare("Acceptor", Body::ok(At::tick(6.0)))
        .spawns("Acceptor", [SpawnSpec::new("Link", At::tick(3.0))])
        .at(1.0, Request::Shutdown);
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    assert_eq!(
        d.spawns.iter().map(|s| s.result).collect::<Vec<_>>(),
        vec![Err(SpawnError::ScopeStopping)],
        "a settling scope admits no new instances"
    );
    assert!(d.eol().spawned("Link").is_empty());
}

/// The INV-16 half of `C-14`: a `cancel()` with a live instance drains it, and
/// the key it imports is released only afterwards.
#[test]
fn c14_a_cancel_with_a_live_instance_drains_it_before_the_import_releases() {
    let plan = hc(false, false);
    let script = Script::new()
        .prepare("Endpoint", Body::ok(At::plus(0.0)))
        .cleanup("Endpoint", Cleanup::Ok(secs(1)))
        .prepare("Link/Sock", Body::ok(At::plus(0.0)))
        .cleanup("Link/Sock", Cleanup::Ok(secs(2)))
        .prepare("Acceptor", Body::ok(At::plus(0.0)))
        .spawns("Acceptor", [SpawnSpec::new("Link", At::plus(0.0))])
        .serve("Acceptor", [Serve::StopsAfter(secs(0))])
        .at(2.0, Request::Cancel);
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    let t = d.eol();
    let id = t.spawned("Link")[0];
    assert_eq!(
        t.instance_ends("Link"),
        vec![(id, Outcome::Cancelled)],
        "the instance ended with the cancel that reached it"
    );
    let release = t.cleanup_start("Endpoint").expect("released");
    assert!(t.instance_end_at("Link", id).expect("ended") <= release);
    assert_eq!(d.report.outcome, Outcome::Cancelled);
}

/// `I-34` — a node that depends on **every** instance being ready.
///
/// The acceptor's start body spawns its links and awaits each `Child::ready()`
/// before returning `Serving` (INV-17), so the scope's readiness includes the
/// instances and anything that `needs` the acceptor starts after them. This is
/// the intent the design comparison recorded as the one uncovered case.
#[test]
fn i34_a_dependent_of_every_instance_starts_only_once_they_are_all_ready() {
    let plan = hc(false, true);
    let script = Script::new()
        .prepare("Endpoint", Body::ok(At::plus(0.0)))
        .cleanup("Endpoint", Cleanup::Ok(secs(0)))
        .prepare("Link/Sock", Body::ok(At::plus(2.0)))
        .cleanup("Link/Sock", Cleanup::Ok(secs(0)))
        .prepare("Acceptor", Body::ok(At::plus(0.0)))
        .spawns(
            "Acceptor",
            [
                SpawnSpec::new("Link", At::plus(0.0)).awaited(),
                SpawnSpec::new("Link", At::plus(0.0)).awaited(),
            ],
        )
        .serve("Acceptor", [Serve::StopsAfter(secs(0))])
        .prepare("Router", Body::ok(At::plus(0.0)))
        .at(9.0, Request::Shutdown);
    let d = ScriptedDriver::run(&plan, &script).expect("runs");
    d.check();
    let t = d.eol();
    let ids = t.spawned("Link");
    assert_eq!(ids.len(), 2);
    let router = t.start("Router").expect("the router started");
    for id in &ids {
        let ready = t
            .at_of(is_ready, "Link/Sock", *id)
            .unwrap_or_else(|| panic!("instance {id} never became ready"));
        assert!(
            ready <= router,
            "the router started at {router} before instance {id} was ready at {ready}"
        );
    }
    assert!(
        t.ready("Acceptor").expect("ready") >= 2.0,
        "the acceptor's own readiness waited for the instances"
    );
    assert!(d.report.is_clean(), "{}", d.report);
}
