use sdax::prelude::*;
use sdax::{Invalid, Rule};
use std::time::Duration;

fn bounded() -> Shutdown {
    Shutdown::within(Duration::from_secs(2))
}

fn leaf(stop_bounded: bool) -> Plan<()> {
    let mut p = Plan::builder("Leaf plan");
    let service = p.service("Worker");
    let service = if stop_bounded {
        service.stop_within(Duration::from_secs(1))
    } else {
        service
    };
    service.start(|_cx, ()| async { Ok(Serving::new((), std::future::pending())) });
    p.build(Policy::FailFast, bounded(), Mode::Resident)
        .unwrap()
}

// Exercise every component/template combination down to four boundaries.
// Child budgets are bounded: they do not start until their release graph opens.
fn tree(
    depth: usize,
    templates: usize,
    stop_bounded: bool,
    shutdown: Shutdown,
) -> Result<Plan<()>, Invalid> {
    let mut child = leaf(stop_bounded);
    for level in (0..depth).rev() {
        let mut parent = Plan::builder("Container plan");
        let name = format!("Level{level}");
        if templates & (1 << level) != 0 {
            parent.template(&name, &child);
        } else {
            parent.component(&name, &child);
        }
        let result = parent.build(
            Policy::FailFast,
            if level == 0 { shutdown } else { bounded() },
            Mode::Resident,
        );
        if level == 0 {
            return result;
        }
        child = result.unwrap();
    }
    unreachable!()
}

#[test]
fn unbounded_root_rejects_services_through_every_child_boundary() {
    for depth in 1..=4 {
        for templates in 0..(1 << depth) {
            let invalid = tree(depth, templates, false, Shutdown::unbounded()).unwrap_err();
            assert_eq!(invalid.checks.len(), 1, "{invalid}");
            let finding = &invalid.checks[0];
            assert_eq!(finding.rule, Rule::ServiceUnbounded);
            let path = (0..depth)
                .map(|i| format!("Level{i}"))
                .chain(["Worker".into()])
                .collect::<Vec<_>>()
                .join("/");
            assert_eq!(finding.nodes, [path]);
            assert!(finding.fix.contains("root"));
        }
    }
}

#[test]
fn bounded_root_or_explicit_service_stop_accepts_the_same_trees() {
    for depth in 1..=4 {
        for templates in 0..(1 << depth) {
            assert!(tree(depth, templates, false, bounded()).is_ok());
            assert!(tree(depth, templates, true, Shutdown::unbounded()).is_ok());
        }
    }
}

#[test]
fn reused_child_reports_each_registration_in_declaration_order() {
    let child = leaf(false);
    let mut p = Plan::builder("Root");
    p.component("First", &child);
    p.service("Local")
        .start(|_cx, ()| async { Ok(Serving::new((), std::future::pending())) });
    p.template("Second", &child);
    let invalid = p
        .build(Policy::FailFast, Shutdown::unbounded(), Mode::Resident)
        .unwrap_err();
    assert_eq!(
        invalid
            .checks
            .iter()
            .map(|f| f.nodes[0].as_str())
            .collect::<Vec<_>>(),
        ["First/Worker", "Local", "Second/Worker"]
    );
    assert!(invalid
        .checks
        .iter()
        .all(|f| f.rule == Rule::ServiceUnbounded));
}
