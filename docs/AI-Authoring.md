# AI authoring reference

A plan is an immutable lifecycle declaration. Build it once; each start or mount
has separate run state. Every declaration must reach its terminal method before
build: resources end in release, effects in compensate or persistent, and
services in serve. A finished declaration is a Key, not another builder.

For a complete starting plan, use the [starter templates](StarterTemplates.md).

## Typed values and reports

Plan<Out, In> takes In at start, mount, or spawn and may export completed Out.
Plan::builder means In = (). Call p.export(output_key) before build; a last step
is not exported automatically.

Key<T> is a declaration handle. A body needing it receives Arc<T>.
.needs((a, b)) supplies (Arc<A>, Arc<B>); one Key<(A, B)> supplies Arc<(A, B)>.
Step bodies return Result<T, Error>; their keys carry T. Resource acquisition
and effect performance return Result<Held<T>, Error>, with Held created only by
the single-use Cx<Acquire>.

Start with use sdax_tokio::PlanStart;, await the report, then use
report.into_result(). Its success is Option<Arc<Out>>; its error is the full
typed Report, including faults, cleanup failures, incomplete work, and ambiguous
operations. Keep that report typed internally. At a boundary requiring String,
use report.into_result().map_err(|report| report.to_string()); textual rendering
cannot preserve downcasting. When output is required, use into_required_output():
it returns Arc<Out> or RequiredOutputError::Failed / MissingOutput, each retaining
the original report through report() and into_report().

## External acquisition, typed input, and completed output

Put the external action inside the lazy cx.hold factory. Do not perform it first
and then call hold_value. The acquisition and release closures are both move, so
prepare separate clones for their captures. Release receives the registered value
as Arc<T>. The hold factory returns a future yielding Result<T, Error>. If
your API already returns that future, return it directly rather than wrapping
it in another ready or unfinished async block.

~~~rust,guide:ai_acquisition
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
~~~

Retry::attempts(n) means at most n total attempts, not n retries after the first.
Declare retry on the resource or effect and add .idempotent(); never loop around
or clone an acquisition context.

## Formal resource imports and ordinary input

A child input is ordinary per-mount data. A parent-owned resource crosses the
boundary through child.port::<T>(name). Inside the child it is a Key<T> named in
.needs(...). Bind it with child.bind(port, parent_resource_key) before component.
Binding creates the visible lifetime edge that keeps the parent resource alive
through child cleanup. Bind the original child definition again when another
mount needs a different parent resource.

~~~rust,guide:ai_component
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
        report.into_result().expect("clean run").as_deref(),
        Some(&13)
    );
}
~~~

A struct or tuple containing Arc<Resource> is only data; hiding a resource inside
it does not declare cleanup ownership. Keep resource keys explicit.
child.import(parent_key) is the concrete alternative when intentionally
constructing a child for one already-known parent.

## Identified unknown recovery

For an effect that may have happened without a receipt, declare a stable identity
before perform. Configuration with .needs(...), .within(...), .retry(...) and
.idempotent() can go before or after .identified_by(identity_key). The perform body receives
(ordinary_dependencies, Arc<Identity>); recover_unknown receives Arc<Identity>
alone. Recovery must still end in compensate or persistent.

~~~rust,guide:ai_recovery
use sdax::prelude::*;
use sdax_tokio::{PlanStart, TokioRuntime};
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct Destination(&'static str);

#[test]
fn unknown_operation_recovers_by_its_declared_identity() {
    let recovered = Arc::new(Mutex::new(Vec::new()));
    let record = recovered.clone();

    let mut p = Plan::with_input::<u64>("thumbnail publication");
    let operation: Key<u64> = p.input();
    let destination = p
        .step("destination")
        .run(|_cx, ()| async move { Ok(Destination("preview")) });
    p.effect("publish thumbnail")
        .needs(destination)
        .idempotent()
        .within(Duration::from_millis(5))
        .on_ambiguous(Ambiguity::Recover)
        .identified_by(operation)
        .perform(
            |cx, (destination, operation): (Arc<Destination>, Arc<u64>)| async move {
                cx.hold(|| async move {
                    let _request = (destination.0, *operation);
                    std::future::pending::<Result<u64, Error>>().await
                })
                .await
            },
        )
        .recover_unknown(move |_cx, operation: Arc<u64>| {
            let record = record.clone();
            async move {
                record.lock().expect("record").push(*operation);
                Ok(Recovery::Resolved)
            }
        })
        .persistent();
    let plan: Plan<(), u64> = p
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_millis(20)),
            Mode::Finite,
        )
        .expect("valid plan");

    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .expect("runtime");
    let runtime = Arc::new(TokioRuntime::current_thread_no_background_drain(
        tokio_rt.handle().clone(),
    ));
    let report = tokio_rt.block_on(plan.start(runtime, 73));

    assert_eq!(*recovered.lock().expect("record"), [73]);
    assert!(report.ambiguous.is_empty());
    assert!(report.cleanup_failures.is_empty());
}
~~~

Recovery::Resolved discharges uncertainty but does not erase the original timeout
fault. Recovery::StillUnknown, recovery error,
panic, or timeout leaves an ambiguous report entry. A known Err from perform is
a normal fault and is not an unknown outcome. Put operation IDs in domain errors
when their text must include them; arbitrary identity values are not generically
printable.

## Resident service lifecycle

Initialization publishes one stable Arc<H> when it succeeds; Retry bounds
initialization attempts. Restart applies only to later serving episodes and
does not rerun a successful initializer or its dependents. cx.episode() is
one-based. cx.stop().await waits for cooperative shutdown; the owner requests it
through running.shutdown(). Use cx.spawn only for declared templates, never an
untracked task.

~~~rust,guide:ai_service
use sdax::prelude::*;
use sdax_tokio::{PlanStart, TokioRuntime};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct WatchHandle(u64);

#[test]
fn resident_service_initializes_restarts_and_stops() {
    let initializations = Arc::new(AtomicUsize::new(0));
    let episodes = Arc::new(AtomicUsize::new(0));
    let handles = Arc::new(Mutex::new(Vec::new()));

    let mut p = Plan::builder("index monitor");
    p.service("watch index")
        .idempotent()
        .restart(Restart::on_error(Backoff::fixed(Duration::from_secs(1))).max(1))
        .stop_within(Duration::from_secs(1))
        .initialize({
            let initializations = initializations.clone();
            move |_cx, ()| {
                initializations.fetch_add(1, Ordering::SeqCst);
                async { Ok(WatchHandle(91)) }
            }
        })
        .serve({
            let episodes = episodes.clone();
            let handles = handles.clone();
            move |cx, handle: Arc<WatchHandle>| {
                episodes.store(cx.episode() as usize, Ordering::SeqCst);
                handles
                    .lock()
                    .expect("handles")
                    .push((Arc::as_ptr(&handle) as usize, handle.0));
                async move {
                    if cx.episode() == 1 {
                        Err::<(), Error>("index changed".into())
                    } else {
                        cx.stop().await;
                        Ok(())
                    }
                }
            }
        });
    let plan: Plan = p
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(5)),
            Mode::Resident,
        )
        .expect("valid plan");

    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .expect("runtime");
    let runtime = Arc::new(TokioRuntime::current_thread_no_background_drain(
        tokio_rt.handle().clone(),
    ));
    let report = tokio_rt.block_on(async {
        let mut running = plan.start(runtime, ());
        running.ready().await.expect("ready");
        tokio::time::sleep(Duration::from_secs(2)).await;
        running.shutdown();
        running.await
    });

    assert_eq!(initializations.load(Ordering::SeqCst), 1);
    assert_eq!(episodes.load(Ordering::SeqCst), 2);
    let handles = handles.lock().expect("handles");
    assert_eq!(handles.len(), 2);
    assert_eq!(handles[0], handles[1]);
    assert!(report.is_clean());
}
~~~
