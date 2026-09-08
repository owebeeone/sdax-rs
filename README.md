# sdax

`sdax` declares the lifecycle of one async process: its resources, finite
work, resident services, effects, and the dependencies between them. The
engine starts eligible work, tears down acquired values in reverse dependency
order, and returns one complete report.

The public guides live in the [documentation](https://github.com/owebeeone/sdax-rs/tree/main/docs).
Rustdoc is the API dictionary. `sdax::host` is not an authoring surface.

| crate | role |
|---|---|
| [`sdax`](https://github.com/owebeeone/sdax-rs/tree/main/crates/sdax) | std-only plan authoring, validation, inspection, and simulation |
| [`sdax-tokio`](https://github.com/owebeeone/sdax-rs/tree/main/crates/sdax-tokio) | Tokio runtime adapter and run driver |
| [`sdax-testkit`](https://github.com/owebeeone/sdax-rs/tree/main/crates/sdax-testkit) | development-only lifecycle test harness |

## Install from source

The crates are not yet published on crates.io. Add both dependencies from the
same repository:

```toml
[dependencies]
sdax = { git = "https://github.com/owebeeone/sdax-rs", package = "sdax" }
sdax-tokio = { git = "https://github.com/owebeeone/sdax-rs", package = "sdax-tokio" }

[dev-dependencies]
tokio = { version = "=1.53.1", default-features = false, features = ["rt", "time"] }
```

Cargo pins the shared Git revision in your application's `Cargo.lock`. After
publication, use `cargo add sdax sdax-tokio`.

## A complete lifecycle

This test makes a working directory, writes a report, and releases the
directory after both a successful run and a failed run. The acquisition happens
inside `cx.hold`, so when its future reports success the engine records the
cleanup obligation in that same poll. The failure case keeps the whole `Report` by using
`into_result`, then inspects its fault and cleanup details.

```rust
use sdax::prelude::*;
use sdax_tokio::{PlanStart, TokioRuntime};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Clone)]
struct Request {
    name: &'static str,
    reject: bool,
}

struct WorkDir(PathBuf);

fn workspace(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("sdax-readme-{name}-{nonce}"))
}

#[test]
fn cleans_up_after_success_and_failure() {
    let success_dir = workspace("success");
    let failure_dir = workspace("failure");
    let success_dir_for_plan = success_dir.clone();
    let failure_dir_for_plan = failure_dir.clone();
    let mut p = Plan::with_input::<Request>("write a report");
    let request = p.input();
    let directory = p
        .resource("work directory")
        .needs(request)
        .acquire(move |cx, request: Arc<Request>| {
            let path = if request.reject {
                failure_dir_for_plan.clone()
            } else {
                success_dir_for_plan.clone()
            };
            async move {
                cx.hold(|| async move {
                    std::fs::create_dir_all(&path)?;
                    Ok::<WorkDir, Error>(WorkDir(path))
                })
                .await
            }
        })
        .release(|_cx, directory: Arc<WorkDir>| async move {
            std::fs::remove_dir_all(&directory.0)?;
            Ok(())
        });
    let bytes = p
        .step("write report")
        .needs((directory, request))
        .run(|_, (directory, request): (Arc<WorkDir>, Arc<Request>)| async move {
            std::fs::write(directory.0.join("report.txt"), request.name)?;
            if request.reject {
                Err::<usize, Error>("the report was rejected".into())
            } else {
                Ok(request.name.len())
            }
        });
    let plan = p
        .export(bytes)
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
    let rt = Arc::new(TokioRuntime::current_thread_no_background_drain(
        tokio_rt.handle().clone(),
    ));

    let success = tokio_rt.block_on(plan.start(
        rt.clone(),
        Request {
            name: "accepted",
            reject: false,
        },
    ));
    assert_eq!(success.into_result().expect("clean run").as_deref(), Some(&8));
    assert!(!success_dir.exists(), "the successful run released its directory");

    let failure = tokio_rt.block_on(plan.start(
        rt,
        Request {
            name: "rejected",
            reject: true,
        },
    ));
    let failure = failure.into_result().expect_err("the step failed");
    assert_eq!(failure.outcome, Outcome::Failed);
    assert_eq!(failure.faults.len(), 1);
    assert_eq!(failure.faults[0].node.leaf(), "write report");
    assert!(failure.cleanup_failures.is_empty(), "cleanup still succeeded");
    assert!(!failure_dir.exists(), "the failed run released its directory");
}
```

The [AI authoring reference](https://github.com/owebeeone/sdax-rs/blob/main/docs/AI-Authoring.md)
collects the exact service, component, acquisition, and unknown-outcome recovery
forms. For more examples, start with the [quick start](https://github.com/owebeeone/sdax-rs/blob/main/docs/QuickStart.md)
and the [cleanup guide](https://github.com/owebeeone/sdax-rs/blob/main/docs/Cleanup.md).

From a clone of this repository, run:

```sh
cargo test --workspace --locked --offline
```

The release process is documented in [RELEASE.md](https://github.com/owebeeone/sdax-rs/blob/main/RELEASE.md).
