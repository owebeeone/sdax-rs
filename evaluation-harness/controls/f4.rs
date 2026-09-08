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
    let mut c = Plan::with_input::<u32>("dispatch_scope");
    let id = c.input();
    let a = env.clone();
    let r = env.clone();
    let z = env.clone();
    c.effect("dispatch")
        .idempotent()
        .within(Duration::from_millis(3))
        .on_ambiguous(Ambiguity::Recover)
        .identified_by(id)
        .perform(move |cx, ((), id)| {
            let e = a.clone();
            async move {
                cx.hold(|| async move {
                    if *id % 2 == 0 {
                        e.record(format!("ack:{}", *id));
                        Ok(*id + 900)
                    } else {
                        e.record(format!("unknown:{}", *id));
                        std::future::pending::<Result<u32, Error>>().await
                    }
                })
                .await
            }
        })
        .recover_unknown(move |_, id| {
            let e = r.clone();
            async move {
                e.record(format!("reconcile:{}", *id));
                Ok(Recovery::StillUnknown)
            }
        })
        .compensate(move |_, receipt| {
            let e = z.clone();
            async move {
                e.record(format!("undo:{}", *receipt));
                Ok(())
            }
        });
    let out = c.step("completed").run(|_, ()| async { Ok(1u32) });
    let c = finish(c.export(out));
    let mut p = Plan::with_input::<u32>("dual");
    let input = p.input();
    let even = p
        .step("even")
        .needs(input)
        .run(|_, n: Arc<u32>| async move { Ok(*n * 2) });
    let odd = p
        .step("odd")
        .needs(input)
        .run(|_, n: Arc<u32>| async move { Ok(*n * 2 + 1) });
    p.component("delivered", &c, even);
    p.component("pending", &c, odd);
    let out = p
        .step("root_output")
        .needs(input)
        .run(|_, n: Arc<u32>| async move { Ok(*n) });
    finish(p.export(out))
}
