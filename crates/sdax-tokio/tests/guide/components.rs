use sdax::prelude::*;
use sdax_tokio::{PlanStart, TokioRuntime};
use std::sync::Arc;
use std::time::Duration;

fn build<O, I>(p: PlanBuilder<O, I>) -> Plan<O, I> {
    p.build(
        Policy::FailFast,
        Shutdown::within(Duration::from_secs(1)),
        Mode::Finite,
    )
    .expect("valid plan")
}

#[test]
fn repeated_component_mounts_bind_independent_inputs() {
    let mut child = Plan::with_input::<u32>("double");
    let input = child.input();
    let doubled = child
        .step("double")
        .needs(input)
        .run(|_, value: Arc<u32>| async move { Ok(*value * 2) });
    let child = build(child.export(doubled));

    let mut parent = Plan::with_input::<u32>("parent");
    let input = parent.input();
    let offset = parent
        .step("offset")
        .needs(input)
        .run(|_, value: Arc<u32>| async move { Ok(*value + 10) });
    let left = parent.component("left", &child, input);
    let right = parent.component("right", &child, offset);
    let answer = parent
        .step("answer")
        .needs((left, right))
        .run(|_, values: (Arc<u32>, Arc<u32>)| async move { Ok((*values.0, *values.1)) });
    let parent = build(parent.export(answer));

    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("runtime");
    let rt = Arc::new(TokioRuntime::current_thread_no_background_drain(
        tokio_rt.handle().clone(),
    ));
    let report = tokio_rt.block_on(parent.start(rt, 3));
    assert_eq!(
        report.into_result().expect("clean run").as_deref(),
        Some(&(6, 26))
    );
}
