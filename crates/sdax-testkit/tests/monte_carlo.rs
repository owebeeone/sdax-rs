//! Suite (d) — the Monte Carlo walk
//! (`cargo test -p sdax-testkit --test monte_carlo`).
//!
//! Every case is one seed. The seed makes a plan (sometimes an invalid one on
//! purpose), a script and a schedule; the plan runs on the **same**
//! [`ScriptedDriver`] the conformance suite uses; and every trace is judged by
//! the independent [`invariants`](sdax_testkit::invariants) checker, never by
//! the machine's own state.
//!
//! **Seed discipline.** The fast loop uses a fixed seed so CI is
//! deterministic. `SDAX_MC_SEED=<u64>` replays a run seed; `SDAX_MC_CASES=<n>`
//! sets the count. On any failure the run seed, the case index, the case seed,
//! the plan, the script and the trace up to the failing step go to stderr, so
//! one line of output reproduces the case exactly.
//!
//! **Coverage floors.** A random walk that quietly stops reaching a corner is
//! a walk that passes without testing anything. Every corner in `CORNERS` has
//! a floor the default run must clear, so the generator regressing fails the
//! suite instead of going unnoticed.

use sdax::*;
use sdax_testkit::mc::{gen, prng, script};
use sdax_testkit::{Driven, ScriptedDriver};
use std::collections::BTreeMap;

/// The fast loop's run seed. Fixed and documented so CI is deterministic;
/// `SDAX_MC_SEED` overrides it, and `monte_carlo_big` derives one from the
/// clock and prints it.
const FIXED_SEED: u64 = 0x5DA0_2026_0906_0001;

/// How many cases the fast loop walks. Chosen to fit the workspace test
/// budget; `SDAX_MC_CASES` overrides it.
const DEFAULT_CASES: u64 = 3000;

/// One in this many cases is re-run from its seed and compared byte for byte
/// (INV-14). Sampled rather than universal so the fast loop stays inside its
/// budget; `monte_carlo_big` samples the same way over far more cases.
const DETERMINISM_EVERY: u64 = 16;

/// The corners the walk must reach, and how often at least. A floor that has
/// to be lowered is a finding, not a tidy-up.
const CORNERS: &[(&str, usize)] = &[
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
    ("pool wait", 5),
    ("retry after a held attempt", 5),
    ("second cancel during cleanup", 5),
    ("service restart", 5),
    ("two faults in one tick", 5),
];

/// What the walk reached, by corner and by outcome.
#[derive(Default)]
struct Coverage {
    hits: BTreeMap<&'static str, usize>,
}

impl Coverage {
    fn hit(&mut self, corner: &'static str) {
        *self.hits.entry(corner).or_insert(0) += 1;
    }

    fn get(&self, corner: &str) -> usize {
        self.hits.get(corner).copied().unwrap_or(0)
    }

    /// The histogram, one line per corner, floors named where there is one.
    fn render(&self, cases: u64) -> String {
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
    fn shortfalls(&self) -> Vec<String> {
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

fn is_start(k: &TraceKind) -> bool {
    matches!(k, TraceKind::Start(_))
}

fn is_cleanup_start(k: &TraceKind) -> bool {
    matches!(k, TraceKind::ReleaseStart | TraceKind::CompensateStart)
}

/// Count the corners this one run reached. Everything is read from the trace,
/// the view and the effects the machine returned — never from its state.
fn corners(d: &Driven, view: &PlanView, cov: &mut Coverage) {
    let effects = d.effects();
    for n in &view.nodes {
        let evs = node_events(d, &n.path);

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
            let key = d.key(&n.path.to_string());
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

        // A fault raised inside a component.
        if n.path.to_string().contains('/') && evs.iter().any(|k| matches!(k, TraceKind::Fail(..)))
        {
            cov.hit("component fault");
        }
    }

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

    if d.waited(|r| matches!(r, Reason::Exclusive)) > 0 {
        cov.hit("exclusive contention");
    }
    if d.waited(|r| matches!(r, Reason::Pool)) > 0 {
        cov.hit("pool wait");
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

/// Everything needed to reproduce one case, printed on failure.
struct CaseFailure {
    run_seed: u64,
    index: u64,
    case_seed: u64,
    what: String,
    plan: String,
    script: String,
    trace: String,
}

impl CaseFailure {
    fn render(&self) -> String {
        format!(
            "\n=== monte carlo case failed ===\n\
             SDAX_MC_SEED={} (run seed 0x{:016X})\n\
             case index {}, case seed {} (0x{:016X})\n\
             replay: SDAX_MC_SEED={} SDAX_MC_CASES={} cargo test -p sdax-testkit --test monte_carlo\n\
             \n{}\n\nplan:\n{}\nscript:\n{}\n\n{}",
            self.run_seed,
            self.run_seed,
            self.index,
            self.case_seed,
            self.case_seed,
            self.run_seed,
            self.index + 1,
            self.what,
            self.plan,
            self.script,
            self.trace,
        )
    }
}

/// The trace and the fed steps up to (and including) the failing step.
fn upto_failure<Out>(d: &Driven<Out>) -> String {
    let (steps, events) = d
        .first_violation
        .unwrap_or((d.steps.len(), d.trace.events.len()));
    let cut = Trace {
        events: d.trace.events[..events.min(d.trace.events.len())].to_vec(),
    };
    let mut s = format!("trace (to the failing step, {events} events):\n");
    s.push_str(&sdax_testkit::eol::Eol(&cut).render());
    s.push_str(&format!("steps (to the failing step, {steps} fed):\n"));
    for st in d.steps.iter().take(steps) {
        s.push_str(&format!("  t={} {} -> {:?}\n", st.at, st.event, st.effects));
    }
    s
}

/// One case: generate, build (or be refused), run, check, count.
fn run_case(run_seed: u64, index: u64, cov: &mut Coverage) -> Result<(), Box<CaseFailure>> {
    run_seeded(run_seed, index, prng::case_seed(run_seed, index), cov)
}

/// One case from its own seed, however that seed was arrived at.
fn run_seeded(
    run_seed: u64,
    index: u64,
    case_seed: u64,
    cov: &mut Coverage,
) -> Result<(), Box<CaseFailure>> {
    let mut g = prng::SplitMix64::new(case_seed);
    let generated = gen::generate(&mut g);
    let declaration = generated.lines.join("\n");
    let fail = |what: String, plan: String, script: String, trace: String| {
        Box::new(CaseFailure {
            run_seed,
            index,
            case_seed,
            what,
            plan,
            script,
            trace,
        })
    };

    let plan = match (generated.plan, generated.expect_invalid) {
        // Meant to be invalid, and refused by the rule the mutation names.
        (Err(invalid), Some(rule)) => {
            if invalid.checks.iter().any(|c| c.rule == rule) {
                cov.hit("invalid plan refused");
                return Ok(());
            }
            return Err(fail(
                format!(
                    "the mutation {} was refused, but not by {rule:?}:\n{invalid}",
                    generated.mutation
                ),
                declaration,
                "(not built)".into(),
                String::new(),
            ));
        }
        (Ok(plan), Some(rule)) => {
            return Err(fail(
                format!(
                    "the mutation {} should have been refused by {rule:?}, and built instead",
                    generated.mutation
                ),
                format!("{declaration}\n\n{}", plan.inspect()),
                "(not run)".into(),
                String::new(),
            ));
        }
        (Err(invalid), None) => {
            return Err(fail(
                format!("a plan with no mutation was refused:\n{invalid}"),
                declaration,
                "(not built)".into(),
                String::new(),
            ));
        }
        (Ok(plan), None) => plan,
    };

    let view = plan.inspect();
    let bounded = view.shutdown.budget().is_some();
    let script = script::generate(&mut g, &view, bounded);
    let rendered_plan = format!("{declaration}\n\n{view}");

    let d = match ScriptedDriver::run(&plan, &script) {
        Ok(d) => d,
        Err(e) => {
            return Err(fail(
                format!("the script would not run: {e:?}"),
                rendered_plan,
                format!("{script:?}"),
                String::new(),
            ))
        }
    };

    if !d.violations.is_empty() || !d.rejections.is_empty() || d.stuck {
        let mut what = String::new();
        if d.stuck {
            what.push_str("stuck: the run stopped short of End with nothing left to deliver\n");
        }
        for v in &d.violations {
            what.push_str(&format!("{}: {}\n", v.rule, v.detail));
        }
        for r in &d.rejections {
            what.push_str(&format!("rejected: {r}\n"));
        }
        return Err(fail(
            what,
            rendered_plan,
            format!("{script:?}"),
            upto_failure(&d),
        ));
    }

    corners(&d, &view, cov);

    // INV-14: the same seed is the same run, byte for byte.
    if index % DETERMINISM_EVERY == 0 {
        let mut g2 = prng::SplitMix64::new(case_seed);
        let again = gen::generate(&mut g2);
        let plan2 = match again.plan {
            Ok(p) => p,
            Err(invalid) => {
                let what = format!(
                    "the same seed built a plan once and was refused the next time:\n{invalid}"
                );
                return Err(fail(
                    what,
                    rendered_plan,
                    format!("{script:?}"),
                    String::new(),
                ));
            }
        };
        let view2 = plan2.inspect();
        let script2 = script::generate(&mut g2, &view2, view2.shutdown.budget().is_some());
        let d2 = ScriptedDriver::run(&plan2, &script2).expect("the same script runs again");
        let (a, b) = (d.eol().render(), d2.eol().render());
        if a != b {
            let what = format!(
                "INV-14: the same seed produced two different traces\n\
                 --- first ---\n{a}--- second ---\n{b}"
            );
            return Err(fail(
                what,
                rendered_plan,
                format!("{script:?}"),
                String::new(),
            ));
        }
    }

    Ok(())
}

/// Walk `cases` cases from `run_seed`, stopping at the first failure.
fn walk(run_seed: u64, cases: u64) -> (Coverage, Option<Box<CaseFailure>>) {
    let mut cov = Coverage::default();
    for index in 0..cases {
        if let Err(f) = run_case(run_seed, index, &mut cov) {
            return (cov, Some(f));
        }
    }
    (cov, None)
}

fn env_u64(name: &str) -> Option<u64> {
    std::env::var(name)
        .ok()
        .map(|v| v.parse().unwrap_or_else(|_| panic!("{name} must be a u64")))
}

/// The fast loop: a fixed seed, a fixed count, every floor asserted. Run it
/// with `--nocapture` to see the histogram.
#[test]
fn monte_carlo() {
    let run_seed = env_u64("SDAX_MC_SEED").unwrap_or(FIXED_SEED);
    let cases = env_u64("SDAX_MC_CASES").unwrap_or(DEFAULT_CASES);
    let (cov, failure) = walk(run_seed, cases);
    if let Some(f) = failure {
        eprintln!("{}", f.render());
        panic!("monte carlo case {} failed (see stderr)", f.index);
    }
    print!("{}", cov.render(cases));
    let short = cov.shortfalls();
    assert!(
        short.is_empty(),
        "the walk stopped reaching {} corner(s) — the generator regressed, \
         the floor is not the problem:\n  {}\n{}",
        short.len(),
        short.join("\n  "),
        cov.render(cases),
    );
}

/// The long walk: a seed from the clock, printed first so a failure replays.
/// Ignored by default; `cargo test -p sdax-testkit --test monte_carlo --
/// --ignored --nocapture` runs it.
#[test]
#[ignore = "long: 50000 cases, minutes not seconds"]
fn monte_carlo_big() {
    let run_seed = env_u64("SDAX_MC_SEED").unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("after the epoch")
            .as_nanos() as u64
            | 1
    });
    let cases = env_u64("SDAX_MC_CASES").unwrap_or(50_000);
    eprintln!("monte_carlo_big: SDAX_MC_SEED={run_seed} SDAX_MC_CASES={cases}");
    let (cov, failure) = walk(run_seed, cases);
    if let Some(f) = failure {
        eprintln!("{}", f.render());
        panic!("monte carlo case {} failed (see stderr)", f.index);
    }
    print!("{}", cov.render(cases));
    let short = cov.shortfalls();
    assert!(short.is_empty(), "{}", short.join("\n  "));
}

/// The walk itself must be reproducible: the same run seed visits the same
/// cases and reaches the same corners.
#[test]
fn a_run_seed_reproduces_the_whole_walk() {
    let (a, fa) = walk(FIXED_SEED, 64);
    let (b, fb) = walk(FIXED_SEED, 64);
    assert!(
        fa.is_none() && fb.is_none(),
        "64 cases from the fixed seed pass"
    );
    assert_eq!(a.hits, b.hits, "the same run seed is the same walk");
}

/// Every case seed that found a bug, and what it found. Replayed on every
/// run, so the fixes stay fixed.
///
/// **Honest limit.** The generator itself was corrected several times during
/// the walk (a pool that made itself unused, a mutation that lied), so a seed
/// captured before one of those corrections now builds a plan of the same
/// family rather than the identical one. The shapes are pinned by name in
/// `tests/conformance/regressions.rs`; this list pins the seeds.
const SEEDS_THAT_FOUND_BUGS: &[(u64, &str)] = &[
    (
        6299039668138085550,
        "sim: a hold later than the body's own ending",
    ),
    (
        15604115191173039358,
        "machine: abandon dropped the parked faults",
    ),
    (
        3133216432680936309,
        "machine: an interrupted component had no terminal observation",
    ),
    (
        15845907256369010671,
        "machine: RecordOrder.steps counted import nodes",
    ),
    (
        2052455747394511223,
        "machine: settle's Pending/Waiting arm dropped the parked faults",
    ),
    (
        16408307695250133523,
        "machine: a serve fault was dropped when a restart followed",
    ),
    (
        4814721321710810844,
        "checker: a persistent effect has no compensation to wait for",
    ),
    (
        6364352641463191117,
        "machine: abandon_all opened an inner release before a dependent ended",
    ),
    (
        4529049039084370172,
        "machine: a failed component kept admitting",
    ),
    (
        13115684965286655053,
        "checker: INV-9's escape did not know Ambiguous",
    ),
    (
        1784653771357370795,
        "gen: a resident plan that restarts forever never ends",
    ),
    (
        14688502050082075365,
        "machine: a skipped component's inner scope held the release gate shut",
    ),
    (
        666241015029950078,
        "machine: a ready component's inner scope was never settled",
    ),
    (
        13281131733918634243,
        "machine: skip_dependents dropped the parked faults",
    ),
    (
        3320581108311238895,
        "machine: an inner scope's own budget ended it with no ReleaseStart",
    ),
    (
        6593995201653377501,
        "gen: PoolStarve starved the wrong pool",
    ),
    (
        17502963814378720686,
        "machine: a queued waiter started after a need stopped being Ready",
    ),
    (
        2796147909731581100,
        "machine: abandon_inner opened an inner release before a dependent component ended",
    ),
    (
        2682709018262434330,
        "machine: a component whose inner scope settled by itself never ended its attempt",
    ),
    (
        2300709931059428012,
        "machine: admit started a waiter a nested admit had already started",
    ),
    (
        11779147375297488456,
        "sim: a timed-out blocking attempt's late outcome ended the next attempt",
    ),
];

#[test]
fn every_seed_that_found_a_bug_replays_clean() {
    let mut cov = Coverage::default();
    for (seed, what) in SEEDS_THAT_FOUND_BUGS {
        if let Err(f) = run_seeded(*seed, 0, *seed, &mut cov) {
            eprintln!("{}", f.render());
            panic!("the seed that found {what:?} fails again (see stderr)");
        }
    }
}
