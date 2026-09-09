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

struct Base {
    mount: &'static str,
    amount: u32,
}

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
                        amount: base.amount + input.rate,
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

fn base_resource(
    parent: &mut PlanBuilder,
    name: &str,
    mount: &'static str,
    amount: u32,
    events: &Events,
) -> Key<Base> {
    let released = events.clone();
    parent
        .resource(name)
        .acquire(move |cx, ()| async move { Ok(cx.hold_value(Base { mount, amount })) })
        .release(move |_cx, base: Arc<Base>| {
            let released = released.clone();
            async move {
                released
                    .lock()
                    .expect("events")
                    .push((base.mount, "base release"));
                Ok(())
            }
        })
}

fn parent_with(
    left: QuoteInput,
    right: Option<QuoteInput>,
    events: &Events,
) -> Plan<(u32, Option<u32>)> {
    let (child, base_port) = quote_component(events);
    let mut parent = Plan::builder("pricing");
    let base = base_resource(&mut parent, "base", "parent", 100, events);
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

#[test]
fn repeated_definition_binds_distinct_resources_per_mount() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let (child, base_port) = quote_component(&events);
    let mut parent = Plan::builder("separate bases");
    let left_base = base_resource(&mut parent, "left base", "left base", 10, &events);
    let right_base = base_resource(&mut parent, "right base", "right base", 20, &events);
    let left_input = parent.step("left input").run(|_cx, ()| async move {
        Ok(QuoteInput {
            mount: "left",
            rate: 1,
            fail: Fail::Never,
            cleanup_fails: false,
        })
    });
    let right_input = parent.step("right input").run(|_cx, ()| async move {
        Ok(QuoteInput {
            mount: "right",
            rate: 2,
            fail: Fail::Never,
            cleanup_fails: false,
        })
    });
    let left_bound = child.bind(base_port, left_base).expect("left binding");
    let right_bound = child.bind(base_port, right_base).expect("right binding");
    let left = parent.component("left", &left_bound, left_input);
    let right = parent.component("right", &right_bound, right_input);
    let output = parent
        .step("outputs")
        .needs((left, right))
        .run(|_cx, values: (Arc<u32>, Arc<u32>)| async move { Ok((*values.0, Some(*values.1))) });
    let plan = build(parent.export(output));

    assert_eq!(output_at_string_boundary(run(&plan)), Ok((11, Some(22))));
    let events = events.lock().expect("events");
    assert!(
        position(&events, ("left", "rates release"))
            < position(&events, ("left base", "base release"))
    );
    assert!(
        position(&events, ("right", "rates release"))
            < position(&events, ("right base", "base release"))
    );
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
        .into_required_output()
        .map(|output| *output)
        .map_err(|error| error.to_string())
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
