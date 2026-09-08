use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::runtime::Runtime;
use tokio::task::{JoinError, JoinHandle};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Ok,
    Failed,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Fault {
    node: &'static str,
    phase: &'static str,
    message: &'static str,
}

#[derive(Debug)]
struct Summary {
    outcome: Outcome,
    output: Option<u64>,
    faults: Vec<Fault>,
    cleanup_failures: Vec<Fault>,
    events: Vec<&'static str>,
    spawned: usize,
    joined: usize,
    active: usize,
    disposed: usize,
}

#[derive(Default)]
struct TaskCounts {
    spawned: AtomicUsize,
    joined: AtomicUsize,
    active: AtomicUsize,
}

struct ActiveTask(Arc<TaskCounts>);

impl Drop for ActiveTask {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::Relaxed);
    }
}

#[derive(Debug)]
struct Resource {
    value: u64,
    disposed: Arc<AtomicUsize>,
}

impl Drop for Resource {
    fn drop(&mut self) {
        self.disposed.fetch_add(1, Ordering::Relaxed);
    }
}

struct Run {
    tasks: Arc<TaskCounts>,
    events: Arc<Mutex<Vec<&'static str>>>,
    disposed: Arc<AtomicUsize>,
}

impl Run {
    fn new() -> Self {
        Run {
            tasks: Arc::new(TaskCounts::default()),
            events: Arc::new(Mutex::new(Vec::new())),
            disposed: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn spawn<T, F>(&self, future: F) -> JoinHandle<T>
    where
        T: Send + 'static,
        F: Future<Output = T> + Send + 'static,
    {
        self.tasks.spawned.fetch_add(1, Ordering::Relaxed);
        self.tasks.active.fetch_add(1, Ordering::Relaxed);
        let active = ActiveTask(self.tasks.clone());
        // This is the handwritten Tokio comparator: the task must be owned and
        // joined here so its drain cost is counted against that comparator.
        #[allow(clippy::disallowed_methods)]
        tokio::spawn(async move {
            let _active = active;
            future.await
        })
    }

    async fn join<T>(&self, task: JoinHandle<T>) -> Result<T, JoinError> {
        let result = task.await;
        self.tasks.joined.fetch_add(1, Ordering::Relaxed);
        result
    }

    async fn run<T, F>(&self, future: F) -> T
    where
        T: Send + 'static,
        F: Future<Output = T> + Send + 'static,
    {
        self.join(self.spawn(future))
            .await
            .expect("handwritten lifecycle task panicked")
    }

    fn finish(
        self,
        outcome: Outcome,
        output: Option<u64>,
        faults: Vec<Fault>,
        cleanup_failures: Vec<Fault>,
    ) -> Summary {
        Summary {
            outcome,
            output,
            faults,
            cleanup_failures,
            events: self.events.lock().unwrap().clone(),
            spawned: self.tasks.spawned.load(Ordering::Relaxed),
            joined: self.tasks.joined.load(Ordering::Relaxed),
            active: self.tasks.active.load(Ordering::Relaxed),
            disposed: self.disposed.load(Ordering::Relaxed),
        }
    }
}

async fn acquire(run: &Run, event: &'static str, value: u64) -> Resource {
    let events = run.events.clone();
    let disposed = run.disposed.clone();
    run.run(async move {
        events.lock().unwrap().push(event);
        Resource { value, disposed }
    })
    .await
}

async fn release(run: &Run, event: &'static str, resource: Resource) {
    let events = run.events.clone();
    run.run(async move {
        events.lock().unwrap().push(event);
        drop(resource);
    })
    .await;
}

async fn normal() -> Summary {
    let run = Run::new();
    let resource = acquire(&run, "acquire_resource", 41).await;
    let events = run.events.clone();
    let value = resource.value;
    let output = run
        .run(async move {
            events.lock().unwrap().push("use_resource");
            value + 1
        })
        .await;
    release(&run, "release_resource", resource).await;
    run.finish(Outcome::Ok, Some(output), Vec::new(), Vec::new())
}

async fn startup_failure() -> Summary {
    let run = Run::new();
    let upstream = acquire(&run, "acquire_upstream", 41).await;
    let events = run.events.clone();
    let result = run
        .run(async move {
            events.lock().unwrap().push("fail_downstream_run");
            Err::<Resource, _>("fixture startup failure")
        })
        .await;
    let error = result.unwrap_err();
    release(&run, "release_upstream", upstream).await;
    run.finish(
        Outcome::Failed,
        None,
        vec![Fault {
            node: "failed",
            phase: "run",
            message: error,
        }],
        Vec::new(),
    )
}

async fn cleanup_failure() -> Summary {
    let run = Run::new();
    let upstream = acquire(&run, "acquire_upstream", 41).await;
    let downstream = acquire(&run, "acquire_downstream", upstream.value).await;
    let output = run.run(std::future::ready(downstream.value)).await;
    let events = run.events.clone();
    let result = run
        .run(async move {
            events.lock().unwrap().push("fail_downstream_release");
            drop(downstream);
            Err::<(), _>("fixture downstream release failure")
        })
        .await;
    let error = result.unwrap_err();
    release(&run, "release_upstream", upstream).await;
    run.finish(
        Outcome::Ok,
        Some(output),
        Vec::new(),
        vec![Fault {
            node: "downstream",
            phase: "release",
            message: error,
        }],
    )
}

async fn cancellation_after_acquisition() -> Summary {
    let run = Run::new();
    let held = Arc::new(Mutex::new(None));
    let held_by_task = held.clone();
    let events = run.events.clone();
    let disposed = run.disposed.clone();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let pending = run.spawn(async move {
        events.lock().unwrap().push("hold_resource");
        *held_by_task.lock().unwrap() = Some(Resource {
            value: 41,
            disposed,
        });
        let _ = started_tx.send(());
        std::future::pending::<()>().await;
    });
    started_rx.await.expect("acquisition reports held resource");
    pending.abort();
    assert!(run.join(pending).await.unwrap_err().is_cancelled());
    let resource = held.lock().unwrap().take().expect("resource was held");
    release(&run, "release_resource", resource).await;
    run.finish(Outcome::Cancelled, None, Vec::new(), Vec::new())
}

struct Expected {
    outcome: Outcome,
    output: Option<u64>,
    faults: Vec<Fault>,
    cleanup_failures: Vec<Fault>,
    events: Vec<&'static str>,
    tasks: usize,
    disposed: usize,
}

fn normal_expected() -> Expected {
    Expected {
        outcome: Outcome::Ok,
        output: Some(42),
        faults: Vec::new(),
        cleanup_failures: Vec::new(),
        events: vec!["acquire_resource", "use_resource", "release_resource"],
        tasks: 3,
        disposed: 1,
    }
}

fn startup_expected() -> Expected {
    Expected {
        outcome: Outcome::Failed,
        output: None,
        faults: vec![Fault {
            node: "failed",
            phase: "run",
            message: "fixture startup failure",
        }],
        cleanup_failures: Vec::new(),
        events: vec![
            "acquire_upstream",
            "fail_downstream_run",
            "release_upstream",
        ],
        tasks: 3,
        disposed: 1,
    }
}

fn cleanup_expected() -> Expected {
    Expected {
        outcome: Outcome::Ok,
        output: Some(41),
        faults: Vec::new(),
        cleanup_failures: vec![Fault {
            node: "downstream",
            phase: "release",
            message: "fixture downstream release failure",
        }],
        events: vec![
            "acquire_upstream",
            "acquire_downstream",
            "fail_downstream_release",
            "release_upstream",
        ],
        tasks: 5,
        disposed: 2,
    }
}

fn cancelled_expected() -> Expected {
    Expected {
        outcome: Outcome::Cancelled,
        output: None,
        faults: Vec::new(),
        cleanup_failures: Vec::new(),
        events: vec!["hold_resource", "release_resource"],
        tasks: 2,
        disposed: 1,
    }
}

fn check(summary: &Summary, expected: &Expected) -> Result<(), &'static str> {
    if summary.outcome != expected.outcome {
        return Err("outcome");
    }
    if summary.output != expected.output {
        return Err("output");
    }
    if summary.faults != expected.faults {
        return Err("faults");
    }
    if summary.cleanup_failures != expected.cleanup_failures {
        return Err("cleanup_failures");
    }
    if summary.events != expected.events {
        return Err("events");
    }
    if summary.spawned != expected.tasks {
        return Err("spawned");
    }
    if summary.joined != expected.tasks {
        return Err("joined");
    }
    if summary.active != 0 {
        return Err("active");
    }
    if summary.disposed != expected.disposed {
        return Err("disposed");
    }
    Ok(())
}

pub fn verify(executor: &Runtime) {
    check(&executor.block_on(normal()), &normal_expected()).unwrap();
    check(&executor.block_on(startup_failure()), &startup_expected()).unwrap();
    check(&executor.block_on(cleanup_failure()), &cleanup_expected()).unwrap();
    check(
        &executor.block_on(cancellation_after_acquisition()),
        &cancelled_expected(),
    )
    .unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn independent_negative_controls_reject_wrong_order_fault_disposal_and_drain() {
        let executor = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();

        let mut value = executor.block_on(normal());
        assert_eq!(check(&value, &normal_expected()), Ok(()));
        value.events.swap(1, 2);
        assert_eq!(check(&value, &normal_expected()), Err("events"));
        let mut value = executor.block_on(normal());
        value.output = Some(99);
        assert_eq!(check(&value, &normal_expected()), Err("output"));

        let mut value = executor.block_on(startup_failure());
        value.faults[0].phase = "prepare";
        assert_eq!(check(&value, &startup_expected()), Err("faults"));

        let mut value = executor.block_on(cleanup_failure());
        value.cleanup_failures[0].message = "incomplete failure";
        assert_eq!(check(&value, &cleanup_expected()), Err("cleanup_failures"));
        let mut value = executor.block_on(cleanup_failure());
        value.disposed -= 1;
        assert_eq!(check(&value, &cleanup_expected()), Err("disposed"));

        let mut value = executor.block_on(cancellation_after_acquisition());
        value.active += 1;
        assert_eq!(check(&value, &cancelled_expected()), Err("active"));
    }
}
