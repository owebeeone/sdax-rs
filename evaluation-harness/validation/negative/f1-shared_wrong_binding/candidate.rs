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
    let mut c = Plan::with_input::<u32>("encoder");
    let bias = c.input();
    let tens = c.port::<Resource>("tens");
    let units = c.port::<Resource>("units");
    let a = env.clone();
    let z = env.clone();
    let encoded = c
        .resource("encoded")
        .needs((tens, units, bias))
        .acquire(
            move |cx, (t, u, b): (Arc<Resource>, Arc<Resource>, Arc<u32>)| {
                let e = a.clone();
                async move {
                    cx.hold(|| async move {
                        e.open(
                            &format!("slot-{}", *b),
                            10 * t.value + u.value + *b,
                            &[&t, &u],
                        )
                        .await
                    })
                    .await
                }
            },
        )
        .release(move |_, r| {
            let e = z.clone();
            async move { e.close(&r).await }
        });
    let out = c
        .step("digit")
        .needs(encoded)
        .run(|_, v: Arc<Resource>| async move { Ok(v.value) });
    let c = finish(c.export(out));
    let mut p = Plan::with_input::<u32>("mirror");
    let input = p.input();
    let a = env.clone();
    let z = env.clone();
    let west = p
        .resource("west")
        .needs(input)
        .acquire(move |cx, n: Arc<u32>| {
            let e = a.clone();
            async move {
                cx.hold(|| async move { e.open("west", *n + 1, &[]).await })
                    .await
            }
        })
        .release(move |_, r| {
            let e = z.clone();
            async move { e.close(&r).await }
        });
    let a = env.clone();
    let z = env.clone();
    let east = p
        .resource("east")
        .needs(input)
        .acquire(move |cx, n: Arc<u32>| {
            let e = a.clone();
            async move {
                cx.hold(|| async move { e.open("east", *n + 4, &[]).await })
                    .await
            }
        })
        .release(move |_, r| {
            let e = z.clone();
            async move { e.close(&r).await }
        });
    let b2 = p.step("two").run(|_, ()| async { Ok(2u32) });
    let b5 = p.step("five").run(|_, ()| async { Ok(5u32) });
    let low = c.bind(tens, west).unwrap().bind(units, east).unwrap();
    let high = c.bind(tens, west).unwrap().bind(units, east).unwrap();
    let l = p.component("low", &low, b2);
    let h = p.component("high", &high, b5);
    let out = p
        .step("answer")
        .needs((l, h))
        .run(|_, (l, h): (Arc<u32>, Arc<u32>)| async move { Ok(*l * 1000 + *h) });
    finish(p.export(out))
}
