use sdax::host::Observer;
use sdax::{
    Ambiguity, Backoff, Error, Mode, Outcome, Plan, PlanBuilder, Policy, Recovery, Report, Restart,
    Retry, Shutdown, TraceEvent,
};
use sdax_tokio::{PlanStart, TokioRuntime};
use std::future::poll_fn;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

const BUDGET: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Chain,
    Wide,
    Sparse,
}

#[derive(Clone, Debug)]
pub struct GraphSpec {
    pub shape: Shape,
    pub names: Vec<String>,
    pub parent: Vec<Option<usize>>,
}

impl GraphSpec {
    pub fn generate(shape: Shape, nodes: usize) -> Self {
        assert!(nodes > 0);
        let names = (0..nodes).map(|i| format!("N{i:04}")).collect();
        let parent = (0..nodes)
            .map(|i| match (shape, i) {
                (_, 0) => None,
                (Shape::Chain, _) => Some(i - 1),
                (Shape::Wide, _) => Some(0),
                (Shape::Sparse, _) => Some((i - 1) / 2),
            })
            .collect();
        GraphSpec {
            shape,
            names,
            parent,
        }
    }

    pub fn edges(&self) -> usize {
        self.parent.iter().filter(|p| p.is_some()).count()
    }

    pub fn label(&self) -> &'static str {
        match self.shape {
            Shape::Chain => "graph_chain",
            Shape::Wide => "graph_wide",
            Shape::Sparse => "graph_sparse",
        }
    }
}

pub fn graph_declaration(spec: &GraphSpec, yielding: bool) -> PlanBuilder<u64, u64> {
    let mut p = Plan::with_input::<u64>(spec.label());
    let input = p.input();
    let mut keys: Vec<sdax::Key<u64>> = Vec::with_capacity(spec.names.len());
    for (i, name) in spec.names.iter().enumerate() {
        let key = match spec.parent[i] {
            None => p.step(name).needs(input).run(move |_cx, value| async move {
                if yielding {
                    yield_once().await;
                }
                Ok(*value + i as u64)
            }),
            Some(parent) => {
                let dep = keys[parent];
                p.step(name).needs(dep).run(move |_cx, value| async move {
                    if yielding {
                        yield_once().await;
                    }
                    Ok(*value ^ i as u64)
                })
            }
        };
        keys.push(key);
    }
    p.export(*keys.last().expect("non-empty graph"))
}

pub fn graph(spec: &GraphSpec, yielding: bool) -> Plan<u64, u64> {
    graph_declaration(spec, yielding)
        .build(Policy::FailFast, Shutdown::within(BUDGET), Mode::Finite)
        .expect("graph fixture")
}

pub fn tiny_resource(releases: Arc<AtomicU64>, release_fails: bool) -> Plan<u64, u64> {
    let mut p = Plan::with_input::<u64>("tiny_resource");
    let input = p.input();
    let counter = releases.clone();
    let resource = p
        .resource("resource")
        .needs(input)
        .acquire(|cx, value| async move { Ok(cx.hold_value(*value)) })
        .release(move |_cx, _value| {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::Relaxed);
                if release_fails {
                    Err(
                        std::io::Error::new(std::io::ErrorKind::Other, "fixture release failure")
                            .into(),
                    )
                } else {
                    Ok(())
                }
            }
        });
    let output = p
        .step("use")
        .needs(resource)
        .run(|_cx, value| async move { Ok(*value + 1) });
    p.export(output)
        .build(Policy::FailFast, Shutdown::within(BUDGET), Mode::Finite)
        .expect("resource fixture")
}

pub fn startup_failure(releases: Arc<AtomicU64>) -> Plan<u64, u64> {
    let mut p = Plan::with_input::<u64>("startup_failure");
    let input = p.input();
    let counter = releases.clone();
    let upstream = p
        .resource("upstream")
        .needs(input)
        .acquire(|cx, value| async move { Ok(cx.hold_value(*value)) })
        .release(move |_cx, _value| {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
        });
    let failed = p
        .step("failed")
        .needs(upstream)
        .run(|_cx, _value| async move {
            Err::<u64, _>(
                std::io::Error::new(std::io::ErrorKind::Other, "fixture startup failure").into(),
            )
        });
    p.export(failed)
        .build(Policy::FailFast, Shutdown::within(BUDGET), Mode::Finite)
        .expect("startup failure fixture")
}

pub fn cleanup_error_keeps_upstream(
    downstream: Arc<AtomicU64>,
    upstream: Arc<AtomicU64>,
) -> Plan<u64, u64> {
    let mut p = Plan::with_input::<u64>("cleanup_error_upstream");
    let input = p.input();
    let up_count = upstream.clone();
    let up = p
        .resource("upstream")
        .needs(input)
        .acquire(|cx, value| async move { Ok(cx.hold_value(*value)) })
        .release(move |_cx, _value| {
            let up_count = up_count.clone();
            async move {
                up_count.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
        });
    let down_count = downstream.clone();
    let down = p
        .resource("downstream")
        .needs(up)
        .acquire(|cx, value| async move { Ok(cx.hold_value(*value)) })
        .release(move |_cx, _value| {
            let down_count = down_count.clone();
            async move {
                down_count.fetch_add(1, Ordering::Relaxed);
                Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "fixture downstream release failure",
                )
                .into())
            }
        });
    let out = p
        .step("use")
        .needs(down)
        .run(|_cx, value| async move { Ok(*value) });
    p.export(out)
        .build(Policy::FailFast, Shutdown::within(BUDGET), Mode::Finite)
        .expect("cleanup error fixture")
}

pub fn flat_component_equivalent(nodes: usize) -> Plan<u64, u64> {
    graph(&GraphSpec::generate(Shape::Chain, nodes), false)
}

pub fn nested_component(inner_nodes: usize) -> Plan<u64, u64> {
    let mut p = Plan::with_input::<u64>("nested_component");
    let input = p.input();
    let seed = p
        .step("seed")
        .needs(input)
        .run(|_cx, value| async move { Ok(*value) });
    let mut child_builder = Plan::builder("child_wrapper");
    let imported = child_builder.import(seed);
    let mut prior = imported;
    for i in 0..inner_nodes {
        prior = child_builder
            .step(&format!("inner_{i:04}"))
            .needs(prior)
            .run(move |_cx, value| async move { Ok(*value ^ i as u64) });
    }
    let wrapped = child_builder
        .export(prior)
        .build(Policy::FailFast, Shutdown::within(BUDGET), Mode::Finite)
        .expect("component child");
    let mounted = p.component("mounted", &wrapped, ());
    let out = p
        .step("out")
        .needs(mounted)
        .run(|_cx, value| async move { Ok(*value) });
    p.export(out)
        .build(Policy::FailFast, Shutdown::within(BUDGET), Mode::Finite)
        .expect("nested component fixture")
}

pub fn repeated_typed_components(mounts: usize) -> Plan<u64, u64> {
    assert!(mounts > 0);
    let mut child = Plan::with_input::<u64>("typed_child");
    let input = child.input();
    let output = child
        .step("increment")
        .needs(input)
        .run(|_cx, value| async move { Ok(*value + 1) });
    let child = child
        .export(output)
        .build(Policy::FailFast, Shutdown::within(BUDGET), Mode::Finite)
        .expect("typed component child");

    let mut parent = Plan::with_input::<u64>("repeated_typed_components");
    let mut prior = parent.input();
    for i in 0..mounts {
        prior = parent.component(&format!("mount_{i:04}"), &child, prior);
    }
    parent
        .export(prior)
        .build(Policy::FailFast, Shutdown::within(BUDGET), Mode::Finite)
        .expect("repeated typed component fixture")
}

pub fn repeated_resource_components(mounts: usize, releases: Arc<AtomicU64>) -> Plan<u64, u64> {
    assert!(mounts > 0);
    let mut child = Plan::with_input::<u64>("resource_child");
    let input = child.input();
    let release_count = releases.clone();
    let held = child
        .resource("held")
        .needs(input)
        .acquire(|cx, value| async move { Ok(cx.hold_value(*value)) })
        .release(move |_cx, _value| {
            let release_count = release_count.clone();
            async move {
                release_count.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
        });
    let output = child
        .step("increment")
        .needs(held)
        .run(|_cx, value| async move { Ok(*value + 1) });
    let child = child
        .export(output)
        .build(Policy::FailFast, Shutdown::within(BUDGET), Mode::Finite)
        .expect("resource component child");

    let mut parent = Plan::with_input::<u64>("repeated_resource_components");
    let mut prior = parent.input();
    for i in 0..mounts {
        prior = parent.component(&format!("mount_{i:04}"), &child, prior);
    }
    parent
        .export(prior)
        .build(Policy::FailFast, Shutdown::within(BUDGET), Mode::Finite)
        .expect("repeated resource component fixture")
}

pub fn cpu_application(nodes: usize) -> Plan<u64, u64> {
    let mut p = Plan::with_input::<u64>("application_hash");
    let input = p.input();
    let mut prior = p
        .step("seed")
        .needs(input)
        .run(|_cx, value| async move { Ok(*value) });
    for i in 1..nodes {
        prior = p
            .step(&format!("hash_{i:04}"))
            .needs(prior)
            .run(move |_cx, value| async move {
                let mut x = *value ^ i as u64;
                for _ in 0..64 {
                    x = x.wrapping_mul(0x9e37_79b9_7f4a_7c15).rotate_left(13) ^ 0xa5a5_a5a5;
                }
                Ok(x)
            });
    }
    p.export(prior)
        .build(Policy::FailFast, Shutdown::within(BUDGET), Mode::Finite)
        .expect("application fixture")
}

pub fn cancel_after_acquire(acquired: Arc<AtomicU64>, released: Arc<AtomicU64>) -> Plan<u64, u64> {
    let mut p = Plan::with_input::<u64>("cancel_after_acquire");
    let input = p.input();
    let acquired_mark = acquired.clone();
    let released_mark = released.clone();
    let resource = p
        .resource("resource")
        .needs(input)
        .acquire(move |cx, value| {
            let acquired_mark = acquired_mark.clone();
            async move {
                let held = cx.hold_value(*value);
                acquired_mark.fetch_add(1, Ordering::Release);
                std::future::pending::<()>().await;
                #[allow(unreachable_code)]
                Ok(held)
            }
        })
        .release(move |_cx, _value| {
            let released_mark = released_mark.clone();
            async move {
                released_mark.fetch_add(1, Ordering::Release);
                Ok(())
            }
        });
    let completed = p
        .step("completed")
        .needs(resource)
        .run(|_cx, value| async move { Ok(*value) });
    p.export(completed)
        .build(Policy::FailFast, Shutdown::within(BUDGET), Mode::Finite)
        .expect("cancel fixture")
}

pub fn known_receipt(compensated: Arc<AtomicU64>) -> Plan<u64, u64> {
    let mut p = Plan::with_input::<u64>("known_receipt");
    let input = p.input();
    let counter = compensated.clone();
    let receipt = p
        .effect("effect")
        .needs(input)
        .on_ambiguous(Ambiguity::Report)
        .perform(|cx, value| async move { Ok(cx.hold_value(*value)) })
        .compensate(move |_cx, _receipt| {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
        });
    p.export(receipt)
        .build(Policy::FailFast, Shutdown::within(BUDGET), Mode::Finite)
        .expect("known receipt fixture")
}

pub fn retry_once() -> Plan<u64, u64> {
    let mut p = Plan::with_input::<u64>("retry_once");
    let input = p.input();
    let output = p
        .step("retry")
        .needs(input)
        .retry(Retry::attempts(2))
        .idempotent()
        .run(|cx, value| async move {
            if cx.attempt() == 1 {
                Err(std::io::Error::new(std::io::ErrorKind::Other, "first attempt").into())
            } else {
                Ok(*value)
            }
        });
    p.export(output)
        .build(Policy::FailFast, Shutdown::within(BUDGET), Mode::Finite)
        .expect("retry fixture")
}

pub fn unknown_recovery(
    resolves: bool,
    started: Arc<AtomicU64>,
    recovered: Arc<AtomicU64>,
) -> Plan<(), u64> {
    let mut p = Plan::with_input::<u64>("unknown_recovery");
    let operation = p.input();
    p.effect("operation")
        .idempotent()
        .on_ambiguous(Ambiguity::Recover)
        .identified_by(operation)
        .perform(move |cx, ((), operation)| {
            let started = started.clone();
            async move {
                cx.hold(|| async move {
                    started.fetch_add(1, Ordering::Release);
                    let _operation = operation;
                    std::future::pending::<Result<u64, Error>>().await
                })
                .await
            }
        })
        .recover_unknown(move |_cx, operation| {
            let recovered = recovered.clone();
            async move {
                assert_eq!(*operation, 41);
                recovered.fetch_add(1, Ordering::Release);
                if resolves {
                    Ok(Recovery::Resolved)
                } else {
                    Err(
                        std::io::Error::new(std::io::ErrorKind::Other, "fixture recovery failure")
                            .into(),
                    )
                }
            }
        })
        .persistent();
    p.build(Policy::FailFast, Shutdown::within(BUDGET), Mode::Resident)
        .expect("unknown recovery fixture")
}

struct StableHandle(u64);

pub fn stable_service_recovery(
    exhausts: bool,
    initializes: Arc<AtomicU64>,
    episodes: Arc<AtomicU64>,
    first_handle: Arc<AtomicUsize>,
    handle_mismatches: Arc<AtomicU64>,
) -> Plan<(), ()> {
    let mut p = Plan::builder("stable_service_recovery");
    p.service("service")
        .idempotent()
        .restart(
            Restart::on_error(Backoff::fixed(Duration::ZERO)).max(if exhausts { 1 } else { 2 }),
        )
        .stop_within(Duration::from_secs(1))
        .initialize(move |_cx, ()| {
            initializes.fetch_add(1, Ordering::Release);
            async { Ok(StableHandle(17)) }
        })
        .serve(move |cx, handle| {
            let pointer = Arc::as_ptr(&handle) as usize;
            if cx.episode() == 1 {
                first_handle.store(pointer, Ordering::Release);
            } else if first_handle.load(Ordering::Acquire) != pointer {
                handle_mismatches.fetch_add(1, Ordering::Release);
            }
            assert_eq!(handle.0, 17);
            episodes.fetch_add(1, Ordering::Release);
            async move {
                if exhausts || cx.episode() < 3 {
                    Err(
                        std::io::Error::new(std::io::ErrorKind::Other, "fixture serving failure")
                            .into(),
                    )
                } else {
                    cx.stop().await;
                    Ok(())
                }
            }
        });
    p.build(Policy::FailFast, Shutdown::within(BUDGET), Mode::Resident)
        .expect("stable service recovery fixture")
}

pub fn dynamic_instances(instance_nodes: usize) -> Plan<(), usize> {
    let mut parent = Plan::with_input::<usize>("dynamic_instances");
    let count = parent.input();

    let mut child = Plan::with_input::<u64>("dynamic_child");
    let child_input = child.input();
    let mut prior = child
        .step("child_seed")
        .needs(child_input)
        .run(|_cx, value| async move { Ok(*value) });
    for i in 1..instance_nodes {
        prior = child
            .step(&format!("child_{i:04}"))
            .needs(prior)
            .run(move |_cx, value| async move { Ok(*value ^ i as u64) });
    }
    child
        .service("child_service")
        .needs(prior)
        .stop_within(Duration::from_secs(1))
        .initialize(|_cx, _value| async move { Ok(()) })
        .serve(|cx, _handle| async move {
            cx.stop().await;
            Ok(())
        });
    let child = child
        .build(Policy::FailFast, Shutdown::within(BUDGET), Mode::Resident)
        .expect("dynamic child");
    let template = parent.template("child", &child);
    parent
        .service("spawner")
        .needs(count)
        .spawns(&template)
        .stop_within(Duration::from_secs(2))
        .initialize(move |cx, count| async move {
            let mut children = Vec::with_capacity(*count);
            for i in 0..*count {
                let child = cx.spawn(&template, i as u64)?;
                child.ready().await?;
                children.push(child);
            }
            Ok(children)
        })
        .serve(|cx, _children| async move {
            cx.stop().await;
            Ok(())
        });
    parent
        .build(Policy::FailFast, Shutdown::within(BUDGET), Mode::Resident)
        .expect("dynamic fixture")
}

pub fn dynamic_sequential_churn(ended: Arc<AtomicU64>) -> Plan<(), usize> {
    let mut parent = Plan::with_input::<usize>("dynamic_sequential_churn");
    let count = parent.input();

    let mut child = Plan::with_input::<u64>("churn_child");
    let child_input = child.input();
    let released = ended.clone();
    let resource = child
        .resource("child_resource")
        .needs(child_input)
        .acquire(|cx, value| async move { Ok(cx.hold_value(*value)) })
        .release(move |_cx, _value| {
            let released = released.clone();
            async move {
                released.fetch_add(1, Ordering::Release);
                Ok(())
            }
        });
    child
        .service("child_service")
        .needs(resource)
        .stop_within(Duration::from_secs(1))
        .initialize(|_cx, _value| async move { Ok(()) })
        .serve(|cx, _handle| async move {
            cx.stop().await;
            Ok(())
        });
    let child = child
        .build(Policy::FailFast, Shutdown::within(BUDGET), Mode::Resident)
        .expect("churn child");
    let template = parent.template("churn_child", &child);
    parent
        .service("churner")
        .needs(count)
        .spawns(&template)
        .stop_within(Duration::from_secs(2))
        .initialize(move |cx, count| {
            let ended = ended.clone();
            async move {
                for i in 0..*count {
                    let target = ended.load(Ordering::Acquire) + 1;
                    let child = cx.spawn(&template, i as u64)?;
                    child.ready().await?;
                    child.stop();
                    poll_fn(|task| {
                        if ended.load(Ordering::Acquire) >= target {
                            Poll::Ready(())
                        } else {
                            task.waker().wake_by_ref();
                            Poll::Pending
                        }
                    })
                    .await;
                }
                Ok(())
            }
        })
        .serve(|cx, _handle| async move {
            cx.stop().await;
            Ok(())
        });
    parent
        .build(Policy::FailFast, Shutdown::within(BUDGET), Mode::Resident)
        .expect("sequential churn fixture")
}

pub struct CountingObserver(pub AtomicU64);

impl Observer for CountingObserver {
    fn event(&self, _event: &TraceEvent) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

pub fn adapter(
    executor: &tokio::runtime::Runtime,
    observer: Option<Arc<dyn Observer>>,
) -> Arc<TokioRuntime> {
    let rt = TokioRuntime::current_thread_no_background_drain(executor.handle().clone());
    Arc::new(match observer {
        Some(observer) => rt.with_observer(observer),
        None => rt,
    })
}

pub fn run_finite(
    executor: &tokio::runtime::Runtime,
    rt: Arc<TokioRuntime>,
    plan: &Plan<u64, u64>,
    input: u64,
) -> (Report<u64>, u64) {
    let report = executor.block_on(plan.start(rt, input));
    let checksum = report.output.as_deref().copied().unwrap_or(0)
        ^ report.faults.len() as u64
        ^ report.cleanup_failures.len() as u64
        ^ match report.outcome {
            Outcome::Ok => 1,
            Outcome::Failed => 2,
            Outcome::Cancelled => 3,
        };
    (report, checksum)
}

pub async fn yield_once() {
    let mut yielded = false;
    poll_fn(move |cx| {
        if yielded {
            Poll::Ready(())
        } else {
            yielded = true;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    })
    .await
}

pub fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("benchmark runtime")
}

pub fn resident_noop() -> Plan<(), u64> {
    let mut p = Plan::with_input::<u64>("resident_noop");
    let input = p.input();
    p.service("service")
        .needs(input)
        .stop_within(Duration::from_secs(1))
        .initialize(|_cx, _input| async move { Ok(()) })
        .serve(|cx, _handle| async move {
            cx.stop().await;
            Ok(())
        });
    p.build(Policy::FailFast, Shutdown::within(BUDGET), Mode::Resident)
        .expect("resident fixture")
}
