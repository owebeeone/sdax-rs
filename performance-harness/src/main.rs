mod alloc;
mod fixtures;
mod measure;
mod tokio_lifecycle;

use fixtures::*;
use measure::{allocated, timed, Sample};
use sdax::host::engine::{Effect, Event, Machine};
use sdax::host::{bodies_of_with_input, Observer};
use sdax::{Outcome, Phase, Plan, TraceKind};
use sdax_tokio::{PlanStart, TokioRuntime};
use std::collections::VecDeque;
use std::future::Future;
use std::hint::black_box;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

#[global_allocator]
static ALLOCATOR: alloc::CountingAllocator = alloc::CountingAllocator;

#[derive(Clone, Copy)]
struct Config {
    samples: usize,
    warmup: usize,
    build_samples: usize,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("verify") => verify(),
        Some("bench") => {
            verify();
            let cfg = Config {
                samples: value(&args, "--samples", 40),
                warmup: value(&args, "--warmup", 8),
                build_samples: value(&args, "--build-samples", 20),
            };
            measure::write_csv(&bench(cfg));
        }
        Some("allocation-probe") => allocation_probe(),
        Some("resident-probe") => resident_probe(value(&args, "--seconds", 10)),
        _ => {
            eprintln!(
                "usage: sdax-performance-harness verify | bench [--samples N] [--warmup N] [--build-samples N] | allocation-probe | resident-probe [--seconds N]"
            );
            std::process::exit(2);
        }
    }
}

fn allocation_probe() {
    println!("workload,mounts,allocations,allocated_bytes,checksum");
    for mounts in [2, 10, 100] {
        let releases = Arc::new(AtomicU64::new(0));
        let capture = || {
            allocated(|| {
                let plan = repeated_resource_components(mounts, releases.clone());
                let checksum = plan.name().len() as u64;
                (plan, checksum)
            })
        };
        let expected = capture();
        for _ in 1..5 {
            assert_eq!(
                capture(),
                expected,
                "allocation probe must be deterministic"
            );
        }
        println!(
            "component_repeated_resource,{mounts},{},{},{}",
            expected.0, expected.1, expected.2
        );
    }
}

fn resident_probe(seconds: usize) {
    let plan = resident_noop();
    let executor = runtime();
    let rt = adapter(&executor, None);
    let report = executor.block_on(async {
        let mut running = plan.start(rt.clone(), 1);
        running.ready().await.expect("resident probe ready");
        tokio::time::sleep(std::time::Duration::from_secs(seconds as u64)).await;
        running.shutdown();
        running.await
    });
    assert_eq!(report.outcome, Outcome::Ok);
    assert!(report.is_clean(), "{report}");
    assert_eq!(rt.tracked(), 0);
    println!(
        "resident_probe_seconds={},outcome=ok,cleanup_failures=0,tracked_tasks=0",
        seconds
    );
}

fn value(args: &[String], flag: &str, default: usize) -> usize {
    args.windows(2)
        .find(|pair| pair[0] == flag)
        .and_then(|pair| pair[1].parse().ok())
        .unwrap_or(default)
}

fn verify() {
    let executor = runtime();
    let rt = adapter(&executor, None);

    tokio_lifecycle::verify(&executor);
    eprintln!("fixture verification: handwritten Tokio lifecycle comparator ok");

    for shape in [Shape::Chain, Shape::Wide, Shape::Sparse] {
        let spec = GraphSpec::generate(shape, 10);
        let plan = graph(&spec, false);
        let (report, checksum) = run_finite(&executor, rt.clone(), &plan, 17);
        assert_eq!(report.outcome, Outcome::Ok, "{shape:?}");
        assert!(report.is_clean(), "{shape:?}: {report}");
        assert_ne!(checksum, 0);
    }
    eprintln!("fixture verification: graphs ok");

    let releases = Arc::new(AtomicU64::new(0));
    let plan = tiny_resource(releases.clone(), false);
    let (report, _) = run_finite(&executor, rt.clone(), &plan, 41);
    assert_eq!(report.outcome, Outcome::Ok);
    assert_eq!(report.output.as_deref(), Some(&42));
    assert!(report.is_clean(), "{report}");
    assert_eq!(releases.load(Ordering::Acquire), 1);
    eprintln!("fixture verification: normal resource release ok");

    let releases = Arc::new(AtomicU64::new(0));
    let plan = startup_failure(releases.clone());
    let (report, _) = run_finite(&executor, rt.clone(), &plan, 41);
    assert_eq!(report.outcome, Outcome::Failed);
    assert_eq!(report.output, None);
    assert_eq!(report.faults.len(), 1);
    assert_eq!(report.faults[0].node.leaf(), "failed");
    assert_eq!(report.faults[0].phase, Phase::Run);
    assert_eq!(report.faults[0].kind.to_string(), "fixture startup failure");
    assert!(report.cleanup_failures.is_empty());
    assert_eq!(releases.load(Ordering::Acquire), 1);
    eprintln!("fixture verification: startup failure ok");

    let downstream = Arc::new(AtomicU64::new(0));
    let upstream = Arc::new(AtomicU64::new(0));
    let plan = cleanup_error_keeps_upstream(downstream.clone(), upstream.clone());
    let (report, _) = run_finite(&executor, rt.clone(), &plan, 41);
    assert_eq!(report.outcome, Outcome::Ok);
    assert_eq!(report.output.as_deref(), Some(&41));
    assert!(report.faults.is_empty());
    assert_eq!(report.cleanup_failures.len(), 1);
    assert_eq!(report.cleanup_failures[0].node.leaf(), "downstream");
    assert_eq!(report.cleanup_failures[0].phase, Phase::ReleaseBody);
    assert_eq!(
        report.cleanup_failures[0].kind.to_string(),
        "fixture downstream release failure"
    );
    let release_order: Vec<_> = report
        .trace
        .as_ref()
        .expect("full trace")
        .events
        .iter()
        .filter(|event| matches!(event.kind, TraceKind::ReleaseStart))
        .map(|event| event.node.as_ref().expect("release node").leaf())
        .collect();
    assert_eq!(release_order, ["downstream", "upstream"]);
    assert_eq!(downstream.load(Ordering::Acquire), 1);
    assert_eq!(upstream.load(Ordering::Acquire), 1);
    eprintln!("fixture verification: cleanup failure ok");

    let acquired = Arc::new(AtomicU64::new(0));
    let released = Arc::new(AtomicU64::new(0));
    let plan = cancel_after_acquire(acquired.clone(), released.clone());
    let report = executor.block_on(async {
        let running = plan.start(rt.clone(), 1);
        let handle = running.handle();
        await_cancel_after(running, handle, acquired.clone(), 1).await
    });
    assert_eq!(report.outcome, Outcome::Cancelled);
    assert_eq!(report.output, None);
    assert!(report.faults.is_empty());
    assert!(report.cleanup_failures.is_empty());
    assert_eq!(released.load(Ordering::Acquire), 1);
    eprintln!("fixture verification: cancellation ok");

    let compensated = Arc::new(AtomicU64::new(0));
    let plan = known_receipt(compensated.clone());
    let (report, _) = run_finite(&executor, rt.clone(), &plan, 5);
    assert_eq!(report.outcome, Outcome::Ok);
    assert_eq!(compensated.load(Ordering::Acquire), 1);
    let plan = retry_once();
    let (report, _) = run_finite(&executor, rt.clone(), &plan, 5);
    assert_eq!(report.outcome, Outcome::Ok);
    assert_eq!(report.output.as_deref(), Some(&5));

    let plan = dynamic_instances(3);
    let report = executor.block_on(async {
        let mut running = plan.start(rt.clone(), 4);
        running.ready().await.expect("dynamic ready");
        running.shutdown();
        running.await
    });
    assert_eq!(report.outcome, Outcome::Ok);
    assert!(report.is_clean(), "{report}");

    let plan = repeated_typed_components(10);
    let (report, _) = run_finite(&executor, rt.clone(), &plan, 5);
    assert_eq!(report.output.as_deref(), Some(&15));
    assert!(report.is_clean(), "{report}");
    eprintln!("fixture verification: repeated typed components ok");

    let releases = Arc::new(AtomicU64::new(0));
    let plan = repeated_resource_components(10, releases.clone());
    for (run, input) in [5, 17].into_iter().enumerate() {
        let (report, _) = run_finite(&executor, rt.clone(), &plan, input);
        assert_eq!(report.output.as_deref(), Some(&(input + 10)));
        assert!(report.is_clean(), "{report}");
        assert_eq!(releases.load(Ordering::Acquire), (run as u64 + 1) * 10);
    }
    eprintln!("fixture verification: repeated resource components cleanup ok");

    for resolves in [true, false] {
        let started = Arc::new(AtomicU64::new(0));
        let recovered = Arc::new(AtomicU64::new(0));
        let plan = unknown_recovery(resolves, started.clone(), recovered.clone());
        let report = executor.block_on(run_unknown_recovery(
            plan.start(rt.clone(), 41),
            started.clone(),
            1,
        ));
        assert_eq!(report.outcome, Outcome::Cancelled);
        assert_eq!(recovered.load(Ordering::Acquire), 1);
        if resolves {
            assert!(report.ambiguous.is_empty());
            assert!(report.cleanup_failures.is_empty());
        } else {
            assert_eq!(report.ambiguous.len(), 1);
            assert_eq!(report.cleanup_failures.len(), 1);
            assert_eq!(report.cleanup_failures[0].phase, sdax::Phase::Recover);
        }
        let report = executor.block_on(run_unknown_recovery(
            plan.start(rt.clone(), 41),
            started.clone(),
            2,
        ));
        assert_eq!(report.outcome, Outcome::Cancelled);
        assert_eq!(recovered.load(Ordering::Acquire), 2);
    }
    eprintln!("fixture verification: unknown recovery ok");

    let initializes = Arc::new(AtomicU64::new(0));
    let episodes = Arc::new(AtomicU64::new(0));
    let first_handle = Arc::new(AtomicUsize::new(0));
    let mismatches = Arc::new(AtomicU64::new(0));
    let plan = stable_service_recovery(
        false,
        initializes.clone(),
        episodes.clone(),
        first_handle,
        mismatches.clone(),
    );
    let report = executor.block_on(run_service_until_episode(
        plan.start(rt.clone(), ()),
        episodes.clone(),
        3,
    ));
    assert_eq!(report.outcome, Outcome::Ok);
    assert!(report.is_clean(), "{report}");
    assert_eq!(initializes.load(Ordering::Acquire), 1);
    assert_eq!(episodes.load(Ordering::Acquire), 3);
    assert_eq!(mismatches.load(Ordering::Acquire), 0);
    let report = executor.block_on(run_service_until_episode(
        plan.start(rt.clone(), ()),
        episodes.clone(),
        6,
    ));
    assert_eq!(report.outcome, Outcome::Ok);
    assert_eq!(initializes.load(Ordering::Acquire), 2);
    assert_eq!(episodes.load(Ordering::Acquire), 6);
    assert_eq!(mismatches.load(Ordering::Acquire), 0);

    let initializes = Arc::new(AtomicU64::new(0));
    let episodes = Arc::new(AtomicU64::new(0));
    let mismatches = Arc::new(AtomicU64::new(0));
    let plan = stable_service_recovery(
        true,
        initializes.clone(),
        episodes.clone(),
        Arc::new(AtomicUsize::new(0)),
        mismatches.clone(),
    );
    let report = executor.block_on(plan.start(rt.clone(), ()));
    assert_eq!(report.outcome, Outcome::Failed);
    assert_eq!(initializes.load(Ordering::Acquire), 1);
    assert_eq!(episodes.load(Ordering::Acquire), 2);
    assert_eq!(mismatches.load(Ordering::Acquire), 0);
    assert_eq!(report.faults.len(), 1);
    assert_eq!(report.faults[0].phase, sdax::Phase::Serve);
    let report = executor.block_on(plan.start(rt.clone(), ()));
    assert_eq!(report.outcome, Outcome::Failed);
    assert_eq!(initializes.load(Ordering::Acquire), 2);
    assert_eq!(episodes.load(Ordering::Acquire), 4);
    assert_eq!(mismatches.load(Ordering::Acquire), 0);
    eprintln!("fixture verification: stable service recovery ok");

    let ended = Arc::new(AtomicU64::new(0));
    let plan = dynamic_sequential_churn(ended.clone());
    let report = executor.block_on(async {
        let mut running = plan.start(rt.clone(), 10);
        running.ready().await.expect("churn ready");
        running.shutdown();
        running.await
    });
    assert_eq!(report.outcome, Outcome::Ok);
    assert!(report.is_clean(), "{report}");
    assert_eq!(ended.load(Ordering::Acquire), 10);
    let report = executor.block_on(async {
        let mut running = plan.start(rt.clone(), 3);
        running.ready().await.expect("second churn ready");
        running.shutdown();
        running.await
    });
    assert_eq!(report.outcome, Outcome::Ok);
    assert_eq!(ended.load(Ordering::Acquire), 13);
    eprintln!("fixture verification: sequential dynamic churn ok");
    assert_eq!(rt.tracked(), 0, "correctness fixtures leave no tasks");
    eprintln!("fixture verification: ok");
}

fn bench(cfg: Config) -> Vec<Sample> {
    let mut out = Vec::new();
    benchmark_runtime_setup(cfg, &mut out);
    for nodes in [1, 10, 100, 1_000] {
        for shape in [Shape::Chain, Shape::Wide, Shape::Sparse] {
            benchmark_graph(cfg, shape, nodes, false, &mut out);
        }
    }
    benchmark_graph(cfg, Shape::Chain, 1, true, &mut out);
    benchmark_resources(cfg, &mut out);
    benchmark_safety_paths(cfg, &mut out);
    benchmark_components(cfg, &mut out);
    benchmark_revised_components(cfg, &mut out);
    benchmark_resident(cfg, &mut out);
    benchmark_service_recovery(cfg, &mut out);
    benchmark_dynamic(cfg, &mut out);
    benchmark_dynamic_churn(cfg, &mut out);
    benchmark_unknown_recovery(cfg, &mut out);
    benchmark_trace(cfg, &mut out);
    benchmark_application(cfg, &mut out);
    out
}

fn benchmark_runtime_setup(cfg: Config, out: &mut Vec<Sample>) {
    for sample in 0..cfg.samples {
        let (nanos, checksum) = timed(|| {
            let executor = runtime();
            let rt = adapter(&executor, None);
            let checksum = rt.tracked() as u64;
            drop(rt);
            drop(executor);
            ((), checksum)
        });
        push(
            out,
            "adapter_setup",
            "current_thread_runtime",
            0,
            0,
            sample,
            nanos,
            0,
            0,
            checksum,
        );
    }
}

fn benchmark_graph(cfg: Config, shape: Shape, nodes: usize, yielding: bool, out: &mut Vec<Sample>) {
    let label = if yielding {
        format!("tiny_yield_{shape:?}").to_lowercase()
    } else {
        GraphSpec::generate(shape, nodes).label().to_string()
    };
    for sample in 0..cfg.build_samples {
        let (nanos, checksum) = timed(|| {
            let spec = GraphSpec::generate(shape, nodes);
            let checksum = spec.names.iter().map(String::len).sum::<usize>() as u64;
            (spec, checksum)
        });
        push(
            out,
            "fixture_generation",
            &label,
            nodes,
            nodes - 1,
            sample,
            nanos,
            0,
            0,
            checksum,
        );
    }
    let spec = GraphSpec::generate(shape, nodes);
    benchmark_build(
        cfg,
        &label,
        nodes,
        spec.edges(),
        || graph(&spec, yielding),
        out,
    );
    let plan = graph(&spec, yielding);
    benchmark_finite_execution(cfg, &label, nodes, spec.edges(), &plan, None, out);
    benchmark_state_setup(cfg, &label, nodes, spec.edges(), &plan, out);
    if !yielding && shape == Shape::Chain {
        benchmark_machine(cfg, &label, nodes, spec.edges(), &plan, out);
    }
}

fn benchmark_build(
    cfg: Config,
    label: &str,
    nodes: usize,
    edges: usize,
    build: impl Fn() -> Plan<u64, u64>,
    out: &mut Vec<Sample>,
) {
    for sample in 0..cfg.build_samples {
        let (nanos, checksum) = timed(|| {
            let plan = build();
            let checksum = plan.name().len() as u64;
            (plan, checksum)
        });
        push(
            out,
            "plan_build",
            label,
            nodes,
            edges,
            sample,
            nanos,
            0,
            0,
            checksum,
        );
    }
    let (allocations, bytes, checksum) = allocated(|| {
        let plan = build();
        let checksum = plan.name().len() as u64;
        (plan, checksum)
    });
    push(
        out,
        "plan_build_alloc",
        label,
        nodes,
        edges,
        0,
        0,
        allocations,
        bytes,
        checksum,
    );
}

fn benchmark_finite_execution(
    cfg: Config,
    label: &str,
    nodes: usize,
    edges: usize,
    plan: &Plan<u64, u64>,
    observer: Option<Arc<dyn Observer>>,
    out: &mut Vec<Sample>,
) {
    benchmark_finite_execution_phase(
        cfg,
        "engine_execution",
        label,
        nodes,
        edges,
        plan,
        observer,
        out,
    );
}

#[allow(clippy::too_many_arguments)]
fn benchmark_finite_execution_phase(
    cfg: Config,
    phase: &'static str,
    label: &str,
    nodes: usize,
    edges: usize,
    plan: &Plan<u64, u64>,
    observer: Option<Arc<dyn Observer>>,
    out: &mut Vec<Sample>,
) {
    let executor = runtime();
    let rt = adapter(&executor, observer);
    for _ in 0..cfg.warmup {
        consume_run(&executor, rt.clone(), plan, 7);
    }
    for sample in 0..cfg.samples {
        let (nanos, checksum) = timed(|| consume_run(&executor, rt.clone(), plan, sample as u64));
        push(
            out, phase, label, nodes, edges, sample, nanos, 0, 0, checksum,
        );
    }
    let (allocations, bytes, checksum) =
        allocated(|| consume_run(&executor, rt.clone(), plan, 0x5a5a));
    let allocation_phase = if phase == "representative_application" {
        "representative_application_alloc"
    } else {
        "engine_execution_alloc"
    };
    push(
        out,
        allocation_phase,
        label,
        nodes,
        edges,
        0,
        0,
        allocations,
        bytes,
        checksum,
    );
    assert_eq!(rt.tracked(), 0);
}

fn consume_run(
    executor: &tokio::runtime::Runtime,
    rt: Arc<TokioRuntime>,
    plan: &Plan<u64, u64>,
    input: u64,
) -> ((), u64) {
    let (report, checksum) = run_finite(executor, rt.clone(), plan, black_box(input));
    assert!(matches!(
        report.outcome,
        Outcome::Ok | Outcome::Failed | Outcome::Cancelled
    ));
    drop(report);
    drain_tracked(executor, &rt);
    ((), checksum)
}

fn drain_tracked(executor: &tokio::runtime::Runtime, rt: &Arc<TokioRuntime>) {
    executor.block_on(async {
        for _ in 0..10_000 {
            if rt.tracked() == 0 {
                return;
            }
            yield_once().await;
        }
    });
    assert_eq!(rt.tracked(), 0, "run left tracked tasks after disposal");
}

fn benchmark_state_setup(
    cfg: Config,
    label: &str,
    nodes: usize,
    edges: usize,
    plan: &Plan<u64, u64>,
    out: &mut Vec<Sample>,
) {
    for sample in 0..cfg.samples {
        let (nanos, checksum) = timed(|| {
            let machine = Machine::with_input(plan).expect("machine");
            let checksum = machine.nodes().len() as u64;
            drop(machine);
            ((), checksum)
        });
        push(
            out,
            "machine_state_setup",
            label,
            nodes,
            edges,
            sample,
            nanos,
            0,
            0,
            checksum,
        );
        let (nanos, checksum) = timed(|| {
            let bodies = bodies_of_with_input(plan, sample as u64);
            let checksum = bodies.export().is_some() as u64;
            drop(bodies);
            ((), checksum)
        });
        push(
            out,
            "body_state_setup",
            label,
            nodes,
            edges,
            sample,
            nanos,
            0,
            0,
            checksum,
        );
    }
}

fn benchmark_machine(
    cfg: Config,
    label: &str,
    nodes: usize,
    edges: usize,
    plan: &Plan<u64, u64>,
    out: &mut Vec<Sample>,
) {
    for sample in 0..cfg.samples {
        let mut machine = Machine::with_input(plan).expect("machine");
        let (nanos, checksum) = timed(|| {
            let checksum = drive_machine(&mut machine);
            ((), checksum)
        });
        push(
            out,
            "pure_machine_events_preseeded",
            label,
            nodes,
            edges,
            sample,
            nanos,
            0,
            0,
            checksum,
        );
    }
}

fn drive_machine(machine: &mut Machine) -> u64 {
    let mut effects: VecDeque<Effect> = machine.begin().into();
    let mut checksum = 0u64;
    while let Some(effect) = effects.pop_front() {
        match effect {
            Effect::Spawn { node, attempt } | Effect::SpawnBlocking { node, attempt } => {
                checksum ^= node.idx as u64 ^ attempt as u64;
                effects.extend(machine.step(Event::Started(node)));
                effects.extend(machine.step(Event::NodeOk(node)));
            }
            Effect::PublishReady { node } => {
                effects.extend(machine.step(Event::ReadyPublished {
                    node,
                    result: Ok(()),
                }));
            }
            Effect::End(outcome) => {
                checksum ^= match outcome {
                    Outcome::Ok => 1,
                    Outcome::Failed => 2,
                    Outcome::Cancelled => 3,
                }
            }
            Effect::Emit(event) => checksum ^= event.at.as_nanos(),
            Effect::CancelTimer(_) | Effect::RefreshDeadline(_) | Effect::Timer { .. } => {}
            other => panic!("unsupported pure-machine fixture effect: {other:?}"),
        }
    }
    let report = machine.take_report().expect("machine report");
    checksum ^ report.faults.len() as u64
}

fn benchmark_resources(cfg: Config, out: &mut Vec<Sample>) {
    for (label, kind) in [
        ("resource_normal", 0u8),
        ("resource_release_failure", 1),
        ("resource_startup_failure", 2),
        ("resource_cleanup_error_upstream", 3),
    ] {
        let a = Arc::new(AtomicU64::new(0));
        let b = Arc::new(AtomicU64::new(0));
        let plan = match kind {
            0 => tiny_resource(a.clone(), false),
            1 => tiny_resource(a.clone(), true),
            2 => startup_failure(a.clone()),
            _ => cleanup_error_keeps_upstream(a.clone(), b.clone()),
        };
        benchmark_finite_execution(
            cfg,
            label,
            if kind == 3 { 3 } else { 2 },
            1,
            &plan,
            None,
            out,
        );
    }
    benchmark_cancel(cfg, out);
}

fn benchmark_safety_paths(cfg: Config, out: &mut Vec<Sample>) {
    let compensated = Arc::new(AtomicU64::new(0));
    let plan = known_receipt(compensated);
    benchmark_finite_execution(cfg, "effect_known_receipt", 1, 0, &plan, None, out);
    let plan = retry_once();
    benchmark_finite_execution(cfg, "retry_one_failure", 1, 0, &plan, None, out);
}

fn benchmark_cancel(cfg: Config, out: &mut Vec<Sample>) {
    let executor = runtime();
    let rt = adapter(&executor, None);
    let acquired = Arc::new(AtomicU64::new(0));
    let released = Arc::new(AtomicU64::new(0));
    let plan = cancel_after_acquire(acquired.clone(), released.clone());
    for sample in 0..cfg.samples {
        let before = released.load(Ordering::Acquire);
        let (nanos, checksum) = timed(|| {
            let report = executor.block_on(async {
                let running = plan.start(rt.clone(), sample as u64);
                let handle = running.handle();
                let target = acquired.load(Ordering::Acquire) + 1;
                await_cancel_after(running, handle, acquired.clone(), target).await
            });
            let checksum = report.cleanup_failures.len() as u64 + 3;
            drop(report);
            drain_tracked(&executor, &rt);
            ((), checksum)
        });
        assert_eq!(released.load(Ordering::Acquire), before + 1);
        push(
            out,
            "engine_execution",
            "resource_cancel_after_acquire",
            1,
            0,
            sample,
            nanos,
            0,
            0,
            checksum,
        );
    }
}

async fn await_cancel_after<O: Send + Sync + 'static>(
    running: sdax_tokio::Running<O>,
    handle: sdax_tokio::RunHandle,
    acquired: Arc<AtomicU64>,
    target: u64,
) -> sdax::Report<O> {
    let mut running = Box::pin(running);
    let mut cancelled = false;
    std::future::poll_fn(move |cx| {
        if let std::task::Poll::Ready(report) = running.as_mut().poll(cx) {
            return std::task::Poll::Ready(report);
        }
        if !cancelled && acquired.load(Ordering::Acquire) >= target {
            handle.cancel();
            cancelled = true;
        }
        cx.waker().wake_by_ref();
        std::task::Poll::Pending
    })
    .await
}

async fn run_unknown_recovery(
    running: sdax_tokio::Running<()>,
    started: Arc<AtomicU64>,
    target: u64,
) -> sdax::Report<()> {
    let handle = running.handle();
    await_cancel_after(running, handle, started, target).await
}

async fn run_service_until_episode(
    running: sdax_tokio::Running<()>,
    episodes: Arc<AtomicU64>,
    target: u64,
) -> sdax::Report<()> {
    let handle = running.handle();
    let mut running = Box::pin(running);
    let mut shutdown = false;
    std::future::poll_fn(move |cx| {
        if let std::task::Poll::Ready(report) = running.as_mut().poll(cx) {
            return std::task::Poll::Ready(report);
        }
        if !shutdown && episodes.load(Ordering::Acquire) >= target {
            handle.shutdown();
            shutdown = true;
        }
        cx.waker().wake_by_ref();
        std::task::Poll::Pending
    })
    .await
}

fn benchmark_components(cfg: Config, out: &mut Vec<Sample>) {
    for nodes in [10, 100] {
        let flat = flat_component_equivalent(nodes);
        benchmark_finite_execution(cfg, "component_flat", nodes, nodes - 1, &flat, None, out);
        let nested = nested_component(nodes);
        benchmark_finite_execution(
            cfg,
            "component_nested",
            nodes + 3,
            nodes + 2,
            &nested,
            None,
            out,
        );
    }
}

fn benchmark_revised_components(cfg: Config, out: &mut Vec<Sample>) {
    for mounts in [2, 10, 100] {
        let label = format!("component_repeated_typed_{mounts}");
        benchmark_build(
            cfg,
            &label,
            mounts,
            mounts,
            || repeated_typed_components(mounts),
            out,
        );
        let plan = repeated_typed_components(mounts);
        benchmark_finite_execution(cfg, &label, mounts, mounts, &plan, None, out);
    }
}

fn benchmark_resident(cfg: Config, out: &mut Vec<Sample>) {
    let plan = resident_noop();
    let executor = runtime();
    let rt = adapter(&executor, None);
    for _ in 0..cfg.warmup {
        consume_resident(&executor, rt.clone(), &plan, 1);
    }
    for sample in 0..cfg.samples {
        let (nanos, checksum) =
            timed(|| consume_resident(&executor, rt.clone(), &plan, sample as u64));
        push(
            out,
            "engine_execution",
            "resident_ready_shutdown",
            1,
            0,
            sample,
            nanos,
            0,
            0,
            checksum,
        );
    }
}

fn benchmark_service_recovery(cfg: Config, out: &mut Vec<Sample>) {
    for exhausts in [false, true] {
        let label = if exhausts {
            "service_restart_exhausted"
        } else {
            "service_restart_stable_handle"
        };
        let initializes = Arc::new(AtomicU64::new(0));
        let episodes = Arc::new(AtomicU64::new(0));
        let mismatches = Arc::new(AtomicU64::new(0));
        let plan = stable_service_recovery(
            exhausts,
            initializes.clone(),
            episodes.clone(),
            Arc::new(AtomicUsize::new(0)),
            mismatches.clone(),
        );
        let executor = runtime();
        let rt = adapter(&executor, None);
        for _ in 0..cfg.warmup {
            consume_service_recovery(
                &executor,
                rt.clone(),
                &plan,
                exhausts,
                initializes.clone(),
                episodes.clone(),
            );
        }
        for sample in 0..cfg.samples {
            let (nanos, checksum) = timed(|| {
                consume_service_recovery(
                    &executor,
                    rt.clone(),
                    &plan,
                    exhausts,
                    initializes.clone(),
                    episodes.clone(),
                )
            });
            push(
                out,
                "engine_execution",
                label,
                1,
                0,
                sample,
                nanos,
                0,
                0,
                checksum,
            );
        }
        let (allocations, bytes, checksum) = allocated(|| {
            consume_service_recovery(
                &executor,
                rt.clone(),
                &plan,
                exhausts,
                initializes.clone(),
                episodes.clone(),
            )
        });
        push(
            out,
            "engine_execution_alloc",
            label,
            1,
            0,
            0,
            0,
            allocations,
            bytes,
            checksum,
        );
        assert_eq!(mismatches.load(Ordering::Acquire), 0);
        assert_eq!(rt.tracked(), 0);
    }
}

fn consume_service_recovery(
    executor: &tokio::runtime::Runtime,
    rt: Arc<TokioRuntime>,
    plan: &Plan<(), ()>,
    exhausts: bool,
    initializes: Arc<AtomicU64>,
    episodes: Arc<AtomicU64>,
) -> ((), u64) {
    let initialize_target = initializes.load(Ordering::Acquire) + 1;
    let target = episodes.load(Ordering::Acquire) + if exhausts { 2 } else { 3 };
    let report = if exhausts {
        executor.block_on(plan.start(rt.clone(), ()))
    } else {
        executor.block_on(run_service_until_episode(
            plan.start(rt.clone(), ()),
            episodes.clone(),
            target,
        ))
    };
    assert_eq!(
        report.outcome,
        if exhausts {
            Outcome::Failed
        } else {
            Outcome::Ok
        }
    );
    assert_eq!(initializes.load(Ordering::Acquire), initialize_target);
    assert_eq!(episodes.load(Ordering::Acquire), target);
    if exhausts {
        assert_eq!(report.faults.len(), 1);
        assert_eq!(report.faults[0].phase, sdax::Phase::Serve);
    } else {
        assert!(report.is_clean(), "{report}");
    }
    let checksum = report.faults.len() as u64 + report.cleanup_failures.len() as u64 + target;
    drop(report);
    drain_tracked(executor, &rt);
    ((), checksum)
}

fn consume_resident(
    executor: &tokio::runtime::Runtime,
    rt: Arc<TokioRuntime>,
    plan: &Plan<(), u64>,
    input: u64,
) -> ((), u64) {
    let report = executor.block_on(async {
        let mut running = plan.start(rt.clone(), input);
        running.ready().await.expect("resident ready");
        running.shutdown();
        running.await
    });
    assert_eq!(report.outcome, Outcome::Ok);
    let checksum = report.cleanup_failures.len() as u64 + 1;
    drop(report);
    drain_tracked(executor, &rt);
    ((), checksum)
}

fn benchmark_dynamic(cfg: Config, out: &mut Vec<Sample>) {
    for instances in [1, 10, 100] {
        let label = format!("dynamic_live_instances_{instances}");
        let plan = dynamic_instances(3);
        let executor = runtime();
        let rt = adapter(&executor, None);
        for _ in 0..cfg.warmup {
            consume_dynamic(&executor, rt.clone(), &plan, instances);
        }
        for sample in 0..cfg.samples {
            let (nanos, checksum) =
                timed(|| consume_dynamic(&executor, rt.clone(), &plan, instances));
            push(
                out,
                "engine_execution",
                &label,
                3,
                2,
                sample,
                nanos,
                0,
                0,
                checksum ^ instances as u64,
            );
        }
        let (allocations, bytes, checksum) =
            allocated(|| consume_dynamic(&executor, rt.clone(), &plan, instances));
        push(
            out,
            "engine_execution_alloc",
            &label,
            3,
            2,
            instances,
            0,
            allocations,
            bytes,
            checksum ^ instances as u64,
        );
    }
}

fn benchmark_dynamic_churn(cfg: Config, out: &mut Vec<Sample>) {
    for instances in [1, 10, 100] {
        let label = format!("dynamic_sequential_churn_{instances}");
        let ended = Arc::new(AtomicU64::new(0));
        let plan = dynamic_sequential_churn(ended.clone());
        let executor = runtime();
        let rt = adapter(&executor, None);
        for _ in 0..cfg.warmup {
            consume_dynamic_churn(&executor, rt.clone(), &plan, instances, ended.clone());
        }
        for sample in 0..cfg.samples {
            let (nanos, checksum) = timed(|| {
                consume_dynamic_churn(&executor, rt.clone(), &plan, instances, ended.clone())
            });
            push(
                out,
                "engine_execution",
                &label,
                2,
                1,
                sample,
                nanos,
                0,
                0,
                checksum,
            );
        }
        let (allocations, bytes, checksum) = allocated(|| {
            consume_dynamic_churn(&executor, rt.clone(), &plan, instances, ended.clone())
        });
        push(
            out,
            "engine_execution_alloc",
            &label,
            2,
            1,
            instances,
            0,
            allocations,
            bytes,
            checksum,
        );
        assert_eq!(rt.tracked(), 0);
    }
}

fn consume_dynamic_churn(
    executor: &tokio::runtime::Runtime,
    rt: Arc<TokioRuntime>,
    plan: &Plan<(), usize>,
    instances: usize,
    ended: Arc<AtomicU64>,
) -> ((), u64) {
    let before = ended.load(Ordering::Acquire);
    let report = executor.block_on(async {
        let mut running = plan.start(rt.clone(), instances);
        running.ready().await.expect("churn ready");
        running.shutdown();
        running.await
    });
    assert_eq!(report.outcome, Outcome::Ok);
    assert_eq!(ended.load(Ordering::Acquire), before + instances as u64);
    let checksum = report.cleanup_failures.len() as u64 + before + instances as u64;
    drop(report);
    drain_tracked(executor, &rt);
    ((), checksum)
}

fn consume_dynamic(
    executor: &tokio::runtime::Runtime,
    rt: Arc<TokioRuntime>,
    plan: &Plan<(), usize>,
    instances: usize,
) -> ((), u64) {
    let report = executor.block_on(async {
        let mut running = plan.start(rt.clone(), instances);
        running.ready().await.expect("dynamic ready");
        running.shutdown();
        running.await
    });
    assert_eq!(report.outcome, Outcome::Ok);
    let checksum = report.cleanup_failures.len() as u64 + 1;
    drop(report);
    drain_tracked(executor, &rt);
    ((), checksum)
}

fn benchmark_unknown_recovery(cfg: Config, out: &mut Vec<Sample>) {
    for resolves in [true, false] {
        let label = if resolves {
            "effect_unknown_recovery_resolved"
        } else {
            "effect_unknown_recovery_failed"
        };
        let started = Arc::new(AtomicU64::new(0));
        let recovered = Arc::new(AtomicU64::new(0));
        let plan = unknown_recovery(resolves, started.clone(), recovered.clone());
        let executor = runtime();
        let rt = adapter(&executor, None);
        for _ in 0..cfg.warmup {
            consume_unknown_recovery(
                &executor,
                rt.clone(),
                &plan,
                resolves,
                started.clone(),
                recovered.clone(),
            );
        }
        for sample in 0..cfg.samples {
            let (nanos, checksum) = timed(|| {
                consume_unknown_recovery(
                    &executor,
                    rt.clone(),
                    &plan,
                    resolves,
                    started.clone(),
                    recovered.clone(),
                )
            });
            push(
                out,
                "engine_execution",
                label,
                1,
                0,
                sample,
                nanos,
                0,
                0,
                checksum,
            );
        }
        let (allocations, bytes, checksum) = allocated(|| {
            consume_unknown_recovery(
                &executor,
                rt.clone(),
                &plan,
                resolves,
                started.clone(),
                recovered.clone(),
            )
        });
        push(
            out,
            "engine_execution_alloc",
            label,
            1,
            0,
            0,
            0,
            allocations,
            bytes,
            checksum,
        );
        assert_eq!(rt.tracked(), 0);
    }
}

fn consume_unknown_recovery(
    executor: &tokio::runtime::Runtime,
    rt: Arc<TokioRuntime>,
    plan: &Plan<(), u64>,
    resolves: bool,
    started: Arc<AtomicU64>,
    recovered: Arc<AtomicU64>,
) -> ((), u64) {
    let start_target = started.load(Ordering::Acquire) + 1;
    let recovery_target = recovered.load(Ordering::Acquire) + 1;
    let report = executor.block_on(run_unknown_recovery(
        plan.start(rt.clone(), 41),
        started,
        start_target,
    ));
    assert_eq!(report.outcome, Outcome::Cancelled);
    assert_eq!(recovered.load(Ordering::Acquire), recovery_target);
    if resolves {
        assert!(report.ambiguous.is_empty());
        assert!(report.cleanup_failures.is_empty());
    } else {
        assert_eq!(report.ambiguous.len(), 1);
        assert_eq!(report.cleanup_failures.len(), 1);
    }
    let checksum =
        report.ambiguous.len() as u64 + report.cleanup_failures.len() as u64 + recovery_target;
    drop(report);
    drain_tracked(executor, &rt);
    ((), checksum)
}

fn benchmark_trace(cfg: Config, out: &mut Vec<Sample>) {
    let plan = graph(&GraphSpec::generate(Shape::Chain, 100), false);
    benchmark_finite_execution(cfg, "trace_default", 100, 99, &plan, None, out);
    let observer = Arc::new(CountingObserver(AtomicU64::new(0)));
    benchmark_finite_execution(
        cfg,
        "trace_observer",
        100,
        99,
        &plan,
        Some(observer.clone()),
        out,
    );
    assert!(observer.0.load(Ordering::Relaxed) > 0);
}

fn benchmark_application(cfg: Config, out: &mut Vec<Sample>) {
    for nodes in [10, 100] {
        let plan = cpu_application(nodes);
        benchmark_finite_execution_phase(
            cfg,
            "representative_application",
            "application_hash_sdax",
            nodes,
            nodes - 1,
            &plan,
            None,
            out,
        );
        for sample in 0..cfg.samples {
            let (nanos, checksum) = timed(|| ((), handwritten_hash(nodes, sample as u64)));
            push(
                out,
                "representative_application",
                "application_hash_handwritten",
                nodes,
                nodes - 1,
                sample,
                nanos,
                0,
                0,
                checksum,
            );
        }
    }
}

fn handwritten_hash(nodes: usize, input: u64) -> u64 {
    let mut value = input;
    for i in 1..nodes {
        value ^= i as u64;
        for _ in 0..64 {
            value = value.wrapping_mul(0x9e37_79b9_7f4a_7c15).rotate_left(13) ^ 0xa5a5_a5a5;
        }
    }
    black_box(value)
}

#[allow(clippy::too_many_arguments)]
fn push(
    out: &mut Vec<Sample>,
    phase: &'static str,
    workload: &str,
    nodes: usize,
    edges: usize,
    sample: usize,
    nanos: u128,
    allocations: u64,
    allocated_bytes: u64,
    checksum: u64,
) {
    out.push(Sample {
        phase,
        workload: workload.to_string(),
        nodes,
        edges,
        sample,
        nanos,
        allocations,
        allocated_bytes,
        checksum,
    });
}
