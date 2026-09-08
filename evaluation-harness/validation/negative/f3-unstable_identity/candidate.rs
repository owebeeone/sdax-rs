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
    let mut p = Plan::with_input::<u32>("transport");
    let input = p.input();
    let a = env.clone();
    let z = env.clone();
    let permit = p
        .resource("permit")
        .needs(input)
        .acquire(move |cx, n: Arc<u32>| {
            let e = a.clone();
            async move {
                cx.hold(|| async move { e.open("permit", *n, &[]).await })
                    .await
            }
        })
        .release(move |_, r| {
            let e = z.clone();
            async move { e.close(&r).await }
        });
    let identity = p
        .step("identity")
        .needs(input)
        .run(|_, n: Arc<u32>| async move { Ok(*n * 100 + 17) });
    let a = env.clone();
    let r = env.clone();
    let z = env.clone();
    p.effect("send")
        .needs(permit)
        .idempotent()
        .retry(Retry::attempts(3).backoff(Backoff::fixed(Duration::from_millis(1))))
        .within(Duration::from_millis(4))
        .on_ambiguous(Ambiguity::Recover)
        .identified_by(identity)
        .perform(move |cx, (permit, id): (Arc<Resource>, Arc<u32>)| {
            let e = a.clone();
            let attempt = cx.attempt();
            async move {
                cx.hold(|| async move {
                    e.record(format!(
                        "send:{attempt}:{}:{}:{}",
                        *id + attempt,
                        permit.value,
                        Arc::as_ptr(&id) as usize
                    ));
                    if attempt < 3 {
                        return Err("known rejection".into());
                    }
                    std::future::pending::<Result<u32, Error>>().await
                })
                .await
            }
        })
        .recover_unknown(move |cx, id| {
            let e = r.clone();
            async move {
                e.record(format!("recover:{}:{}", cx.attempt(), *id));
                if e.live_count() != 1 {
                    return Err("permit not live".into());
                }
                e.record("permit-live-at-recovery");
                let audit = e.open("audit", *id, &[]).await?;
                e.close(&audit).await?;
                Ok(Recovery::Resolved)
            }
        })
        .compensate(move |_, receipt| {
            let e = z.clone();
            async move {
                e.record(format!("compensate:{}", *receipt));
                Ok(())
            }
        });
    let out = p
        .step("completed")
        .needs(input)
        .run(|_, n: Arc<u32>| async move { Ok(*n + 1) });
    finish(p.export(out))
}
