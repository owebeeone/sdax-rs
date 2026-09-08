use crate::lifecycle::{
    check, expected, normalize_error, Case, Evidence, Fault, FixtureError, Outcome, Summary,
    TrackedResource,
};
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::runtime::Runtime;
use tokio::task::{JoinError, JoinHandle};

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

struct Run {
    tasks: Arc<TaskCounts>,
    evidence: Evidence,
}

impl Run {
    fn new(evidence: Evidence) -> Self {
        Run {
            tasks: Arc::new(TaskCounts::default()),
            evidence,
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
        // The handwritten comparator owns and joins this task; its drain cost is
        // inside the comparator boundary.
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
        let events = self.evidence.events();
        Summary {
            outcome,
            output,
            faults,
            cleanup_failures,
            internal_records: events.len()
                + self.tasks.spawned.load(Ordering::Relaxed)
                + self.tasks.joined.load(Ordering::Relaxed),
            events,
            spawned: self.tasks.spawned.load(Ordering::Relaxed),
            joined: self.tasks.joined.load(Ordering::Relaxed),
            active: self.tasks.active.load(Ordering::Relaxed),
            disposed: self.evidence.disposed(),
        }
    }
}

async fn acquire(run: &Run, event: &'static str, value: u64) -> TrackedResource {
    let evidence = run.evidence.clone();
    run.run(async move {
        evidence.record(event);
        evidence.resource(value)
    })
    .await
}

async fn release(run: &Run, event: &'static str, resource: TrackedResource) {
    let evidence = run.evidence.clone();
    run.run(async move {
        evidence.record(event);
        drop(resource);
    })
    .await;
}

async fn normal(evidence: Evidence) -> Summary {
    let run = Run::new(evidence);
    let resource = acquire(&run, "acquire_resource", 41).await;
    let use_evidence = run.evidence.clone();
    let value = resource.value;
    let output = run
        .run(async move {
            use_evidence.record("use_resource");
            value + 1
        })
        .await;
    release(&run, "release_resource", resource).await;
    run.finish(Outcome::Ok, Some(output), Vec::new(), Vec::new())
}

async fn startup_failure(evidence: Evidence) -> Summary {
    let run = Run::new(evidence);
    let upstream = acquire(&run, "acquire_upstream", 41).await;
    let fail_evidence = run.evidence.clone();
    let result = run
        .run(async move {
            fail_evidence.record("fail_downstream_run");
            Err::<TrackedResource, _>(FixtureError::startup())
        })
        .await;
    let error = match result {
        Ok(_) => panic!("startup fixture unexpectedly succeeded"),
        Err(error) => error,
    };
    let fault = normalize_error("failed", 1, "run", &error);
    release(&run, "release_upstream", upstream).await;
    run.finish(Outcome::Failed, None, vec![fault], Vec::new())
}

async fn cleanup_failure(evidence: Evidence) -> Summary {
    let run = Run::new(evidence);
    let upstream = acquire(&run, "acquire_upstream", 41).await;
    let downstream = acquire(&run, "acquire_downstream", upstream.value).await;
    let use_evidence = run.evidence.clone();
    let value = downstream.value;
    let output = run
        .run(async move {
            use_evidence.record("use_resource");
            value
        })
        .await;
    let fail_evidence = run.evidence.clone();
    let result = run
        .run(async move {
            fail_evidence.record("fail_downstream_release");
            drop(downstream);
            Err::<(), _>(FixtureError::cleanup())
        })
        .await;
    let error = result.unwrap_err();
    let fault = normalize_error("downstream", 1, "release", &error);
    release(&run, "release_upstream", upstream).await;
    run.finish(Outcome::Ok, Some(output), Vec::new(), vec![fault])
}

async fn cancellation_after_acquisition(
    evidence: Evidence,
    ready: tokio::sync::oneshot::Receiver<()>,
) -> Summary {
    let run = Run::new(evidence);
    let held = Arc::new(Mutex::new(None));
    let held_by_task = held.clone();
    let task_evidence = run.evidence.clone();
    let pending = run.spawn(async move {
        task_evidence.record("hold_resource");
        *held_by_task.lock().unwrap() = Some(task_evidence.resource(41));
        task_evidence.signal_ready();
        std::future::pending::<()>().await;
    });
    ready.await.expect("acquisition reports held resource");
    pending.abort();
    assert!(run.join(pending).await.unwrap_err().is_cancelled());
    let resource = held.lock().unwrap().take().expect("resource was held");
    release(&run, "release_resource", resource).await;
    run.finish(Outcome::Cancelled, None, Vec::new(), Vec::new())
}

pub fn run(executor: &Runtime, case: Case, evidence: &Evidence) -> Summary {
    let ready = evidence.begin(case);
    executor.block_on(async {
        match case {
            Case::Normal => normal(evidence.clone()).await,
            Case::StartupFailure => startup_failure(evidence.clone()).await,
            Case::CleanupFailure => cleanup_failure(evidence.clone()).await,
            Case::Cancellation => {
                cancellation_after_acquisition(
                    evidence.clone(),
                    ready.expect("cancellation receiver"),
                )
                .await
            }
        }
    })
}

pub fn verify(executor: &Runtime) {
    for case in Case::ALL {
        let evidence = Evidence::new();
        check(&run(executor, case, &evidence), &expected(case)).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn independent_negative_controls_reject_wrong_summary_fields() {
        let executor = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();

        let evidence = Evidence::new();
        let mut value = run(&executor, Case::Normal, &evidence);
        assert_eq!(check(&value, &expected(Case::Normal)), Ok(()));
        value.events.swap(1, 2);
        assert_eq!(check(&value, &expected(Case::Normal)), Err("events"));
        let mut value = run(&executor, Case::Normal, &evidence);
        value.output = Some(99);
        assert_eq!(check(&value, &expected(Case::Normal)), Err("output"));

        let mut value = run(&executor, Case::StartupFailure, &evidence);
        assert_eq!(value.faults[0].sources, ["fixture startup source"]);
        value.faults[0].sources.clear();
        assert_eq!(
            check(&value, &expected(Case::StartupFailure)),
            Err("faults")
        );
        let mut value = run(&executor, Case::StartupFailure, &evidence);
        value.faults[0].typed_fixture_error = false;
        assert_eq!(
            check(&value, &expected(Case::StartupFailure)),
            Err("faults")
        );

        let mut value = run(&executor, Case::CleanupFailure, &evidence);
        value.cleanup_failures[0].message = "incomplete failure".to_owned();
        assert_eq!(
            check(&value, &expected(Case::CleanupFailure)),
            Err("cleanup_failures")
        );
        let mut value = run(&executor, Case::CleanupFailure, &evidence);
        value.disposed -= 1;
        assert_eq!(
            check(&value, &expected(Case::CleanupFailure)),
            Err("disposed")
        );

        let mut value = run(&executor, Case::Cancellation, &evidence);
        value.active += 1;
        assert_eq!(
            check(&value, &expected(Case::Cancellation)),
            Err("active")
        );
        let mut value = run(&executor, Case::Cancellation, &evidence);
        value.joined -= 1;
        assert_eq!(
            check(&value, &expected(Case::Cancellation)),
            Err("joined")
        );
    }
}
