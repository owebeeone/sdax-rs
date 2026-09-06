# sdax

`sdax` is a declarative async lifecycle for one process. You write a
**plan**: typed nodes (resources, steps, services, effects, …) joined by
`needs` edges. The engine derives when each node may start, the reverse
release graph, cancellation, and the report. You do not schedule waves
and you do not write teardown by hand.

This tree is how to use it. It is not the crate's development history,
and it is not `sdax::host` — you do not implement a runtime, a clock, or
the engine.

Code blocks tagged `rust,guide:<name>` are the **entire** file
`crates/sdax-tokio/tests/guide/<name>.rs`: the `#[test]`, the wrapper,
the assertions. That is what
`cargo test -p sdax-tokio --test guide` runs. They are tests on
purpose, not an application trimmed for the page.

## I want to…

| I want to… | Read… |
|---|---|
| Install and run a three-node plan | [Quick Start](https://github.com/owebeeone/sdax-rs/blob/main/docs/QuickStart.md) |
| Understand the graph, kinds, and policy | [Concepts](https://github.com/owebeeone/sdax-rs/blob/main/docs/Concepts.md) |
| Not leak a connection | [Cleanup](https://github.com/owebeeone/sdax-rs/blob/main/docs/Cleanup.md) |
| Declare every kind of node | [Authoring](https://github.com/owebeeone/sdax-rs/blob/main/docs/Authoring.md) |
| Start, stop, cancel, and drop a run | [Running](https://github.com/owebeeone/sdax-rs/blob/main/docs/Running.md) |
| Read `build` findings and a `Report` | [Errors](https://github.com/owebeeone/sdax-rs/blob/main/docs/Errors.md) |
| Inspect a plan or simulate a run | [Inspect](https://github.com/owebeeone/sdax-rs/blob/main/docs/Inspect.md) |
| Spawn a template at run time | [Instances](https://github.com/owebeeone/sdax-rs/blob/main/docs/Instances.md) |
| See a request, a service graph, a pipeline | [Cookbook](https://github.com/owebeeone/sdax-rs/blob/main/docs/Cookbook.md) |
| API reference | [Reference](https://github.com/owebeeone/sdax-rs/blob/main/docs/Reference.md) |

Use the [Quick Start dependency table](QuickStart.md) before crates.io
publication. After both Rust crates are published, install with
`cargo add sdax sdax-tokio`. The author surface is
`sdax::prelude` plus `sdax_tokio::{TokioRuntime, PlanStart}`. Start a
run with `plan.start(rt, input)` — `()` if the plan has none, a typed
value if you built it with `Plan::with_input`. Item-by-item
signatures live in rustdoc.
