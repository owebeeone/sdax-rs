use sdax::prelude::*;
use sdax_tokio::{PlanStart, TokioRuntime};
use std::sync::Arc;
use std::time::Duration;

struct Palette(u32);

#[derive(Clone, Copy)]
struct Stroke(u32);

fn build<O, I>(builder: PlanBuilder<O, I>) -> Plan<O, I> {
    builder
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(1)),
            Mode::Finite,
        )
        .expect("valid plan")
}

#[test]
fn ordinary_input_is_separate_from_a_formal_resource_port() {
    let mut child = Plan::with_input::<Stroke>("renderer");
    let stroke: Key<Stroke> = child.input();
    let palette_port: Key<Palette> = child.port::<Palette>("palette");
    let pixels = child.step("render").needs((stroke, palette_port)).run(
        |_cx, values: (Arc<Stroke>, Arc<Palette>)| async move {
            Ok(values.0.as_ref().0 + values.1.as_ref().0)
        },
    );
    let child: Plan<u32, Stroke> = build(child.export(pixels));

    let mut parent = Plan::builder("drawing");
    let palette: Key<Palette> = parent
        .resource("palette")
        .acquire(|cx: Cx<Acquire>, ()| async move { Ok(cx.hold_value(Palette(10))) })
        .release(|_cx: Cx<Release>, _palette: Arc<Palette>| async move { Ok(()) });
    let stroke = parent
        .step("stroke input")
        .run(|_cx, ()| async move { Ok(Stroke(3)) });
    let bound_child = child
        .bind(palette_port, palette)
        .expect("formal import binding");
    let rendered = parent.component("rendered layer", &bound_child, stroke);
    let plan: Plan<u32> = build(parent.export(rendered));

    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("runtime");
    let runtime = Arc::new(TokioRuntime::current_thread_no_background_drain(
        tokio_rt.handle().clone(),
    ));
    let report = tokio_rt.block_on(plan.start(runtime, ()));
    assert_eq!(
        *report.into_required_output().expect("completed output"),
        13
    );
}
