//! Independent checks over what a plan says about itself.
//!
//! These recompute the properties from the declaration rather than asking the
//! core to confirm its own answer: the edge set is checked against the nodes'
//! `needs`, and the release order is checked against a closure this module
//! computes itself. A checker that trusted the thing it checks would prove
//! nothing.
//!
//! [`check_plan`] checks the static structure (INV-1, INV-5, INV-6).
//! [`check_trace_prefix`] and [`check_trace`] check a run's trace against
//! the plan view and the report (INV-1…5, 7…12, 15, 18, 20, `orphans:
//! none`), from the view and the trace alone — never from the machine.
//! [`check_arbitration`] and [`check_scopes`] add INV-1's lock and pool
//! *exclusion* clauses, `Skipped`, `terminal` and inner-scope settling
//! (`MUTEX`, `POOL`, `SKIPPED`, `TERMINAL`, `T5-INNER`); [`check_whys`] adds
//! the contract's "`on` lists each reason" (`WHY`). Each of them was missing
//! when `dev-docs/Review-Stage1-Semantics.md` § 5.1 was written, and each is
//! run over every case of suite (d).
//!
//! **Instances.** Every per-node rule groups by [`occ::Occ`] — the copy of a
//! declaration, path plus instance chain — so a template's instances are
//! checked exactly as the static graph is and are never merged into it.
//! [`check_containment`] adds INV-16's two declaration clauses and the
//! instance lifecycle (`INSTANCE`, `T5-INSTANCE`); INV-16's run-time clause is
//! INV-5 applied per copy.
//!
//! **Honest limit.** INV-1 has a negative fixture (an edge no node declared).
//! INV-5 and INV-6 do not: the core derives the release order *from* the edge
//! set, so today the two computations cannot disagree unless the core's
//! derivation regresses — which is exactly what these two checks are for. They
//! are regression guards with positive coverage over every plan shape, not
//! independently falsified checks, and this crate does not claim otherwise.

mod arbitration;
mod containment;
pub mod occ;
mod report;
mod trace;

pub use arbitration::{check_arbitration, check_scopes, check_whys};
pub use containment::{check_containment, check_instance_releases};
pub use report::{check_report, check_report_with_slack};
pub use trace::{check_trace, check_trace_prefix, check_trace_with_slack};

use sdax::host::{Clock, Time};
use sdax::{NodePath, PlanView};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::time::Duration;

/// One thing that does not hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// Which invariant, by its id in the contract.
    pub rule: &'static str,
    /// What was found.
    pub detail: String,
}

pub(crate) fn violation(rule: &'static str, detail: String) -> Violation {
    Violation { rule, detail }
}

/// Check a plan's structure: the declared edges, the alive-during relation,
/// and which cleanups the view says may overlap.
pub fn check_plan(view: &PlanView) -> Vec<Violation> {
    let mut out = Vec::new();
    let paths: Vec<NodePath> = view.nodes.iter().map(|n| n.path.clone()).collect();
    let index = |p: &NodePath| paths.iter().position(|q| q == p);

    // INV-1: the edge set is exactly what the nodes declared.
    for e in &view.edges {
        match view.node(e.from.to_string().as_str()) {
            None => out.push(violation(
                "INV-1",
                format!("edge from unknown node {}", e.from),
            )),
            Some(n) if !n.needs.contains(&e.to) => out.push(violation(
                "INV-1",
                format!(
                    "edge {}→{} is not among {}'s declared needs",
                    e.from, e.to, e.from
                ),
            )),
            Some(_) => {}
        }
    }
    for n in &view.nodes {
        for need in &n.needs {
            if !view.edges.iter().any(|e| e.from == n.path && &e.to == need) {
                out.push(violation(
                    "INV-1",
                    format!("{} declares a need on {} with no edge", n.path, need),
                ));
            }
        }
    }

    // Recompute the ordering closure from the declared edges plus containment:
    // a component's or template's inner nodes are cleaned up as one unit.
    let n = paths.len();
    let mut after = vec![vec![false; n]; n];
    for e in &view.edges {
        if let (Some(i), Some(j)) = (index(&e.from), index(&e.to)) {
            after[i][j] = true;
        }
    }
    for (i, a) in paths.iter().enumerate() {
        for (j, b) in paths.iter().enumerate() {
            if i != j
                && a.segments().len() > b.segments().len()
                && a.segments().starts_with(b.segments())
            {
                after[i][j] = true;
            }
        }
    }
    for k in 0..n {
        for i in 0..n {
            if !after[i][k] {
                continue;
            }
            let row = after[k].clone();
            for (dst, src) in after[i].iter_mut().zip(row) {
                *dst |= src;
            }
        }
    }

    let order = view.release_order();

    // INV-5: a node's release starts only after every dependent's cleanup ended.
    for e in &view.edges {
        if !order.before(&e.from.to_string(), &e.to.to_string()) {
            out.push(violation(
                "INV-5",
                format!(
                    "{} needs {}, so {}'s cleanup must end first",
                    e.from, e.to, e.from
                ),
            ));
        }
    }

    // INV-6: the pairs the view calls unordered are exactly the pairs with no
    // path between them.
    for i in 0..n {
        for j in i + 1..n {
            let independent = !after[i][j] && !after[j][i];
            let said = order.unordered(&paths[i].to_string(), &paths[j].to_string());
            if independent != said {
                out.push(violation(
                    "INV-6",
                    format!(
                        "{} and {}: the view says {}, the declaration says {}",
                        paths[i],
                        paths[j],
                        if said { "unordered" } else { "ordered" },
                        if independent { "unordered" } else { "ordered" }
                    ),
                ));
            }
        }
    }
    out
}

/// The shared conformance suite for [`Clock`] (LBT-009): every implementation
/// must satisfy it, driven by whatever moves that clock forward.
///
/// Checks that time is read from the clock and nowhere else: `now` does not
/// move by itself, it moves by exactly what `advance` was asked for, and a
/// `sleep` completes only once the clock has passed its deadline.
pub fn check_clock(clock: &dyn Clock, advance: impl Fn(Duration)) -> Vec<Violation> {
    let mut out = Vec::new();
    let start = clock.now();
    if clock.now() != start {
        out.push(violation("CLOCK-1", "now() moved with no advance".into()));
    }

    let mut sleeping = clock.sleep(Duration::from_secs(5));
    if poll_once(sleeping.as_mut()).is_ready() {
        out.push(violation(
            "CLOCK-2",
            "a 5s sleep was ready before the clock moved".into(),
        ));
    }
    advance(Duration::from_secs(4));
    if poll_once(sleeping.as_mut()).is_ready() {
        out.push(violation(
            "CLOCK-2",
            "a 5s sleep was ready after only 4s".into(),
        ));
    }
    advance(Duration::from_secs(1));
    if poll_once(sleeping.as_mut()).is_pending() {
        out.push(violation(
            "CLOCK-2",
            "a 5s sleep was still pending after 5s".into(),
        ));
    }

    let expected = Time::from_nanos(start.as_nanos() + 5_000_000_000);
    if clock.now() != expected {
        out.push(violation(
            "CLOCK-3",
            format!(
                "after advancing 5s, now() is {} rather than {expected}",
                clock.now()
            ),
        ));
    }
    out
}

struct NoopWake;
impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

/// Poll a pinned future once with a no-op waker. The harness never sleeps, so
/// a test drives futures by hand.
pub fn poll_once<F: Future + ?Sized>(f: Pin<&mut F>) -> Poll<F::Output> {
    let waker = Waker::from(Arc::new(NoopWake));
    f.poll(&mut Context::from_waker(&waker))
}
