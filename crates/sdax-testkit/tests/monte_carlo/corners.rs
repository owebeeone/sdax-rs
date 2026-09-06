//! The corners the Monte Carlo walk must reach, and the counting that proves
//! it did.
//!
//! A random walk that quietly stops reaching a corner is a walk that passes
//! without testing anything, so every corner here has a floor the default run
//! must clear. Everything is counted from the declaration, the trace and the
//! effects the machine returned — never from the machine's own state, except
//! the three reachability counters that say so.

use sdax::*;
use sdax_testkit::Driven;
use std::collections::BTreeMap;

/// The corners the walk must reach, and how often at least. A floor that has
/// to be lowered is a finding, not a tidy-up.
pub(crate) const CORNERS: &[(&str, usize)] = &[
    ("a lock on an imported resource", 10),
    ("a service holds a pool", 50),
    ("abandon during cleanup", 5),
    ("ambiguity Compensate on an interrupted effect", 1),
    ("ambiguity Report on an interrupted effect", 1),
    ("ambiguity Retry on an interrupted effect", 1),
    ("budget expiry abandons a release", 1),
    ("cancel during backoff", 5),
    ("component fault", 5),
    ("cooperative grace completes", 5),
    ("cooperative grace expires", 5),
    ("exclusive contention", 5),
    ("invalid plan refused", 20),
    ("isolate skip propagation", 5),
    ("nested component", 5),
    ("pool wait", 5),
    ("queued behind an earlier waiter", 2),
    ("retry after a held attempt", 5),
    ("second cancel during cleanup", 5),
    ("service restart", 5),
    ("two faults in one tick", 5),
    ("two locks on one node", 10),
    // Stage 3: templates and dynamic instances.
    ("a template is declared", 100),
    ("a template importing a parent key", 50),
    ("a template that is never instantiated", 20),
    ("an instance is spawned", 100),
    ("two or more instances of one template", 20),
    ("an instance is closed while the run keeps going", 10),
    ("an instance faults", 20),
    ("an instance is cancelled with the run", 20),
    ("an instance outlives a cancel", 5),
    ("an instance is abandoned at the budget", 2),
    ("an instance's readiness is awaited", 10),
    ("a spawn refused: the scope is stopping", 5),
    ("a spawn refused: a foreign template", 5),
    ("a nested template", 2),
    // A root plan's own per-run input: the node dropped from the run, and the
    // node that needs it starting anyway.
    ("a root plan takes a per-run input", 300),
    ("the root input's reader ran", 200),
];

/// What the walk reached, by corner and by outcome.
#[derive(Default)]
pub(crate) struct Coverage {
    pub(crate) hits: BTreeMap<&'static str, usize>,
}

impl Coverage {
    pub(crate) fn hit(&mut self, corner: &'static str) {
        *self.hits.entry(corner).or_insert(0) += 1;
    }

    pub(crate) fn get(&self, corner: &str) -> usize {
        self.hits.get(corner).copied().unwrap_or(0)
    }

    /// The histogram, one line per corner, floors named where there is one.
    pub(crate) fn render(&self, cases: u64) -> String {
        let mut s = format!("coverage over {cases} cases:\n");
        for (name, count) in &self.hits {
            let floor = CORNERS
                .iter()
                .find(|(c, _)| c == name)
                .map(|(_, f)| format!("  (floor {f})"))
                .unwrap_or_default();
            s.push_str(&format!("  {count:>6}  {name}{floor}\n"));
        }
        for (name, floor) in CORNERS {
            if !self.hits.contains_key(name) {
                s.push_str(&format!("       0  {name}  (floor {floor}) NOT REACHED\n"));
            }
        }
        s
    }

    /// Every floor that was not cleared.
    pub(crate) fn shortfalls(&self) -> Vec<String> {
        CORNERS
            .iter()
            .filter(|(name, floor)| self.get(name) < *floor)
            .map(|(name, floor)| format!("{name}: {} < {floor}", self.get(name)))
            .collect()
    }
}

fn node_events<'a>(d: &'a Driven, path: &NodePath) -> Vec<&'a TraceKind> {
    d.trace
        .events
        .iter()
        .filter(|e| e.node.as_ref() == Some(path))
        .map(|e| &e.kind)
        .collect()
}

/// The scope a path lives in: everything but its last segment.
fn scope_of(p: &NodePath) -> String {
    let segs = p.segments();
    segs[..segs.len().saturating_sub(1)].join("/")
}

fn scope_of_str(p: &str) -> String {
    match p.rfind('/') {
        Some(i) => p[..i].to_string(),
        None => String::new(),
    }
}

fn is_start(k: &TraceKind) -> bool {
    matches!(k, TraceKind::Start(_))
}

fn is_cleanup_start(k: &TraceKind) -> bool {
    matches!(k, TraceKind::ReleaseStart | TraceKind::CompensateStart)
}

/// Count the corners this one run reached. Everything is read from the trace,
/// the view and the effects the machine returned — never from its state.
pub(crate) fn corners(d: &Driven, view: &PlanView, cov: &mut Coverage) {
    let effects = d.effects();
    for n in &view.nodes {
        let evs = node_events(d, &n.path);

        // The plan-shape corners the generator could not reach before the
        // Stage 1 review (`Review-Stage1-Semantics.md` § 5.2). Each is counted
        // from the view, so a generator that stops producing one fails the
        // floor instead of quietly narrowing the walk.
        let scope = scope_of(&n.path);
        let locks: Vec<String> = ["exclusive", "shared"]
            .iter()
            .filter_map(|a| n.attr(a))
            .flat_map(|v| v.split(", ").map(|x| x.to_string()).collect::<Vec<_>>())
            .collect();
        if locks.len() >= 2 {
            cov.hit("two locks on one node");
        }
        if locks.iter().any(|l| scope_of_str(l) != scope) {
            cov.hit("a lock on an imported resource");
        }
        if n.kind == Kind::Component && n.path.segments().len() >= 2 {
            cov.hit("nested component");
        }
        if n.kind == Kind::Service && n.attr("limit").is_some() {
            cov.hit("a service holds a pool");
        }

        // A retry after an attempt that had already registered a value.
        if let Some(h) = evs.iter().position(|k| matches!(k, TraceKind::Held)) {
            if evs[h + 1..].iter().any(|k| is_start(k)) {
                cov.hit("retry after a held attempt");
            }
        }

        // An interrupt with no body in flight: the attempt had already failed
        // and the node was sitting in backoff waiting for the next one.
        for (i, k) in evs.iter().enumerate() {
            if matches!(k, TraceKind::Interrupted { .. })
                && evs[..i]
                    .iter()
                    .rposition(|p| is_start(p) || matches!(p, TraceKind::Fail(..)))
                    .is_some_and(|p| matches!(evs[p], TraceKind::Fail(..)))
            {
                cov.hit("cancel during backoff");
                break;
            }
        }

        // The budget cut off an obligation that had already started.
        if let Some(a) = evs.iter().position(|k| matches!(k, TraceKind::Abandoned)) {
            if evs[..a].iter().any(|k| is_cleanup_start(k)) {
                cov.hit("abandon during cleanup");
            }
            let released = evs[..a]
                .iter()
                .rposition(|k| matches!(k, TraceKind::ReleaseStart));
            if released.is_some_and(|r| {
                !evs[r..]
                    .iter()
                    .any(|k| matches!(k, TraceKind::ReleaseOk | TraceKind::ReleaseFail(_)))
            }) {
                cov.hit("budget expiry abandons a release");
            }
        }

        // A service that came up, went down and came up again.
        if n.kind == Kind::Service {
            if let Some(r) = evs.iter().position(|k| matches!(k, TraceKind::Ready)) {
                if evs[r + 1..].iter().any(|k| is_start(k)) {
                    cov.hit("service restart");
                }
            }
        }

        // An effect interrupted between starting and holding, by policy.
        if evs.iter().any(|k| matches!(k, TraceKind::Ambiguous)) {
            match n.attr("ambiguous") {
                Some("report") => cov.hit("ambiguity Report on an interrupted effect"),
                Some("compensate") => cov.hit("ambiguity Compensate on an interrupted effect"),
                Some("retry") => cov.hit("ambiguity Retry on an interrupted effect"),
                _ => {}
            }
        }

        // A cooperative cancel: `Signal` first, then `Abort` only if the grace
        // ran out before the body answered.
        if n.attr("cancel")
            .is_some_and(|c| c.starts_with("cooperative"))
        {
            // A template's inner node has one run key per live instance, and
            // none at all if the run never instantiated it.
            for key in d.keys_of(&n.path.to_string()) {
                let signal = format!("Signal({key:?})");
                let abort = format!("Abort({key:?})");
                if let Some(s) = effects.iter().position(|e| *e == signal) {
                    if effects[s..].contains(&abort) {
                        cov.hit("cooperative grace expires");
                    } else {
                        cov.hit("cooperative grace completes");
                    }
                }
            }
        }

        // A fault raised inside a component.
        if n.path.to_string().contains('/') && evs.iter().any(|k| matches!(k, TraceKind::Fail(..)))
        {
            cov.hit("component fault");
        }
    }

    instance_corners(d, view, cov);

    // Two independent faults landing on the same tick.
    let mut fault_ticks: BTreeMap<u64, usize> = BTreeMap::new();
    for e in &d.trace.events {
        if matches!(e.kind, TraceKind::Fail(..)) {
            *fault_ticks.entry(e.at.as_nanos()).or_insert(0) += 1;
        }
    }
    if fault_ticks.values().any(|c| *c >= 2) {
        cov.hit("two faults in one tick");
    }

    if view.policy == Policy::Isolate
        && d.trace
            .events
            .iter()
            .any(|e| matches!(e.kind, TraceKind::Skipped { .. }))
    {
        cov.hit("isolate skip propagation");
    }

    if d.trace
        .events
        .iter()
        .any(|e| matches!(e.kind, TraceKind::RequestDuringCleanup))
    {
        cov.hit("second cancel during cleanup");
    }

    // These three read the machine's own `why`, so they are *reachability*
    // counters — they say the walk got a run into that kind of contention, not
    // that the arbitration was right. Correctness is the checker's: `MUTEX`
    // and `POOL` recompute INV-1's exclusion clauses from the view and the
    // trace, and `WHY` fails any empty `Waiting{on}`.
    if d.waited(|r| matches!(r, Reason::Exclusive)) > 0 {
        cov.hit("exclusive contention");
    }
    if d.waited(|r| matches!(r, Reason::Pool)) > 0 {
        cov.hit("pool wait");
    }
    if d.waited(|r| matches!(r, Reason::QueuedBehind)) > 0 {
        cov.hit("queued behind an earlier waiter");
    }

    // Informational tallies: no floor, but a shift in them is visible.
    cov.hit(match d.report.outcome {
        Outcome::Ok => "= outcome Ok",
        Outcome::Failed => "= outcome Failed",
        Outcome::Cancelled => "= outcome Cancelled",
    });
    if d.stuck {
        cov.hit("= the run hung (no deadline, no request)");
    }
}

/// The corners templates and instances add. Everything is read from the
/// declaration, the trace and the `SpawnError`s the bodies were handed —
/// never from the machine's state.
pub(crate) fn instance_corners(d: &Driven, view: &PlanView, cov: &mut Coverage) {
    let settling = d
        .trace
        .events
        .iter()
        .position(|e| matches!(e.kind, TraceKind::Settling));
    let cancelled = d.report.outcome == Outcome::Cancelled;
    for n in view.nodes.iter().filter(|n| n.kind == Kind::Template) {
        cov.hit("a template is declared");
        if !n.needs.is_empty() {
            cov.hit("a template importing a parent key");
        }
        if n.path.segments().len() > 1 {
            cov.hit("a nested template");
        }
        let mut spawned = 0usize;
        for (i, e) in d.trace.events.iter().enumerate() {
            if e.node.as_ref() != Some(&n.path) {
                continue;
            }
            match e.kind {
                TraceKind::InstanceSpawned(_) => {
                    spawned += 1;
                    cov.hit("an instance is spawned");
                }
                TraceKind::InstanceEnded(_, o) => {
                    if o == Outcome::Failed {
                        cov.hit("an instance faults");
                    }
                    if o == Outcome::Cancelled {
                        cov.hit("an instance is cancelled with the run");
                    }
                    match settling {
                        Some(s) if i < s => {
                            cov.hit("an instance is closed while the run keeps going")
                        }
                        Some(s) if cancelled && i > s => cov.hit("an instance outlives a cancel"),
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        if spawned == 0 {
            cov.hit("a template that is never instantiated");
        }
        if spawned >= 2 {
            cov.hit("two or more instances of one template");
        }
    }
    // An instance's own node abandoned when the budget ran out.
    for e in &d.trace.events {
        let (Some(p), Some(o)) = (&e.node, &e.order) else {
            continue;
        };
        if matches!(e.kind, TraceKind::Abandoned) && o.steps.iter().any(|(_, i)| i.is_some()) {
            let _ = p;
            cov.hit("an instance is abandoned at the budget");
        }
    }
    for s in &d.spawns {
        match s.result {
            Err(SpawnError::ScopeStopping) => cov.hit("a spawn refused: the scope is stopping"),
            Err(SpawnError::ForeignTemplate) => cov.hit("a spawn refused: a foreign template"),
            _ => {}
        }
    }
    // A start body that awaited `Child::ready()` before returning (INV-17):
    // the instance reached steady state before the spawner became `Ready`.
    for n in view.nodes.iter().filter(|n| !n.spawns.is_empty()) {
        let ready = d.eol().ready(&n.path.to_string());
        let inner_ready = d.trace.events.iter().any(|e| {
            matches!(e.kind, TraceKind::Ready)
                && e.order
                    .as_ref()
                    .is_some_and(|o| o.steps.iter().any(|(_, i)| i.is_some()))
        });
        if ready.is_some() && inner_ready {
            cov.hit("an instance's readiness is awaited");
        }
    }
}
