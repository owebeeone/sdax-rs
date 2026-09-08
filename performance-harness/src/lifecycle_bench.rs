use crate::fixtures::{adapter, runtime};
use crate::lifecycle;
use crate::lifecycle_plans;
use crate::measure::{profiled, timed, Sample};
use crate::{drain_tracked, push, tokio_lifecycle};
use sdax::Plan;
use sdax_tokio::{PlanStart, TokioRuntime};
use std::future::Future;
use std::sync::Arc;

#[derive(Clone, Copy)]
struct Config {
    samples: usize,
    warmup: usize,
}

pub fn bench(samples: usize, warmup: usize, tokio_first: bool) -> Vec<Sample> {
    let cfg = Config { samples, warmup };
    let executor = runtime();
    let rt = adapter(&executor, None);
    let mut samples = Vec::new();
    for case in lifecycle::Case::ALL {
        let sdax_evidence = lifecycle::Evidence::new();
        let plan = lifecycle_plans::plan(case, sdax_evidence.clone());
        let tokio_evidence = lifecycle::Evidence::new();
        let expected = lifecycle::expected(case);
        if tokio_first {
            benchmark_tokio_lifecycle(
                cfg,
                &executor,
                case,
                &tokio_evidence,
                &expected,
                &mut samples,
            );
            benchmark_sdax_lifecycle(
                cfg,
                &executor,
                rt.clone(),
                &plan,
                case,
                &sdax_evidence,
                &expected,
                &mut samples,
            );
        } else {
            benchmark_sdax_lifecycle(
                cfg,
                &executor,
                rt.clone(),
                &plan,
                case,
                &sdax_evidence,
                &expected,
                &mut samples,
            );
            benchmark_tokio_lifecycle(
                cfg,
                &executor,
                case,
                &tokio_evidence,
                &expected,
                &mut samples,
            );
        }
    }
    samples
}

pub fn verify() {
    let executor = runtime();
    let rt = adapter(&executor, None);
    for case in lifecycle::Case::ALL {
        let evidence = lifecycle::Evidence::new();
        let plan = lifecycle_plans::plan(case, evidence.clone());
        let summary = run_sdax_lifecycle(&executor, rt.clone(), &plan, case, &evidence);
        lifecycle::check(&summary, &lifecycle::expected(case)).unwrap();
    }
}

fn run_sdax_lifecycle(
    executor: &tokio::runtime::Runtime,
    rt: Arc<TokioRuntime>,
    plan: &Plan<u64, u64>,
    case: lifecycle::Case,
    evidence: &lifecycle::Evidence,
) -> lifecycle::Summary {
    let ready = evidence.begin(case);
    let report = if case == lifecycle::Case::Cancellation {
        executor.block_on(await_lifecycle_cancel(
            plan.start(rt.clone(), 41),
            ready.expect("cancellation acquisition receiver"),
        ))
    } else {
        executor.block_on(plan.start(rt.clone(), 41))
    };
    let mut summary = lifecycle::normalize_report(&report, evidence);
    drop(report);
    drain_tracked(executor, &rt);
    summary.active = rt.tracked();
    summary.disposed = evidence.disposed();
    summary
}

async fn await_lifecycle_cancel(
    running: sdax_tokio::Running<u64>,
    ready: tokio::sync::oneshot::Receiver<()>,
) -> sdax::Report<u64> {
    let handle = running.handle();
    let mut running = Box::pin(running);
    let mut ready = Box::pin(ready);
    let mut cancelled = false;
    std::future::poll_fn(move |cx| {
        if let std::task::Poll::Ready(report) = running.as_mut().poll(cx) {
            return std::task::Poll::Ready(report);
        }
        if !cancelled && ready.as_mut().poll(cx).is_ready() {
            handle.cancel();
            cancelled = true;
        }
        std::task::Poll::Pending
    })
    .await
}

fn checked_lifecycle(
    summary: lifecycle::Summary,
    expected: &lifecycle::Expected,
) -> ((), u64) {
    lifecycle::check(&summary, expected).unwrap();
    let checksum = lifecycle::checksum(&summary);
    drop(summary);
    ((), checksum)
}

#[allow(clippy::too_many_arguments)]
fn benchmark_sdax_lifecycle(
    cfg: Config,
    executor: &tokio::runtime::Runtime,
    rt: Arc<TokioRuntime>,
    plan: &Plan<u64, u64>,
    case: lifecycle::Case,
    evidence: &lifecycle::Evidence,
    expected: &lifecycle::Expected,
    out: &mut Vec<Sample>,
) {
    for _ in 0..cfg.warmup {
        checked_lifecycle(
            run_sdax_lifecycle(executor, rt.clone(), plan, case, evidence),
            expected,
        );
    }
    for sample in 0..cfg.samples {
        let (nanos, checksum) = timed(|| {
            checked_lifecycle(
                run_sdax_lifecycle(executor, rt.clone(), plan, case, evidence),
                expected,
            )
        });
        push(
            out,
            "lifecycle_sdax",
            case.label(),
            case.nodes(),
            case.nodes().saturating_sub(1),
            sample,
            nanos,
            0,
            0,
            checksum,
        );
    }
    benchmark_lifecycle_profiles(out, "lifecycle_sdax_alloc", case, || {
        checked_lifecycle(
            run_sdax_lifecycle(executor, rt.clone(), plan, case, evidence),
            expected,
        )
    });
}

fn benchmark_tokio_lifecycle(
    cfg: Config,
    executor: &tokio::runtime::Runtime,
    case: lifecycle::Case,
    evidence: &lifecycle::Evidence,
    expected: &lifecycle::Expected,
    out: &mut Vec<Sample>,
) {
    for _ in 0..cfg.warmup {
        checked_lifecycle(tokio_lifecycle::run(executor, case, evidence), expected);
    }
    for sample in 0..cfg.samples {
        let (nanos, checksum) = timed(|| {
            checked_lifecycle(tokio_lifecycle::run(executor, case, evidence), expected)
        });
        push(
            out,
            "lifecycle_tokio",
            case.label(),
            case.nodes(),
            case.nodes().saturating_sub(1),
            sample,
            nanos,
            0,
            0,
            checksum,
        );
    }
    benchmark_lifecycle_profiles(out, "lifecycle_tokio_alloc", case, || {
        checked_lifecycle(tokio_lifecycle::run(executor, case, evidence), expected)
    });
}

fn benchmark_lifecycle_profiles(
    out: &mut Vec<Sample>,
    phase: &'static str,
    case: lifecycle::Case,
    mut run: impl FnMut() -> ((), u64),
) {
    let mut first = None;
    for sample in 0..5 {
        let (profile, checksum) = profiled(&mut run);
        assert_eq!(
            profile.retained_bytes, 0,
            "{phase}/{} retained requested bytes",
            case.label()
        );
        let observed = (profile, checksum);
        if let Some(expected) = first {
            assert_eq!(
                observed,
                expected,
                "{phase}/{} allocation profile is not deterministic",
                case.label()
            );
        } else {
            first = Some(observed);
        }
        out.push(Sample {
            phase,
            workload: case.label().to_owned(),
            nodes: case.nodes(),
            edges: case.nodes().saturating_sub(1),
            sample,
            nanos: 0,
            allocations: profile.allocations,
            allocated_bytes: profile.requested_bytes,
            peak_live_bytes: profile.peak_live_bytes,
            retained_bytes: profile.retained_bytes,
            checksum,
        });
    }
}
