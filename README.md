# sdax

A declarative async lifecycle for one process: a typed **plan**, a
derived release graph, reverse teardown. You declare resources, steps,
services, effects and the `needs` edges between them. The engine
derives start eligibility, cleanup, cancellation, and the report.

How to use it: **[`docs`](https://github.com/owebeeone/sdax-rs/blob/main/docs/README.md)**.

`use sdax::prelude::*;` and `use sdax_tokio::PlanStart;` then
`plan.start(rt, input)`. A `Plan::builder` plan takes `()`. A
`Plan::with_input` plan takes one typed value per run. rustdoc is the
API dictionary. `sdax::host` is not the author surface and may change
before 1.0.

| crate | role |
|---|---|
| `sdax` | std-only core: authoring, validate, inspect, simulate. The crates.io package. |
| `sdax-tokio` | tokio adapter and run driver (`plan.start`, `Running`). |
| `sdax-testkit` | dev-only harness. `publish = false`. |

## Install from source

The Rust crates are not yet published on crates.io. Add both dependencies
from the same Git repository. Until the repository is public, cloning it
requires read access:

```toml
[dependencies]
sdax = { git = "https://github.com/owebeeone/sdax-rs", package = "sdax" }
sdax-tokio = { git = "https://github.com/owebeeone/sdax-rs", package = "sdax-tokio" }

[dev-dependencies]
tokio = { version = "=1.53.1", default-features = false, features = ["rt", "time", "test-util"] }
```

Cargo records the common resolved Git commit in your application's `Cargo.lock`.
The direct Tokio dev-dependency supplies the runtime and paused-time support used
by the [Quick Start test](https://github.com/owebeeone/sdax-rs/blob/main/docs/QuickStart.md).

After both crates are published, use registry dependencies instead:

```sh
cargo add sdax sdax-tokio
```

From a clone of this repository, run the workspace tests with:

```sh
cargo test --workspace --locked --offline
```

Release: [`RELEASE.md`](https://github.com/owebeeone/sdax-rs/blob/main/RELEASE.md). Contract and stage history:
`dev-docs/`. Version `0.1.0` is unreleased; nothing is frozen.
