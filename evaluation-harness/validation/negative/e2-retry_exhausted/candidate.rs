use crate::support::Resource;
use crate::support::*;
use sdax::prelude::*;
use std::sync::Arc;
use std::time::Duration;
fn finish<O, I>(p: PlanBuilder<O, I>) -> Plan<O, I>
where
    O: Send + Sync + 'static,
    I: Send + Sync + 'static,
{
    p.build(
        Policy::Isolate,
        Shutdown::within(Duration::from_millis(100)),
        Mode::Finite,
    )
    .unwrap()
}
pub fn boundary(report: Report<u32>) -> Result<u32, String> {
    report
        .into_result()
        .map_err(|r| r.to_string())?
        .map(|v| *v)
        .ok_or_else(|| "missing output".into())
}
pub fn build(env: Env) -> Plan<u32, u32> {
    let mut c = Plan::with_input::<u32>("pricing");
    let input = c.input();
    let imported = c.port::<Resource>("base");
    let a = env.clone();
    let z = env.clone();
    let rates = c
        .resource("rates")
        .needs((imported, input))
        .idempotent()
        .retry(Retry::attempts(1).backoff(Backoff::fixed(Duration::from_millis(1))))
        .acquire(move |cx, (b, n): (Arc<Resource>, Arc<u32>)| {
            let e = a.clone();
            let attempt = cx.attempt();
            async move {
                cx.hold(|| async move {
                    e.record(format!("attempt:rates:{attempt}"));
                    if attempt == 1 {
                        return Err("transient rates".into());
                    }
                    e.open("rates", *n, &[&b]).await
                })
                .await
            }
        })
        .release(move |_, v| {
            let e = z.clone();
            async move { e.close(&v).await }
        });
    let a = env.clone();
    let z = env.clone();
    let derived = c
        .resource("derived")
        .needs((imported, rates))
        .acquire(move |cx, (b, r): (Arc<Resource>, Arc<Resource>)| {
            let e = a.clone();
            async move {
                cx.hold(|| async move { e.open("derived", b.value + r.value, &[&b, &r]).await })
                    .await
            }
        })
        .release(move |_, v| {
            let e = z.clone();
            async move { e.close(&v).await }
        });
    let out = c
        .step("completed")
        .needs(derived)
        .run(|_, v: Arc<Resource>| async move { Ok(v.value) });
    let c = finish(c.export(out));
    let mut p = Plan::with_input::<u32>("parent");
    let input = p.input();
    let a = env.clone();
    let z = env.clone();
    let base = p
        .resource("base")
        .needs(input)
        .acquire(move |cx, n: Arc<u32>| {
            let e = a.clone();
            async move {
                cx.hold(|| async move { e.open("base", *n, &[]).await })
                    .await
            }
        })
        .release(move |_, v| {
            let e = z.clone();
            async move { e.close(&v).await }
        });
    let offset = p.step("offset").run(|_, ()| async { Ok(7u32) });
    let bound = c.bind(imported, base).unwrap();
    let out = p.component("priced", &bound, offset);
    finish(p.export(out))
}
