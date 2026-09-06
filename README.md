# sdax

A declarative async lifecycle for one process: a typed **plan**, a
derived release graph, reverse teardown. You declare resources, steps,
services, effects and the `needs` edges between them. The engine
derives start eligibility, cleanup, cancellation, and the report.

How to use it: **[`docs/`](docs/README.md)**.

```sh
cargo add sdax sdax-tokio
```

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

```sh
cargo test --workspace --locked --offline
```

Release: [`RELEASE.md`](RELEASE.md). Contract and stage history:
`dev-docs/`. Version `0.1.0` is unreleased; nothing is frozen.
