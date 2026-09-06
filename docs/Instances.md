# Instances

A **template** is a nested plan you instantiate at run time with
`cx.spawn(&template, input)`. Write the child with `Plan::template::<In>`
— the same constructor as `Plan::with_input` — register it on the
parent with `.template(name, &plan)`, and declare which service may
spawn it with `.spawns(&template)`. The built plan can also be started
as a root with `start(rt, input)`.

Two facts that surprise people:

1. Every key an instance **imports** is released only after that
   instance has ended. The parent's resource outlives the child.
2. A start body that awaits `Child::ready()` makes the parent's
   readiness include that instance. Dependents of the service then
   wait for the child too.

`Child` has `ready`, `stop`, and `id`. It is not a `needs` edge and it
carries no typed output to the parent.

`cx.spawn` can refuse. The body sees the error; it does not reach the
engine as a panic.

| Refusal | Meaning |
|---|---|
| `ForeignTemplate` | the template belongs to another plan |
| `UndeclaredTemplate` | this node did not declare `spawns(&template)` |
| `ScopeStopping` | the scope is settling and admits no new instances |

A template must not import the spawning service's own key — that is a
readiness deadlock, and `build` rejects it (`V-SPAWN-SELF-IMPORT`).
Import a resource the service needs instead.

`Mode::Finite` is rejected on a plan that declares a template. The
parent below is `Resident`. The child is `Finite`: one resource that
imports the parent's endpoint, then it ends. The parent's release
asserts the child already released — the imported key outlived the
instance.

```rust,guide:spawn_child
use sdax::prelude::*;
use sdax_tokio::{PlanStart, TokioRuntime};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

struct Endpoint;
struct Sock;

#[test]
fn a_service_spawns_a_child_that_releases_before_the_import() {
    let parent_released = Arc::new(AtomicBool::new(false));
    let child_released = Arc::new(AtomicBool::new(false));
    let child_flag = child_released.clone();
    let parent_flag = parent_released.clone();
    let saw_child_first = Arc::new(AtomicBool::new(false));
    let order = saw_child_first.clone();

    let mut p = Plan::builder("Mesh");
    let endpoint = p
        .resource("Endpoint")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Endpoint)) })
        .release(move |_cx, _e: Arc<Endpoint>| {
            order.store(child_flag.load(Ordering::SeqCst), Ordering::SeqCst);
            parent_flag.store(true, Ordering::SeqCst);
            async move { Ok(()) }
        });

    let mut t = Plan::template::<u8>("Link");
    let imported = t.import(endpoint);
    let cr = child_released.clone();
    t.resource("Sock")
        .needs(imported)
        .acquire(|cx, _e: Arc<Endpoint>| async move { Ok(cx.hold_value(Sock)) })
        .release(move |_cx, _s: Arc<Sock>| {
            cr.store(true, Ordering::SeqCst);
            async move { Ok(()) }
        });
    let child = t
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(2)),
            Mode::Finite,
        )
        .expect("valid template");

    let link = p.template("Link", &child);
    p.service("Accept")
        .needs(endpoint)
        .spawns(&link)
        .stop_within(Duration::from_secs(1))
        .start(move |cx, _e: Arc<Endpoint>| async move {
            let ch = cx.spawn(&link, 1u8)?;
            ch.ready().await?;
            Ok(Serving::new((), async { Ok(()) }))
        });
    let plan = p
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(10)),
            Mode::Resident,
        )
        .expect("valid");

    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .expect("runtime");
    let rt = Arc::new(TokioRuntime::current_thread_no_background_drain(
        tokio_rt.handle().clone(),
    ));
    let report = tokio_rt.block_on(async {
        let mut running = plan.start(rt.clone(), ());
        running.ready().await.expect("steady");
        running.shutdown();
        running.await
    });
    assert_eq!(report.outcome, Outcome::Ok);
    assert!(report.is_clean());
    assert!(child_released.load(Ordering::SeqCst));
    assert!(parent_released.load(Ordering::SeqCst));
    assert!(
        saw_child_first.load(Ordering::SeqCst),
        "imported key released only after the instance ended"
    );
}
```
