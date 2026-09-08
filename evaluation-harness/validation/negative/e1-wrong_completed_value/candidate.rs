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
    let mut p = Plan::with_input::<u32>("pipeline");
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
    let a = env.clone();
    let z = env.clone();
    let derived = p
        .resource("derived")
        .needs(base)
        .acquire(move |cx, b: Arc<Resource>| {
            let e = a.clone();
            async move {
                cx.hold(|| async move { e.open("derived", b.value + 5, &[&b]).await })
                    .await
            }
        })
        .release(move |_, v| {
            let e = z.clone();
            async move { e.close(&v).await }
        });
    let output = p
        .step("completed")
        .needs(derived)
        .run(|_, v: Arc<Resource>| async move { Ok(v.value * 3) });
    finish(p.export(output))
}
