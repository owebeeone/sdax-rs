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
    let mut leaf = Plan::with_input::<u32>("projection");
    let li = leaf.input();
    let lp = leaf.port::<Resource>("sleeve");
    let a = env.clone();
    let z = env.clone();
    let lens = leaf
        .resource("lens")
        .needs((lp, li))
        .acquire(move |cx, (s, n): (Arc<Resource>, Arc<u32>)| {
            let e = a.clone();
            async move {
                cx.hold(|| async move { e.open("lens", s.value * 2 + *n, &[&s]).await })
                    .await
            }
        })
        .release(move |_, r| {
            let e = z.clone();
            async move { e.close(&r).await }
        });
    let out = leaf
        .step("out")
        .needs(lens)
        .run(|_, r: Arc<Resource>| async move { Ok(r.value) });
    let leaf = finish(leaf.export(out));
    let mut mid = Plan::with_input::<(u32, u32)>("middle");
    let mi = mid.input();
    let mp = mid.port::<Resource>("vault");
    let a = env.clone();
    let z = env.clone();
    let sleeve = mid
        .resource("sleeve")
        .needs((mp, mi))
        .acquire(move |cx, (v, n): (Arc<Resource>, Arc<(u32, u32)>)| {
            let e = a.clone();
            async move {
                cx.hold(|| async move { e.open("sleeve", v.value + n.0, &[&v]).await })
                    .await
            }
        })
        .release(move |_, r| {
            let e = z.clone();
            async move { e.close(&r).await }
        });
    let scalar = mid
        .step("scalar")
        .needs(mi)
        .run(|_, n: Arc<(u32, u32)>| async move { Ok(n.1) });
    let bound = leaf.bind(lp, sleeve).unwrap();
    let out = mid.component("leaf", &bound, scalar);
    let mid = finish(mid.export(out));
    let mut p = Plan::with_input::<u32>("root");
    let input = p.input();
    let a = env.clone();
    let z = env.clone();
    let vault = p
        .resource("vault")
        .needs(input)
        .acquire(move |cx, n: Arc<u32>| {
            let e = a.clone();
            async move {
                cx.hold(|| async move { e.open("vault", 3 * *n, &[]).await })
                    .await
            }
        })
        .release(move |_, r| {
            let e = z.clone();
            async move { e.close(&r).await }
        });
    let pair = p
        .step("pair")
        .needs(input)
        .run(|_, n: Arc<u32>| async move { Ok((*n + 2, *n + 5)) });
    let bound = mid.bind(mp, vault).unwrap();
    let out = p.component("middle", &bound, pair);
    finish(p.export(out))
}
