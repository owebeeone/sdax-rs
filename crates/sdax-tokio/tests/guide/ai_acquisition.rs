use sdax::prelude::*;
use sdax_tokio::{PlanStart, TokioRuntime};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

struct Archive(u32);

fn open_archive() -> Result<Archive, Error> {
    Ok(Archive(40))
}

#[test]
fn external_acquisition_is_inside_hold() {
    let acquisitions = Arc::new(AtomicUsize::new(0));
    let releases = Arc::new(AtomicUsize::new(0));
    let acquire_count = acquisitions.clone();
    let release_count = releases.clone();

    let mut p = Plan::with_input::<u32>("archive lookup");
    let request: Key<u32> = p.input();
    let archive: Key<Archive> = p
        .resource("archive")
        .acquire(move |cx: Cx<Acquire>, ()| {
            let acquire_count = acquire_count.clone();
            async move {
                cx.hold(|| {
                    acquire_count.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(open_archive())
                })
                .await
            }
        })
        .release(move |_cx: Cx<Release>, archive: Arc<Archive>| {
            let release_count = release_count.clone();
            async move {
                assert_eq!(archive.0, 40);
                release_count.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        });
    let answer = p.step("answer").needs((request, archive)).run(
        |_cx, values: (Arc<u32>, Arc<Archive>)| async move { Ok(*values.0 + values.1.as_ref().0) },
    );
    let plan: Plan<u32, u32> = p
        .export(answer)
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(1)),
            Mode::Finite,
        )
        .expect("valid plan");

    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("runtime");
    let runtime = Arc::new(TokioRuntime::current_thread_no_background_drain(
        tokio_rt.handle().clone(),
    ));
    let report = tokio_rt.block_on(plan.start(runtime, 2));
    assert_eq!(
        report.into_result().expect("clean run").as_deref(),
        Some(&42)
    );
    assert_eq!(acquisitions.load(Ordering::SeqCst), 1);
    assert_eq!(releases.load(Ordering::SeqCst), 1);
}
