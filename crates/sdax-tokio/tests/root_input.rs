//! `I-04`, done properly: one plan value, many runs, each with its own typed
//! input.
//!
//! The corpus intent is "build the per-request orchestration once, run it for
//! thousands of concurrent requests, *each with its own typed state*". The
//! conformance row for it (`c04_two_runs_of_one_plan_value_share_nothing`)
//! asserts **isolation** — two runs of one `Plan` value share no slots, locks
//! or pools — and it passed while the intent was half-met, because its two runs
//! never needed different inputs to prove it. Nothing here holds a `Mutex`, an
//! `Arc<Cell>` or any other shared cell: the only way a run can differ from its
//! sibling is the value handed to `start`.

use sdax::*;
use sdax_tokio::{PlanStart, TokioRuntime};
use std::sync::Arc;
use std::time::Duration;

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn rt() -> (tokio::runtime::Runtime, Arc<TokioRuntime>) {
    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .expect("runtime");
    let rt = Arc::new(TokioRuntime::current_thread_no_background_drain(
        tokio_rt.handle().clone(),
    ));
    (tokio_rt, rt)
}

/// One request's state, as an author would write it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Request {
    tenant: String,
    items: u32,
}

/// What the plan exports.
#[derive(Debug, PartialEq, Eq)]
struct Response {
    tenant: String,
    total: u32,
}

/// The per-request plan: built once, run per request.
///
/// `Db` is a resource every run acquires for itself; `Price` and `Quota` read
/// the run's own input; `Respond` exports the answer derived from both.
fn request_plan() -> Plan<Response, Request> {
    let mut p = Plan::with_input::<Request>("Request");
    let req = p.input();
    let db = p
        .resource("Db")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(7u32)) })
        .release(|_cx, _d: Arc<u32>| async move { Ok(()) });
    let price = p
        .step("Price")
        .needs((req, db))
        .run(|_cx, d: (Arc<Request>, Arc<u32>)| async move { Ok(d.0.items * *d.1) });
    let quota = p
        .step("Quota")
        .needs(req)
        .run(|_cx, r: Arc<Request>| async move { Ok(r.tenant.len() as u32) });
    let respond = p.step("Respond").needs((req, price, quota)).run(
        |_cx, d: (Arc<Request>, Arc<u32>, Arc<u32>)| async move {
            Ok(Response {
                tenant: d.0.tenant.clone(),
                total: *d.1 + *d.2,
            })
        },
    );
    p.export(respond)
        .build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Finite)
        .expect("a valid per-request plan")
}

/// `I-04` — one plan value, two runs, two different inputs, two answers each
/// derived from its own input and from nothing shared.
#[test]
fn i04_one_plan_two_runs_each_with_its_own_typed_input() {
    let plan = request_plan();
    let (tokio_rt, rt) = rt();

    let (a, b) = tokio_rt.block_on(async {
        let a = plan
            .start(
                rt.clone(),
                Request {
                    tenant: "acme".into(),
                    items: 3,
                },
            )
            .await;
        let b = plan
            .start(
                rt.clone(),
                Request {
                    tenant: "globex-inc".into(),
                    items: 5,
                },
            )
            .await;
        (a, b)
    });

    assert_eq!(a.outcome, Outcome::Ok);
    assert_eq!(b.outcome, Outcome::Ok);
    assert_eq!(
        a.output.as_deref(),
        Some(&Response {
            tenant: "acme".into(),
            total: 3 * 7 + 4,
        }),
        "the first run's answer is derived from the first run's input"
    );
    assert_eq!(
        b.output.as_deref(),
        Some(&Response {
            tenant: "globex-inc".into(),
            total: 5 * 7 + 10,
        }),
        "and the second's from the second's, from the same plan value"
    );

    // The plan value is untouched by either run: it is still `Send + Sync` and
    // still startable.
    fn is_send_sync<T: Send + Sync>(_: &T) {}
    is_send_sync(&plan);
}

/// Concurrent runs, interleaved on one runtime: the inputs do not cross.
#[test]
fn many_concurrent_runs_each_keep_their_own_input() {
    let plan = request_plan();
    let (tokio_rt, rt) = rt();
    let answers = tokio_rt.block_on(async {
        let mut running = Vec::new();
        for i in 0..16u32 {
            running.push(plan.start(
                rt.clone(),
                Request {
                    tenant: format!("t{i}"),
                    items: i,
                },
            ));
        }
        let mut out = Vec::new();
        for r in running {
            out.push(r.await);
        }
        out
    });
    for (i, r) in answers.iter().enumerate() {
        let i = i as u32;
        assert_eq!(r.outcome, Outcome::Ok);
        assert_eq!(
            r.output.as_deref(),
            Some(&Response {
                tenant: format!("t{i}"),
                total: i * 7 + format!("t{i}").len() as u32,
            }),
            "run {i} answered from run {i}'s input"
        );
    }
}

/// The instance half of the same rule: a slot holds `Arc<T>`
/// (`OD-SPAWN-INPUT`), so a body that `needs` the per-instance input receives
/// the value `cx.spawn` was handed. The root and the instance seed the same
/// kind of node the same way; if they disagreed, one of them would silently
/// never build its body.
#[test]
fn a_spawned_instance_body_receives_the_input_cx_spawn_was_given() {
    let seen = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let inner_seen = seen.clone();
    let mut t = Plan::with_input::<u8>("Link");
    let conn = t.input();
    t.step("Inner").needs(conn).run(move |_cx, v: Arc<u8>| {
        let seen = inner_seen.clone();
        async move {
            seen.store(*v as u32, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
    });
    let child = t
        .build(Policy::Isolate, Shutdown::within(secs(2)), Mode::Finite)
        .expect("a valid template");

    let mut p = Plan::builder("Mesh");
    let links = p.template("Link", &child);
    p.service("Accept")
        .spawns(&links)
        .stop_within(secs(1))
        .initialize(move |cx, ()| async move {
            let instance = cx.spawn(&links, 42u8)?;
            instance.ready().await?;
            Ok(())
        })
        .serve(|_cx, _handle| async { Ok(()) });
    let plan = p
        .build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Resident)
        .expect("valid");

    let (tokio_rt, rt) = rt();
    tokio_rt.block_on(async {
        let mut running = plan.start(rt.clone(), ());
        running.ready().await.expect("reaches steady state");
        drop(running);
        tokio::time::sleep(secs(60)).await;
    });
    assert_eq!(
        seen.load(std::sync::atomic::Ordering::SeqCst),
        42,
        "the instance's body read the input cx.spawn supplied"
    );
}

/// A plan with no input is started with `()`, and `try_start` takes the input
/// on the same footing as `start`.
#[test]
fn a_plan_with_no_input_is_started_with_unit() {
    let mut p = Plan::builder("Plain");
    p.step("Work").run(|_cx, ()| async move { Ok(1u32) });
    let plan = p
        .build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Finite)
        .expect("valid");
    let (tokio_rt, rt) = rt();
    let report = tokio_rt.block_on(async { plan.try_start(rt.clone(), ()).expect("runs").await });
    assert_eq!(report.outcome, Outcome::Ok);
}
