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
    let mut leaf = Plan::with_input::<u32>("leaf");
    let input = leaf.input();
    let port = leaf.port::<Resource>("trunk");
    let a = env.clone();
    let z = env.clone();
    let red = leaf
        .resource("red")
        .needs((port, input))
        .acquire(move |cx, (t, n): (Arc<Resource>, Arc<u32>)| {
            let e = a.clone();
            async move {
                cx.hold(|| async move { e.open("red", *n + 1, &[&t]).await })
                    .await
            }
        })
        .release(move |_, r| {
            let e = z.clone();
            async move { e.close(&r).await }
        });
    let a = env.clone();
    let z = env.clone();
    let blue = leaf
        .resource("blue")
        .needs((port, input))
        .acquire(move |cx, (t, n): (Arc<Resource>, Arc<u32>)| {
            let e = a.clone();
            async move {
                cx.hold(|| async move { e.open("blue", *n + 2, &[&t]).await })
                    .await
            }
        })
        .release(move |_, r| {
            let e = z.clone();
            async move { e.close(&r).await }
        });
    let out = leaf.step("aggregate").needs((red, blue)).run(
        |_, _: (Arc<Resource>, Arc<Resource>)| async {
            Err::<u32, Error>(Box::new(Cause(
                "aggregation refused",
                Some(Box::new(Cause(
                    "checksum mismatch",
                    Some(Box::new(Cause("sector 19", None))),
                ))),
            )))
        },
    );
    let leaf = finish(leaf.export(out));
    let mut mid = Plan::with_input::<u32>("middle");
    let input = mid.input();
    let mp = mid.port::<Resource>("trunk");
    let bound = leaf.bind(port, mp).unwrap();
    let out = mid.component("shard", &bound, input);
    let mid = finish(mid.export(out));
    let mut p = Plan::with_input::<u32>("root");
    let input = p.input();
    let a = env.clone();
    let z = env.clone();
    let trunk = p
        .resource("trunk")
        .needs(input)
        .acquire(move |cx, n: Arc<u32>| {
            let e = a.clone();
            async move {
                cx.hold(|| async move { e.open("trunk", *n, &[]).await })
                    .await
            }
        })
        .release(move |_, r| {
            let e = z.clone();
            async move { e.close(&r).await }
        });
    let bound = mid.bind(mp, trunk).unwrap();
    let out = p.component("session", &bound, input);
    finish(p.export(out))
}
