# AI authoring reference

Use one `Plan` builder for values that must live together. Finish every
declaration with its terminal method before `build`. A finite plan exports
completed data, not a live resource or service that cleanup is about to end.

## Types and ownership

`Plan<Out, In>` takes one `In` at each root start, static mount, or dynamic
spawn and may export completed `Out`. Call `p.export(key)` before `build`;
merely declaring the last step does not select an output.

`Key<T>` is a typed declaration handle. A body that `.needs(key)` receives
`Arc<T>`. A body that `.needs((a, b))` receives `(Arc<A>, Arc<B>)`, while one
`Key<(A, B)>` produces `Arc<(A, B)>`. Return plain `T` from steps and service
initializers. Resource acquisition and effect performance return `Held<T>`.
Returning `Arc<T>` from an ordinary step deliberately makes its node
`Key<Arc<T>>`, so dependents receive `Arc<Arc<T>>`.

Both `cx.hold(factory).await` and `cx.hold_value(value)` consume the single-use
`Cx<Acquire>`. Put an external acquisition inside the lazy `hold` factory.
`hold_value` is for a value already owned without an external action. Both
accept an existing `Arc<T>` without nesting it, and expected `Held<T>` can use
an unsized `T`, such as `dyn Trait + Send + Sync`. Save `let shared =
cx.shared()` first if later code needs cancellation or clock operations.

A tuple or struct containing `Arc<Resource>` is ordinary data; it does not
create a visible cleanup dependency. Pass ordinary request data through the
plan input. For a parent-owned resource, declare `child.port::<T>(name)`, bind
it with `child.bind(port, parent_resource_key)`, and make every child resource
that uses it name the port in `.needs(...)`.

## Retry, effects, and services

Declare retry on the node. Two total attempts is
`.idempotent().retry(Retry::attempts(2))`; do not loop around or clone an
acquisition context. Each attempt receives fresh authority. `idempotent()` is
the application's claim that re-execution is safe.

An effect must state `on_ambiguous(Ambiguity::Report | Recover | Retry)`.
For recovery, put `.needs(...)` before `.identified_by(identity_key)`, then
call `.perform(...)` and `.recover_unknown(...)`. The perform dependency is
`(ordinary_dependencies, Arc<Identity>)`; the recovery handler receives only
`Arc<Identity>`. `Recovery::Resolved` discharges uncertainty. Recovery errors
can carry and display a domain operation ID; a generic still-unknown record
does not promise to format an arbitrary identity value.

A service initializer publishes one stable `Arc<H>`; each serving episode
uses it. `Retry` controls initialization attempts and `Restart` controls serve
episodes after the first. A serving restart does not rerun a successful
initializer or its dependents. Use `cx.spawn` for declared templates rather
than launching untracked tasks.

## Complete resource-bearing composition

This executable example keeps ordinary mount input separate from a formal
resource port. It computes `base + rate`, retries acquisition declaratively,
exports completed output, and verifies cleanup and mount isolation. The helper
`output_at_string_boundary` retains the complete `Report` until a caller
specifically requires `String`; textual rendering cannot preserve typed error
objects or downcasting.

```rust,guide:components
use sdax::prelude::*;
use sdax_tokio::{PlanStart, TokioRuntime};
use std::sync::{Arc, Mutex};
use std::time::Duration;

type Events = Arc<Mutex<Vec<(&'static str, &'static str)>>>;

#[derive(Clone, Copy)]
enum Fail {
    Never,
    Once,
    Always,
}

#[derive(Clone, Copy)]
struct QuoteInput {
    mount: &'static str,
    rate: u32,
    fail: Fail,
    cleanup_fails: bool,
}

struct Base(u32);

struct Rates {
    mount: &'static str,
    amount: u32,
}

struct Derived {
    mount: &'static str,
    amount: u32,
    cleanup_fails: bool,
}

fn build<O, I>(p: PlanBuilder<O, I>) -> Plan<O, I> {
    p.build(
        Policy::FailFast,
        Shutdown::within(Duration::from_secs(1)),
        Mode::Finite,
    )
    .expect("valid plan")
}

fn quote_component(events: &Events) -> (Plan<u32, QuoteInput>, Key<Base>) {
    let mut child = Plan::with_input::<QuoteInput>("quote");
    let input: Key<QuoteInput> = child.input();
    let base: Key<Base> = child.port::<Base>("base resource");

    let attempts = events.clone();
    let releases = events.clone();
    let rates: Key<Rates> = child
        .resource("rates")
        .needs((input, base))
        .idempotent()
        .retry(Retry::attempts(2))
        .acquire(move |cx, (input, base): (Arc<QuoteInput>, Arc<Base>)| {
            let attempts = attempts.clone();
            async move {
                attempts
                    .lock()
                    .expect("events")
                    .push((input.mount, "rates attempt"));
                let fails = matches!(input.fail, Fail::Always)
                    || matches!(input.fail, Fail::Once) && cx.attempt() == 1;
                if fails {
                    Err("rates unavailable".into())
                } else {
                    Ok(cx.hold_value(Rates {
                        mount: input.mount,
                        amount: base.0 + input.rate,
                    }))
                }
            }
        })
        .release(move |_cx, rates: Arc<Rates>| {
            let releases = releases.clone();
            async move {
                releases
                    .lock()
                    .expect("events")
                    .push((rates.mount, "rates release"));
                Ok(())
            }
        });

    let releases = events.clone();
    let derived: Key<Derived> = child
        .resource("derived")
        .needs((input, rates))
        .acquire(
            |cx, (input, rates): (Arc<QuoteInput>, Arc<Rates>)| async move {
                Ok(cx.hold_value(Derived {
                    mount: rates.mount,
                    amount: rates.amount,
                    cleanup_fails: input.cleanup_fails,
                }))
            },
        )
        .release(move |_cx, derived: Arc<Derived>| {
            let releases = releases.clone();
            async move {
                releases
                    .lock()
                    .expect("events")
                    .push((derived.mount, "derived release"));
                if derived.cleanup_fails {
                    Err("derived cleanup failed".into())
                } else {
                    Ok(())
                }
            }
        });
    let output = child
        .step("completed output")
        .needs(derived)
        .run(|_cx, value: Arc<Derived>| async move { Ok(value.amount) });
    (build(child.export(output)), base)
}

fn parent_with(
    left: QuoteInput,
    right: Option<QuoteInput>,
    events: &Events,
) -> Plan<(u32, Option<u32>)> {
    let (child, base_port) = quote_component(events);
    let mut parent = Plan::builder("pricing");
    let released = events.clone();
    let base = parent
        .resource("base")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Base(100))) })
        .release(move |_cx, _base: Arc<Base>| {
            let released = released.clone();
            async move {
                released
                    .lock()
                    .expect("events")
                    .push(("parent", "base release"));
                Ok(())
            }
        });
    let bound = child.bind(base_port, base).expect("base binding");
    let left_input = parent
        .step("left input")
        .run(move |_cx, ()| async move { Ok(left) });
    let left_output = parent.component("left", &bound, left_input);
    let right_output = right.map(|right| {
        let input = parent
            .step("right input")
            .run(move |_cx, ()| async move { Ok(right) });
        parent.component("right", &bound, input)
    });
    let output = match right_output {
        Some(right_output) => parent
            .step("both outputs")
            .needs((left_output, right_output))
            .run(
                |_cx, values: (Arc<u32>, Arc<u32>)| async move { Ok((*values.0, Some(*values.1))) },
            ),
        None => parent
            .step("one output")
            .needs(left_output)
            .run(|_cx, value: Arc<u32>| async move { Ok((*value, None)) }),
    };
    build(parent.export(output))
}

fn run(plan: &Plan<(u32, Option<u32>)>) -> Report<(u32, Option<u32>)> {
    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .expect("runtime");
    let runtime = Arc::new(TokioRuntime::current_thread_no_background_drain(
        tokio_rt.handle().clone(),
    ));
    tokio_rt.block_on(plan.start(runtime, ()))
}

fn output_at_string_boundary(
    report: Report<(u32, Option<u32>)>,
) -> Result<(u32, Option<u32>), String> {
    report
        .into_result()
        .map_err(|report| report.to_string())?
        .as_deref()
        .copied()
        .ok_or_else(|| "missing completed output".to_owned())
}

fn position(events: &[(&str, &str)], event: (&str, &str)) -> usize {
    events
        .iter()
        .position(|seen| *seen == event)
        .expect("event")
}

#[test]
fn resource_port_and_inputs_are_isolated_across_two_mounts() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let plan = parent_with(
        QuoteInput {
            mount: "left",
            rate: 2,
            fail: Fail::Once,
            cleanup_fails: false,
        },
        Some(QuoteInput {
            mount: "right",
            rate: 7,
            fail: Fail::Never,
            cleanup_fails: false,
        }),
        &events,
    );

    assert_eq!(output_at_string_boundary(run(&plan)), Ok((102, Some(107))));
    let events = events.lock().expect("events");
    assert_eq!(
        events
            .iter()
            .filter(|event| **event == ("left", "rates attempt"))
            .count(),
        2
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| **event == ("right", "rates attempt"))
            .count(),
        1
    );
    for mount in ["left", "right"] {
        assert!(
            position(&events, (mount, "derived release"))
                < position(&events, (mount, "rates release"))
        );
        assert!(
            position(&events, (mount, "rates release"))
                < position(&events, ("parent", "base release"))
        );
    }
}

#[test]
fn permanent_failure_attempts_twice_and_releases_the_base() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let plan = parent_with(
        QuoteInput {
            mount: "left",
            rate: 2,
            fail: Fail::Always,
            cleanup_fails: false,
        },
        None,
        &events,
    );

    let report = run(&plan);
    assert_eq!(report.outcome, Outcome::Failed);
    assert!(report
        .faults
        .iter()
        .any(|fault| fault.node.leaf() == "rates"));
    let events = events.lock().expect("events");
    assert_eq!(
        events
            .iter()
            .filter(|event| **event == ("left", "rates attempt"))
            .count(),
        2
    );
    assert!(events.contains(&("parent", "base release")));
}

#[test]
fn cleanup_failure_is_reported_while_upstream_cleanup_continues() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let plan = parent_with(
        QuoteInput {
            mount: "left",
            rate: 2,
            fail: Fail::Never,
            cleanup_fails: true,
        },
        None,
        &events,
    );

    let report = run(&plan);
    assert_eq!(report.cleanup_failures.len(), 1);
    assert_eq!(report.cleanup_failures[0].node.leaf(), "derived");
    assert!(report.cleanup_failures[0]
        .kind
        .to_string()
        .contains("derived cleanup failed"));
    let events = events.lock().expect("events");
    assert!(
        position(&events, ("left", "derived release"))
            < position(&events, ("left", "rates release"))
    );
    assert!(
        position(&events, ("left", "rates release"))
            < position(&events, ("parent", "base release"))
    );
}
```

The normal test proves each mount has its own input and retry state while both
borrow the bound parent resource. Dependency reversal orders each mount's
`derived` release before `rates`, and both before the parent `base`. Permanent
failure still releases acquired upstream state. A derived cleanup error remains
in `cleanup_failures` while rates and base cleanup continue.

All body errors use `sdax::Error`. When inference needs help, annotate the
result, for example `Err::<(), Error>(cause.into())`. Factories may run again
after declared retry or restart, so capture cloneable configuration and put
attempt-specific state inside the returned future.
