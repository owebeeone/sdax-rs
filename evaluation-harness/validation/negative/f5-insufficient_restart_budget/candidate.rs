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
    let mut p = Plan::with_input::<u32>("resident");
    let input = p.input();
    let a = env.clone();
    let z = env.clone();
    let cable = p
        .resource("cable")
        .needs(input)
        .acquire(move |cx, n: Arc<u32>| {
            let e = a.clone();
            async move {
                cx.hold(|| async move { e.open("cable", *n, &[]).await })
                    .await
            }
        })
        .release(move |_, r| {
            let e = z.clone();
            async move { e.close(&r).await }
        });
    let a = env.clone();
    let s = env.clone();
    let relay = p
        .service("relay")
        .needs(cable)
        .idempotent()
        .retry(Retry::attempts(2).backoff(Backoff::fixed(Duration::from_millis(1))))
        .restart(Restart::on_error(Backoff::fixed(Duration::from_millis(40))).max(1))
        .stop_within(Duration::from_millis(10))
        .initialize(move |cx, c: Arc<Resource>| {
            let e = a.clone();
            async move {
                e.record(format!("init:{}", cx.attempt()));
                if cx.attempt() == 1 {
                    Err("initial connection".into())
                } else {
                    Ok(c.value + 6)
                }
            }
        })
        .serve(move |cx, h: Arc<u32>| {
            let e = s.clone();
            async move {
                e.record(format!(
                    "serve:{}:{}:{}",
                    cx.episode(),
                    *h,
                    Arc::as_ptr(&h) as usize
                ));
                if cx.episode() < 3 {
                    Err("episode lost".into())
                } else {
                    cx.stop().await;
                    e.record("cooperatively-stopped");
                    Ok(())
                }
            }
        });
    let o = env.clone();
    let out = p.step("observer").needs(relay).run(move |_, h: Arc<u32>| {
        let e = o.clone();
        async move {
            e.record(format!("observer:{}:{}", *h, Arc::as_ptr(&h) as usize));
            Ok(*h)
        }
    });
    p.export(out)
        .build(
            Policy::Isolate,
            Shutdown::within(Duration::from_millis(100)),
            Mode::Resident,
        )
        .unwrap()
}
