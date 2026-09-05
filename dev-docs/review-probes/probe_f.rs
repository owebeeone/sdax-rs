//! Substrate review probe (F): a body whose poll returns Ready during the very
//! poll in which the abort is requested (multi-thread). Throwaway.
use sdax::host::{Observer, Runtime};
use sdax::*;
use sdax_testkit::TraceRecorder;
use sdax_tokio::{PlanStart, TokioRuntime};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}
struct Unit;

struct SlowOk {
    cx: Cx<Acquire>,
    polls: u32,
}
impl Future for SlowOk {
    type Output = Result<Held<Unit>, Error>;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.polls += 1;
        if self.polls == 1 {
            let w = cx.waker().clone();
            std::thread::spawn(move || {
                std::thread::sleep(ms(5));
                w.wake();
            });
            return Poll::Pending;
        }
        let h = self.cx.hold_value(Unit);
        std::thread::sleep(ms(40));
        Poll::Ready(Ok(h))
    }
}

#[test]
fn f1_outcome_due_in_the_poll_the_abort_lands_in() {
    let mut hist: Vec<(String, u32)> = Vec::new();
    let mut problems = Vec::new();
    let tokio_rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_time()
        .build()
        .expect("rt");
    for i in 0..20 {
        let released = Arc::new(AtomicUsize::new(0));
        let rl = released.clone();
        let mut p = Plan::builder("SlowOk");
        p.resource("R")
            .acquire(|cx, ()| SlowOk { cx, polls: 0 })
            .release(move |_cx, _u| {
                let rl = rl.clone();
                async move {
                    rl.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            });
        let plan = p
            .build(Policy::FailFast, Shutdown::unbounded(), Mode::Resident)
            .expect("valid");
        let rec = Arc::new(TraceRecorder::new());
        let obs: Arc<dyn Observer> = rec.clone();
        let rt = Arc::new(TokioRuntime::new(tokio_rt.handle().clone()).with_observer(obs));
        let record = Arc::new(std::sync::Mutex::new(sdax_tokio::RunRecord::default()));
        let report = tokio_rt.block_on(async {
            let running = plan.start_with(
                rt.clone(),
                sdax_tokio::RunOptions::new().record(record.clone()),
            );
            let h = running.handle();
            rt.spawn(Box::pin(async move {
                tokio::time::sleep(ms(10 + i)).await;
                h.cancel();
            }));
            tokio::time::timeout(ms(5000), running).await
        })
        .expect("ended");
        let left = tokio_rt.block_on(rt.shutdown(ms(2000)));
        let rej = record.lock().unwrap().rejections.clone();
        let kinds: Vec<String> = rec
            .trace()
            .events
            .iter()
            .filter(|e| e.node.is_some())
            .map(|e| format!("{:?}", e.kind))
            .collect();
        let key = kinds.join(" > ");
        match hist.iter_mut().find(|(k, _)| *k == key) {
            Some((_, n)) => *n += 1,
            None => hist.push((key, 1)),
        }
        if released.load(Ordering::SeqCst) != 1 || left != Ok(()) || !rej.is_empty() {
            problems.push(format!(
                "iter {i}: released {} shutdown {left:?} rej {rej:?} outcome {:?}",
                released.load(Ordering::SeqCst),
                report.outcome
            ));
        }
    }
    println!("F1 node-event sequences over 20 runs:");
    for (k, n) in &hist {
        println!("  {n}x  {k}");
    }
    println!("F1 problems: {problems:?}");
    tokio_rt.shutdown_background();
}
