//! Publication failure must settle host obligations without abandoning tasks.
use sdax::host::{bodies_of, BodySource, CxInner, InstanceId, RawKey, Task};
use sdax::*;
use sdax_tokio::{PlanStart, RunOptions, RunRecord, TokioRuntime};
use std::any::Any;
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct RejectPublication {
    inner: Arc<dyn BodySource>,
    reject: Option<RawKey>,
    suppress: Option<RawKey>,
    calls: Arc<Mutex<Vec<RawKey>>>,
}
impl BodySource for RejectPublication {
    fn publish_ready(&self, node: RawKey, instance: Option<InstanceId>) -> Result<(), Error> {
        self.calls.lock().unwrap().push(node);
        if Some(node) == self.reject {
            Err(Box::new(std::io::Error::other("publication rejected")))
        } else {
            self.inner.publish_ready(node, instance)
        }
    }
    fn body(&self, node: RawKey, instance: Option<InstanceId>, cx: &Arc<CxInner>) -> Option<Task> {
        if Some(node) == self.suppress {
            // Host fault injection: claim body completion without storing its
            // declared value, so component publication must detect the hole.
            Some(Task::Async(Box::pin(async { Ok(()) })))
        } else {
            self.inner.body(node, instance, cx)
        }
    }
    fn cleanup(
        &self,
        node: RawKey,
        instance: Option<InstanceId>,
        cx: &Arc<CxInner>,
    ) -> Option<Task> {
        self.inner.cleanup(node, instance, cx)
    }
    fn store(&self, node: RawKey, instance: Option<InstanceId>, value: Box<dyn Any + Send + Sync>) {
        self.inner.store(node, instance, value)
    }
    fn export(&self) -> Option<Box<dyn Any + Send + Sync>> {
        self.inner.export()
    }
    fn open_instance(
        &self,
        template: RawKey,
        parent: Option<InstanceId>,
        id: InstanceId,
        input: Box<dyn Any + Send + Sync>,
    ) {
        self.inner.open_instance(template, parent, id, input)
    }
    fn close_instance(&self, id: InstanceId) {
        self.inner.close_instance(id)
    }
}

fn drive(
    plan: &Plan,
    reject: Option<RawKey>,
    suppress: Option<RawKey>,
) -> (Report<()>, RunRecord, Vec<RawKey>) {
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .unwrap();
    let rt = Arc::new(TokioRuntime::current_thread_no_background_drain(
        executor.handle().clone(),
    ));
    let record = Arc::new(Mutex::new(RunRecord::default()));
    let calls = Arc::new(Mutex::new(Vec::new()));
    let source = Arc::new(RejectPublication {
        inner: bodies_of::<(), ()>(plan),
        reject,
        suppress,
        calls: calls.clone(),
    });
    let report = executor.block_on(async {
        let running = plan.start_with(
            rt.clone(),
            (),
            RunOptions::new().bodies(source).record(record.clone()),
        );
        let report = tokio::time::timeout(Duration::from_secs(30), running)
            .await
            .expect("publication failure must not strand a spawned task");
        rt.shutdown(Duration::from_secs(1)).await.unwrap();
        assert_eq!(rt.tracked(), 0);
        report
    });
    let record = Arc::try_unwrap(record).ok().unwrap().into_inner().unwrap();
    let calls = Arc::try_unwrap(calls).unwrap().into_inner().unwrap();
    (report, record, calls)
}

#[test]
fn failure_acknowledgements_follow_the_whole_spawn_batch() {
    let mut builder = Plan::builder("Batch");
    let first = builder.join("first", ());
    let second = builder.join("second", ());
    builder
        .step("pending")
        .run(|_, ()| async { std::future::pending::<Result<(), Error>>().await });
    let plan = builder
        .build(Policy::FailFast, Shutdown::unbounded(), Mode::Finite)
        .unwrap();
    let (report, record, calls) = drive(&plan, Some(first.raw()), None);
    assert_eq!(report.outcome, Outcome::Failed);
    assert_eq!(calls, [first.raw(), second.raw()]);
    assert!(record.rejections.is_empty(), "{:?}", record.rejections);
    let begin = &record.steps[0];
    assert_eq!(
        begin
            .effects
            .iter()
            .filter(|effect| effect.starts_with("PublishReady"))
            .count(),
        2
    );
    assert!(begin
        .effects
        .iter()
        .any(|effect| effect.starts_with("Spawn {")));
}

#[test]
fn failed_publication_blocks_consumers_and_releases_held_resources() {
    let counts = Arc::new(Mutex::new((0usize, 0usize)));
    let mut builder = Plan::builder("Cleanup");
    let released = counts.clone();
    let resource = builder
        .resource("resource")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(())) })
        .release(move |_, _| {
            let released = released.clone();
            async move {
                released.lock().unwrap().0 += 1;
                Ok(())
            }
        });
    let join = builder.join("join", resource);
    let consumed = counts.clone();
    builder
        .step("consumer")
        .needs(join)
        .run(move |_, _: Arc<()>| {
            consumed.lock().unwrap().1 += 1;
            async { Ok(()) }
        });
    let plan = builder
        .build(Policy::FailFast, Shutdown::unbounded(), Mode::Finite)
        .unwrap();
    let (report, record, calls) = drive(&plan, Some(join.raw()), None);
    assert_eq!(report.outcome, Outcome::Failed);
    assert_eq!(calls, [join.raw()]);
    assert_eq!(*counts.lock().unwrap(), (1, 0));
    assert_eq!(report.faults.len(), 1);
    assert!(report.cleanup_failures.is_empty());
    assert!(report.output.is_none());
    assert!(record.rejections.is_empty());
}

fn missing_component_export<T: Default + Send + Sync + 'static>() {
    let released = Arc::new(Mutex::new(0usize));
    let count_releases = released.clone();
    let mut child = Plan::builder("Child");
    let resource = child
        .resource("held")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(())) })
        .release(move |_, _| {
            let count = count_releases.clone();
            async move {
                *count.lock().unwrap() += 1;
                Ok(())
            }
        });
    let value = child
        .step("value")
        .needs(resource)
        .run(|_, _: Arc<()>| async { Ok(T::default()) });
    let child = child
        .export(value)
        .build(Policy::FailFast, Shutdown::unbounded(), Mode::Finite)
        .unwrap();
    let mut parent = Plan::builder("Parent");
    let component = parent.component("child", &child);
    let consumed = Arc::new(Mutex::new(false));
    let count = consumed.clone();
    parent
        .step("consumer")
        .needs(component)
        .run(move |_, _: Arc<T>| {
            *count.lock().unwrap() = true;
            async { Ok(()) }
        });
    let plan = parent
        .build(Policy::FailFast, Shutdown::unbounded(), Mode::Finite)
        .unwrap();
    let (report, record, calls) = drive(&plan, None, Some(value.raw()));
    assert_eq!(report.outcome, Outcome::Failed);
    assert_eq!(calls, [component.raw()]);
    assert!(!*consumed.lock().unwrap());
    assert_eq!(*released.lock().unwrap(), 1);
    assert!(report.cleanup_failures.is_empty());
    assert_eq!(report.faults.len(), 1);
    assert!(record.rejections.is_empty());
    assert!(report.output.is_none());
}

#[test]
fn missing_component_export_is_a_host_fault_even_for_explicit_unit() {
    missing_component_export::<u32>();
    missing_component_export::<()>();
}
