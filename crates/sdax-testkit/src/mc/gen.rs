//! Random plans through the author surface: every kind, random `needs`
//! among earlier keys, random locks, pools, attributes, policies, budgets,
//! modes and components — and, sometimes, a declaration that is invalid on
//! purpose, so the validator is under test too.

use super::keys::{add_node, secs, Attrs, Keys, Needs, Unit};
use super::mutate::Mutation;
use super::prng::SplitMix64;
use sdax::*;
use std::time::Duration;

/// A generated declaration.
pub struct Generated {
    /// The plan, or what the validator said.
    pub plan: Result<Plan, Invalid>,
    /// The rule an intentionally invalid plan must be refused by.
    pub expect_invalid: Option<Rule>,
    /// The mutation applied, for the failure print.
    pub mutation: &'static str,
    /// One line per node, for the failure print.
    pub lines: Vec<String>,
}
/// What the generator decided for one scope before building it.
pub(crate) struct Shape {
    pub(crate) policy: Policy,
    pub(crate) shutdown: Shutdown,
    pub(crate) budget: Option<Duration>,
    /// Whether the **root**'s shutdown budget is bounded. Until a nested
    /// scope's release graph opens it is bounded by the root's deadline alone
    /// (T7a), so a service with no `stop_within` anywhere inside an unbounded
    /// run has nothing to end its stop — which is what `V-SERVICE-UNBOUNDED`
    /// refuses for the root's own services and does not reach for a child's.
    pub(crate) bounded_root: bool,
    /// How many template plans enclose this scope. A template's own plan may
    /// declare one — nested instances are a real shape — but the walk stops
    /// at two so a case stays small enough to print.
    pub(crate) depth: usize,
}
fn pick_kind(g: &mut SplitMix64, allow_component: bool) -> Kind {
    let r = g.below(100);
    match r {
        0..=29 => Kind::Resource,
        30..=44 => Kind::Step,
        45..=52 => Kind::TryStep,
        53..=60 => Kind::BlockingStep,
        61..=75 => Kind::Service,
        76..=90 => Kind::Effect,
        91..=94 => Kind::Join,
        _ if allow_component => Kind::Component,
        _ => Kind::Resource,
    }
}
fn pick_needs(g: &mut SplitMix64, keys: &Keys) -> Needs {
    // `add_node` wires at most two unit needs; more would be silently dropped.
    let max_units = 2;
    let units: Vec<Key<Unit>> = g
        .subset(keys.units.len(), max_units)
        .into_iter()
        .map(|i| keys.units[i].0)
        .collect();
    let try_ = if !keys.trys.is_empty() && g.chance(0.4) {
        Some(keys.trys[g.below(keys.trys.len() as u64) as usize].0)
    } else {
        None
    };
    let join = if !keys.joins.is_empty() && g.chance(0.3) {
        Some(keys.joins[g.below(keys.joins.len() as u64) as usize])
    } else {
        None
    };
    Needs { units, try_, join }
}
fn pick_attrs(
    g: &mut SplitMix64,
    kind: Kind,
    needs: &Needs,
    keys: &Keys,
    shape: &Shape,
    pool: &mut dyn FnMut(&mut SplitMix64, bool) -> Pool,
) -> Attrs {
    let mut a = Attrs::default();
    if g.chance(0.2) {
        a.within = Some(secs(g.range(1, 4)));
    }
    if matches!(
        kind,
        Kind::Resource | Kind::Step | Kind::Effect | Kind::BlockingStep
    ) && g.chance(0.25)
    {
        let backoff = match g.below(3) {
            0 => None,
            1 => Some(Backoff::fixed(secs(g.range(0, 2)))),
            _ => Some(Backoff::exponential(secs(1), 2, secs(4))),
        };
        let mut r = Retry::attempts(g.range(2, 3) as u32);
        if let Some(b) = backoff {
            r = r.backoff(b);
        }
        a.retry = Some(r);
    }
    let resources: Vec<Key<Unit>> = needs
        .units
        .iter()
        .copied()
        .filter(|k| keys.units.iter().any(|(u, res)| u == k && *res))
        .collect();
    if !resources.is_empty() && kind != Kind::Join && g.chance(0.3) {
        let k = *g.pick(&resources);
        if g.chance(0.6) {
            a.exclusive.push(k);
        } else {
            a.shared.push(k);
        }
        // T1 takes several grants all or none; one lock per node never asked.
        // `V-DUP-ATTR` refuses one resource locked twice or in two modes, so
        // the second lock has to be a different resource.
        let rest: Vec<Key<Unit>> = resources.iter().copied().filter(|r| *r != k).collect();
        if !rest.is_empty() && g.chance(0.35) {
            let k2 = *g.pick(&rest);
            if g.chance(0.5) {
                a.exclusive.push(k2);
            } else {
                a.shared.push(k2);
            }
        }
    }
    if matches!(kind, Kind::Step | Kind::Resource | Kind::Effect) && g.chance(0.15) {
        a.limit = Some(pool(g, false));
    }
    if kind == Kind::BlockingStep {
        a.pool = Some(pool(g, false));
    }
    // A *resident* pool holder in a plan `V-POOL-STARVE` accepts: the pool has
    // room to spare and no second service. Nothing else in the walk reaches a
    // pool grant that is held for the run's whole life.
    if kind == Kind::Service && g.chance(0.2) {
        a.limit = Some(pool(g, true));
    }
    if matches!(
        kind,
        Kind::Resource | Kind::Step | Kind::Effect | Kind::TryStep
    ) && g.chance(0.2)
    {
        a.cooperative = Some(secs(g.range(0, 2)));
    }
    if kind == Kind::Service {
        let stop = shape
            .budget
            .map(|b| secs(g.range(0, b.as_secs().max(1)).min(b.as_secs())));
        a.stop_within = if shape.budget.is_none() || !shape.bounded_root || g.chance(0.6) {
            stop.or(Some(secs(g.range(1, 3))))
        } else {
            None
        };
        if g.chance(0.3) {
            let mut r = Restart::on_error(Backoff::fixed(secs(g.range(0, 2))));
            if g.chance(0.5) {
                r = r.max(g.range(1, 2) as u32);
            }
            a.restart = Some(r);
            a.idempotent = true;
        }
        a.terminal = g.chance(0.1);
    }
    if kind == Kind::Effect {
        a.ambiguity = *g.pick(&[Ambiguity::Report, Ambiguity::Compensate, Ambiguity::Retry]);
        a.persistent = g.chance(0.25);
        // `V-PERSIST-AMBIG`: a persistent effect has nothing to compensate.
        if a.persistent && a.ambiguity == Ambiguity::Compensate {
            a.ambiguity = *g.pick(&[Ambiguity::Report, Ambiguity::Retry]);
        }
        if a.ambiguity != Ambiguity::Report || a.retry.is_some() {
            a.idempotent = true;
        }
    }
    if kind == Kind::Resource {
        a.by_drop = g.chance(0.2);
    }
    a
}
/// One scope's worth of nodes into `p`. Returns the unit keys declared, and
/// whether a service was declared.
#[allow(clippy::too_many_arguments)]
pub(crate) fn fill<In>(
    g: &mut SplitMix64,
    p: &mut PlanBuilder<(), In>,
    prefix: &str,
    count: usize,
    shape: &Shape,
    keys: &mut Keys,
    lines: &mut Vec<String>,
    mutation: &mut Mutation,
    allow_component: bool,
    parent_units: &[(Key<Unit>, bool)],
) -> bool {
    let mut has_service = false;
    let mut has_template = false;
    let mut pools: Vec<Pool> = Vec::new();
    let mut mutated = false;
    // The names declared *in this scope*: `Mutation::DupName` must reuse one of
    // them. A child plan's node names live in the child's namespace, so
    // repeating one here is not a duplicate and the plan builds.
    let mut names: Vec<String> = Vec::new();
    let mut starved: Option<Pool> = None;
    for i in 0..count {
        let mut kind = pick_kind(g, allow_component);
        if *mutation == Mutation::Mode && !mutated && i + 1 == count {
            kind = Kind::Service;
        }
        let name = format!(
            "{prefix}{}{i}",
            kind.label()
                .chars()
                .next()
                .unwrap_or('n')
                .to_ascii_uppercase()
        );
        if kind == Kind::Component {
            let child = child_plan(
                g,
                &name,
                parent_units
                    .iter()
                    .chain(keys.units.iter())
                    .copied()
                    .collect::<Vec<_>>()
                    .as_slice(),
                shape,
                lines,
                allow_component,
            );
            match child {
                Ok(plan) => {
                    let k = p.component(&name, &plan);
                    keys.units.push((k, false));
                    lines.push(format!(
                        "{name}: component of {} nodes",
                        plan.inspect().nodes.len()
                    ));
                }
                Err(_) => lines.push(format!("{name}: component skipped (child invalid)")),
            }
            names.push(name);
            continue;
        }
        let needs = pick_needs(g, keys);
        // A try-step this node consumes is consumed: `Mutation::TryUnconsumed`
        // must find a genuinely unconsumed one, or the plan builds and the
        // mutation is a lie.
        if let Some(t) = needs.try_ {
            for (k, used) in keys.trys.iter_mut() {
                if *k == t {
                    *used = true;
                }
            }
        }
        let mut pool_of = |g: &mut SplitMix64, resident: bool| -> Pool {
            // A pool made here is used here: a pool with no user is
            // `V-UNUSED-POOL`, and that mistake belongs to
            // `Mutation::UnusedPool`, not to every case that widened the list.
            //
            // A resident holder always gets a fresh pool with room to spare, so
            // two services never share one and `V-POOL-STARVE` (which refuses a
            // pool its resident holders can fill) has nothing to say.
            if resident {
                let pool = p.pool(&format!("{prefix}rpool{}", pools.len()), 2);
                pools.push(pool);
                return pool;
            }
            if pools.is_empty() || g.chance(0.3) {
                let pool = p.pool(
                    &format!("{prefix}pool{}", pools.len()),
                    g.range(1, 2) as usize,
                );
                pools.push(pool);
                pool
            } else {
                *g.pick(&pools)
            }
        };
        let mut a = pick_attrs(g, kind, &needs, keys, shape, &mut pool_of);
        let mut name = name;
        if !mutated {
            match *mutation {
                Mutation::DupName if !names.is_empty() => {
                    name = names[0].clone();
                    mutated = true;
                }
                Mutation::LockNeeds if kind != Kind::Join => {
                    let outside: Vec<Key<Unit>> = keys
                        .units
                        .iter()
                        .filter(|(k, res)| *res && !needs.units.contains(k))
                        .map(|(k, _)| *k)
                        .collect();
                    if let Some(k) = outside.first() {
                        a.exclusive = vec![*k];
                        a.shared.clear();
                        mutated = true;
                    }
                }
                Mutation::IdempotentRequired if kind == Kind::Effect => {
                    a.retry = Some(Retry::attempts(2));
                    a.idempotent = false;
                    mutated = true;
                }
                Mutation::BudgetOrder if kind == Kind::Service && shape.budget.is_some() => {
                    a.stop_within = Some(shape.budget.unwrap_or(secs(1)) + secs(1));
                    mutated = true;
                }
                Mutation::ServiceUnbounded if kind == Kind::Service && shape.budget.is_none() => {
                    a.stop_within = None;
                    mutated = true;
                }
                Mutation::PoolStarve if kind == Kind::Service => {
                    let pool = p.pool(&format!("{prefix}starved"), 1);
                    a.limit = Some(pool);
                    pools.push(pool);
                    starved = Some(pool);
                    mutated = true;
                }
                _ => {}
            }
        }
        if kind == Kind::Service {
            has_service = true;
        }
        lines.push(format!(
            "{name}: {} needs {}u{}{}{}",
            kind.label(),
            needs.units.len(),
            if needs.try_.is_some() { "+try" } else { "" },
            if needs.join.is_some() { "+join" } else { "" },
            a.describe()
        ));
        add_node(p, kind, &name, &needs, &a, keys);
        names.push(name);
    }
    if *mutation == Mutation::PoolStarve && mutated {
        // A non-resident user of *the starved pool* — `pools.last()` is
        // whatever `pool_of` made most recently, which is usually a different
        // pool, and then nothing is starved and the plan builds.
        let pool = starved.expect("the mutation made one");
        let a = Attrs {
            limit: Some(pool),
            ..Attrs::default()
        };
        add_node(
            p,
            Kind::Step,
            &format!("{prefix}starvedStep"),
            &Needs {
                units: vec![],
                try_: None,
                join: None,
            },
            &a,
            keys,
        );
    }
    if *mutation == Mutation::UnusedPool && !mutated {
        p.pool(&format!("{prefix}unused"), 1);
        mutated = true;
    }
    // A template, if this scope has a service to spawn it. Declared after the
    // scope's own nodes so it can import any of them; the late
    // `PlanBuilder::spawns` is what wires it, and it is also the only form
    // that can record `V-SPAWN-KIND` and `V-SPAWN-SELF-IMPORT`.
    let templated =
        super::templates::add_template(g, p, prefix, keys, shape, lines, mutation, &mut mutated);
    if templated {
        has_template = true;
    }
    // Every try-step gets a consumer, unless that is the mistake.
    let unconsumed: Vec<Key<Result<Unit, Error>>> = keys
        .trys
        .iter()
        .filter(|(_, used)| !used)
        .map(|(k, _)| *k)
        .collect();
    if *mutation == Mutation::TryUnconsumed && !mutated && !unconsumed.is_empty() {
        mutated = true;
    } else {
        for (i, t) in unconsumed.into_iter().enumerate() {
            let needs = Needs {
                units: vec![],
                try_: Some(t),
                join: None,
            };
            add_node(
                p,
                Kind::Step,
                &format!("{prefix}Consume{i}"),
                &needs,
                &Attrs::default(),
                keys,
            );
            lines.push(format!("{prefix}Consume{i}: step needs try"));
        }
    }
    for (_, used) in keys.trys.iter_mut() {
        *used = true;
    }
    if !mutated {
        *mutation = Mutation::None;
    }
    // `V-MODE` refuses `Finite` with a service **or a template**, and the
    // caller picks the mode.
    has_service || has_template
}
#[allow(clippy::too_many_arguments)]
fn child_plan(
    g: &mut SplitMix64,
    name: &str,
    parent_units: &[(Key<Unit>, bool)],
    parent: &Shape,
    lines: &mut Vec<String>,
    parent_allowed_components: bool,
) -> Result<Plan<Unit>, Invalid> {
    let parent_budget = parent.budget;
    let mut p = Plan::builder(name);
    let mut keys = Keys::default();
    for i in g.subset(parent_units.len(), 2) {
        let (parent, res) = parent_units[i];
        let k = p.import(parent);
        // The import stands for the parent's node, so if that is a resource
        // this child may *lock* it — the cross-scope arbitration `table.rs`
        // resolves and nothing in the walk used to reach.
        keys.units.push((k, res));
        lines.push(format!(
            "{name}/import: parent key {i}{}",
            if res { " (resource)" } else { "" }
        ));
    }
    let span = secs(g.range(1, parent_budget.map(|b| b.as_secs()).unwrap_or(10).max(1)));
    let budget = Some(span);
    let shape = Shape {
        policy: *g.pick(&[Policy::FailFast, Policy::Isolate]),
        shutdown: Shutdown::within(span),
        budget,
        bounded_root: parent.bounded_root,
        // A component of a template's plan is still inside that template, so
        // the nesting bound has to travel with it. Resetting it here let the
        // walk build `Tpl/Tpl/Tpl/Tpl/Tpl/…`.
        depth: parent.depth,
    };
    let mut none = Mutation::None;
    let count = g.range(1, 3) as usize;
    // Depth 2: a component inside a component, which is where
    // `component_faulted`'s grandparent recursion and `abandon_inner` under an
    // inner `abandon_all` live. Rare, so most children stay flat.
    let nest = parent_allowed_components && g.chance(0.35);
    let has_service = fill(
        g,
        &mut p,
        &format!("{name}/"),
        count,
        &shape,
        &mut keys,
        lines,
        &mut none,
        nest,
        &[],
    );
    let export = match keys
        .units
        .iter()
        .rev()
        .find(|(k, _)| k.raw().plan == p.id())
    {
        Some((k, _)) => *k,
        None => {
            let k = p
                .step(&format!("{name}/Out"))
                .run(|_cx, ()| async move { Ok(Unit) });
            lines.push(format!("{name}/Out: step"));
            k
        }
    };
    let mode = if has_service {
        Mode::Resident
    } else {
        *g.pick(&[Mode::Finite, Mode::Resident])
    };
    p.export(export).build(shape.policy, shape.shutdown, mode)
}
/// A random plan, valid unless a mutation says otherwise.
pub fn generate(g: &mut SplitMix64) -> Generated {
    let mut mutation = if g.chance(0.12) {
        *g.pick(&[
            Mutation::DupName,
            Mutation::LockNeeds,
            Mutation::IdempotentRequired,
            Mutation::BudgetOrder,
            Mutation::Mode,
            Mutation::TryUnconsumed,
            Mutation::UnusedPool,
            Mutation::ServiceUnbounded,
            Mutation::PoolStarve,
            Mutation::SpawnKind,
            Mutation::SpawnSelfImport,
        ])
    } else {
        Mutation::None
    };
    let unbounded = mutation == Mutation::ServiceUnbounded || g.chance(0.12);
    let budget = if unbounded {
        None
    } else {
        Some(secs(g.range(2, 12)))
    };
    let shape = Shape {
        policy: *g.pick(&[Policy::FailFast, Policy::Isolate]),
        shutdown: budget
            .map(Shutdown::within)
            .unwrap_or_else(Shutdown::unbounded),
        budget,
        bounded_root: budget.is_some(),
        depth: 0,
    };
    let mut p = Plan::builder("Mc");
    let mut keys = Keys::default();
    let mut lines = Vec::new();
    let count = g.range(1, 7) as usize;
    let has_service = fill(
        g,
        &mut p,
        "",
        count,
        &shape,
        &mut keys,
        &mut lines,
        &mut mutation,
        true,
        &[],
    );
    let mode = if mutation == Mutation::Mode {
        Mode::Finite
    } else if has_service {
        Mode::Resident
    } else {
        *g.pick(&[Mode::Finite, Mode::Finite, Mode::Resident])
    };
    lines.push(format!(
        "policy {:?}, shutdown {}, mode {mode}, mutation {}",
        shape.policy,
        shape.shutdown,
        mutation.name()
    ));
    Generated {
        plan: p.build(shape.policy, shape.shutdown, mode),
        expect_invalid: mutation.rule(),
        mutation: mutation.name(),
        lines,
    }
}
