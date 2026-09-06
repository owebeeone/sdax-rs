# Concepts

A **plan** is an immutable value. You build it once, validate it once,
and start it any number of times. Each `start` is a **run** with its
own slots and, when you declared one, its own typed input. Concurrent
runs of one plan are isolated; put request state in that input, or in
the values the nodes return — not in globals.

## The acquisition graph

Nodes are joined by typed `needs` edges. A node becomes eligible when
every key it needs is ready — not when a "wave" or a "level" finishes.
There are no start barriers. Two nodes that do not depend on each other
may overlap. The **release graph** is the reverse of `needs`: a node is
released only after every node that needed it has finished its own
cleanup. Unrelated releases may overlap.

`inspect()` shows earliest-start **layers**. Those are not barriers.
Nothing waits for a layer, only for its needs.

## Kinds

| Kind | What it is | Ready when | Cleanup |
|---|---|---|---|
| resource | acquired once, held | `acquire` returns `Held<T>` | `release` (or `release::by_drop()`) |
| step | finite work | `run` returns `Ok` | nothing |
| try-step | finite work whose failure is a value | `run` returns; dependents see `Arc<Result<T, E>>` | nothing |
| blocking step | synchronous work on a declared pool | `run` returns | nothing |
| service | long-lived | `start` returns `Serving` | stop, then the rest of the graph |
| effect | an externally visible action | `perform` returns `Held<R>` | `compensate`, or `persistent` |
| join | synchronisation only | every need is ready | nothing |
| component | a nested plan, once per parent run | the child plan is ready | the child's release graph |
| template | a nested plan factory | *(the node itself is never "ready")* | stop every live instance |

A resource or effect body must return `Held<T>`. Only `cx.hold` and
`cx.hold_value` can mint one. A service is ready when `start` *returns*
`Serving` — being spawned is not readiness.

## Policy is an argument

`build(policy, shutdown, mode)` requires all three.

- **`Policy::FailFast`** — the first fault stops admitting starts and
  cancels in-flight non-service nodes.
- **`Policy::Isolate`** — skip the fault's dependents; the rest of the
  run continues; every fault is aggregated.
- **`Shutdown::within(d)`** — from settling to ended, on the engine
  clock. **`Shutdown::unbounded()`** — every service must then declare
  `stop_within`.
- **`Mode::Finite`** — the run ends when every node has settled.
  Rejected if the plan has a service or a template.
- **`Mode::Resident`** — the run stays at steady state until
  `shutdown()`, `cancel()`, a `FailFast` fault, or a `terminal` service
  finishing.

## If you know Python sdax

There is no elevator and no level adapter. There are no waves. A node
does not have pre / execute / post phases — it has a kind, and that
kind has the bodies that make sense for it. `process_tasks(ctx)` is
`plan.start(rt, input)` → a `Running` you await for a `Report`. A
`Plan::builder` plan takes `()`; a `Plan::with_input` plan takes one
typed value, the same node a template reads from `cx.spawn`. Subtasks
are `cx.spawn(&template, input)`, not a TaskGroup.
