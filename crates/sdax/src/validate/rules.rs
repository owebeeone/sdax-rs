//! One function per validate rule, each with its decision procedure.
//!
//! Every rule reads the recorded declaration and nothing else, so `build`
//! stays pure: inspecting or validating a plan runs no body and performs no
//! effect.

use super::{Ctx, Rule};
use crate::key::RawKey;
use crate::plan::{Kind, PlanIr, Pool};
use crate::policy::{Ambiguity, Mode};

/// `V-EMPTY`: a plan whose only nodes are its input and its imports has
/// nothing to run.
pub(crate) fn empty(c: &mut Ctx<'_>) {
    let runnable =
        c.ir.nodes
            .iter()
            .filter(|n| !matches!(n.kind, Kind::Input | Kind::Import))
            .count();
    if runnable == 0 {
        c.add(
            Rule::Empty,
            Vec::new(),
            Vec::new(),
            format!("plan {:?} declares no nodes", c.ir.name),
            "declare at least one node, or drop the plan",
        );
    }
}

/// `V-FOREIGN-KEY`: for every node N, every key in `needs(N)`, `exclusive(N)`,
/// `shared(N)` and `spawns(N)`, and every pool N names, must belong to this
/// plan. An `Import` node's source is exempt: naming a parent key is what an
/// import is for.
pub(crate) fn foreign_key(c: &mut Ctx<'_>) {
    let id = c.ir.id;
    let mut findings = Vec::new();
    for n in &c.ir.nodes {
        let mut keys: Vec<RawKey> = Vec::new();
        if n.kind != Kind::Component && n.kind != Kind::Template {
            keys.extend(n.needs.iter().copied());
        }
        keys.extend(n.attrs.exclusive.iter().copied());
        keys.extend(n.attrs.shared.iter().copied());
        keys.extend(n.spawns.iter().copied());
        for k in keys {
            if k.plan != id {
                findings.push((
                    n.name.clone(),
                    k,
                    format!(
                        "node {:?} names key {}/{}, which belongs to plan {} and not to this plan ({})",
                        n.name, k.plan, k.idx, k.plan, id
                    ),
                ));
            }
        }
        for pool in [n.attrs.limit, n.attrs.pool].into_iter().flatten() {
            if pool.plan() != id {
                findings.push((
                    n.name.clone(),
                    RawKey {
                        plan: pool.plan(),
                        idx: pool.index(),
                    },
                    format!(
                        "node {:?} names a pool of plan {}, not of this plan ({})",
                        n.name,
                        pool.plan(),
                        id
                    ),
                ));
            }
        }
    }
    for (node, key, detail) in findings {
        c.add(
            Rule::ForeignKey,
            vec![node],
            vec![key],
            detail,
            "declare the node in this plan, or pass the value in with `import`",
        );
    }
}

/// `V-DUP-NAME`: node names within one scope are unique.
pub(crate) fn dup_name(c: &mut Ctx<'_>) {
    let mut dups = Vec::new();
    for (i, a) in c.ir.nodes.iter().enumerate() {
        if matches!(a.kind, Kind::Import | Kind::Input) {
            continue;
        }
        for b in &c.ir.nodes[i + 1..] {
            if a.name == b.name {
                dups.push((a.name.clone(), a.key, b.key));
            }
        }
    }
    for (name, first, second) in dups {
        c.add(
            Rule::DupName,
            vec![name.clone(), name.clone()],
            vec![first, second],
            format!("two nodes are named {name:?} in scope {:?}", c.ir.name),
            "rename one of them; names are how the report and the trace address a node",
        );
    }
}

/// `V-DUP-ATTR`: an attribute set twice on one node is a copy-paste mistake,
/// not a refinement; the builder records each second setting.
pub(crate) fn dup_attr(c: &mut Ctx<'_>) {
    let dups: Vec<(String, Vec<&'static str>)> =
        c.ir.nodes
            .iter()
            .filter_map(|n| {
                let mut seen: Vec<&'static str> = Vec::new();
                let mut twice: Vec<&'static str> = Vec::new();
                for a in &n.attrs.declared {
                    if seen.contains(a) {
                        if !twice.contains(a) {
                            twice.push(a);
                        }
                    } else {
                        seen.push(a);
                    }
                }
                (!twice.is_empty()).then(|| (n.name.clone(), twice))
            })
            .collect();
    for (name, attrs) in dups {
        c.add(
            Rule::DupAttr,
            vec![name.clone()],
            Vec::new(),
            format!("node {name:?} sets {} twice", attrs.join(", ")),
            "set the attribute once; the second value silently won",
        );
    }
}

/// `V-IMPORT-SCOPE`: every key a registered child plan imports must be a node
/// of the registering plan.
///
/// A deeper nesting declares its own `import` at each level, so a grandchild
/// names the child's import node rather than the grandparent's key directly.
pub(crate) fn import_scope(c: &mut Ctx<'_>) {
    let id = c.ir.id;
    let mut findings = Vec::new();
    for n in &c.ir.nodes {
        if !matches!(n.kind, Kind::Component | Kind::Template) {
            continue;
        }
        for k in &n.needs {
            if k.plan != id {
                findings.push((n.name.clone(), *k));
            }
        }
    }
    for (child, key) in findings {
        c.add(
            Rule::ImportScope,
            vec![child.clone()],
            vec![key],
            format!(
                "child plan {child:?} imports key {}/{}, which plan {} does not own",
                key.plan, key.idx, id
            ),
            "import the key at this level first, then pass this plan's key to the child",
        );
    }
}

/// `V-SPAWN-SELF-IMPORT`: for every service S and every template T in
/// `spawns(S)`, no key T imports may be S itself.
///
/// T's instances would wait for S to be `Ready`, and S is ready only when its
/// start body returns — which, if it awaits `Child::ready()`, waits for the
/// instance. Neither side can move.
pub(crate) fn spawn_self_import(c: &mut Ctx<'_>) {
    let mut findings = Vec::new();
    for n in &c.ir.nodes {
        for tpl_key in &n.spawns {
            let Some(tpl) = c.ir.node(*tpl_key) else {
                continue;
            };
            let Some(child) = tpl.child.as_ref() else {
                continue;
            };
            for src in child.nodes.iter().filter_map(|m| m.source) {
                if src == n.key {
                    findings.push((n.name.clone(), tpl.name.clone(), src));
                }
            }
        }
    }
    for (service, template, key) in findings {
        c.add(
            Rule::SpawnSelfImport,
            vec![service.clone(), template.clone()],
            vec![key],
            format!(
                "template {template:?} imports {service:?}, the service that spawns it: \
                 the instance can never be ready while {service:?} waits for it"
            ),
            "import what the instance really needs (the resource, not the service), \
             or let another node spawn the template",
        );
    }
}

/// `V-LOCK-NEEDS`: `exclusive(k)` or `shared(k)` requires `k ∈ needs(N)`. A
/// lock on a value the node never receives cannot be what the author meant.
pub(crate) fn lock_needs(c: &mut Ctx<'_>) {
    let mut findings = Vec::new();
    for n in &c.ir.nodes {
        for k in n.attrs.exclusive.iter().chain(n.attrs.shared.iter()) {
            if !n.needs.contains(k) {
                findings.push((n.name.clone(), *k));
            }
        }
    }
    for (node, key) in findings {
        let lock = c.name(key);
        c.add(
            Rule::LockNeeds,
            vec![node.clone()],
            vec![key],
            format!("node {node:?} locks {lock:?} but does not need it"),
            "add the resource to `needs`, or drop the lock",
        );
    }
}

/// `V-IDEMPOTENT-REQUIRED`: `retry` on an effect, `restart` on a service, or
/// `on_ambiguous ∈ {Compensate, Retry}` all mean the engine may act twice on
/// one external effect; each requires `.idempotent()`.
pub(crate) fn idempotent_required(c: &mut Ctx<'_>) {
    let mut findings = Vec::new();
    for n in &c.ir.nodes {
        if n.attrs.idempotent {
            continue;
        }
        let mut because = Vec::new();
        if n.kind == Kind::Effect && n.attrs.retry.is_some() {
            because.push("retry");
        }
        if n.kind == Kind::Service && n.attrs.restart.is_some() {
            because.push("restart");
        }
        if matches!(
            n.attrs.on_ambiguous,
            Some(Ambiguity::Compensate) | Some(Ambiguity::Retry)
        ) {
            because.push("on_ambiguous");
        }
        if !because.is_empty() {
            findings.push((n.name.clone(), because.join(" and ")));
        }
    }
    for (node, because) in findings {
        c.add(
            Rule::IdempotentRequired,
            vec![node.clone()],
            Vec::new(),
            format!("node {node:?} declares {because} but is not marked idempotent"),
            "add `.idempotent()` if repeating the action is safe, or drop the re-execution",
        );
    }
}

pub(crate) fn pool_users(ir: &PlanIr, pool: Pool) -> Vec<&crate::plan::NodeDecl> {
    ir.nodes
        .iter()
        .filter(|n| n.attrs.limit == Some(pool) || n.attrs.pool == Some(pool))
        .collect()
}

pub(crate) fn child_holds_pool(ir: &PlanIr, pool: Pool) -> bool {
    ir.nodes.iter().any(|n| {
        n.attrs.limit == Some(pool)
            || n.attrs.pool == Some(pool)
            || n.child
                .as_ref()
                .is_some_and(|ch| child_holds_pool(ch, pool))
    })
}

/// `V-POOL-STARVE`: count the pool's *resident* holders — services that take
/// it, and templates whose instances take it (unbounded, since instance count
/// is not known). If they can fill the pool and some non-resident node also
/// takes it, that node can never run.
pub(crate) fn pool_starve(c: &mut Ctx<'_>) {
    let mut findings = Vec::new();
    for (idx, decl) in c.ir.pools.iter().enumerate() {
        let pool = Pool {
            plan: c.ir.id,
            idx: idx as u32,
        };
        let users = pool_users(c.ir, pool);
        let mut resident = 0usize;
        let mut unbounded = false;
        for n in &users {
            match n.kind {
                Kind::Service => resident += 1,
                Kind::Template => unbounded = true,
                _ => {}
            }
        }
        if c.ir.nodes.iter().any(|n| {
            n.kind == Kind::Template
                && n.child
                    .as_ref()
                    .is_some_and(|ch| child_holds_pool(ch, pool))
        }) {
            unbounded = true;
        }
        let starved: Vec<String> = users
            .iter()
            .filter(|n| !matches!(n.kind, Kind::Service | Kind::Template))
            .map(|n| n.name.clone())
            .collect();
        if starved.is_empty() {
            continue;
        }
        if unbounded || resident >= decl.limit {
            let holders = if unbounded {
                "unbounded".to_string()
            } else {
                resident.to_string()
            };
            findings.push((
                starved.clone(),
                format!(
                    "pool {:?} has limit {} and {holders} resident holder(s); {} can never be granted",
                    decl.name,
                    decl.limit,
                    starved.join(", ")
                ),
            ));
        }
    }
    for (starved, detail) in findings {
        c.add(
            Rule::PoolStarve,
            starved,
            Vec::new(),
            detail,
            "raise the pool's limit, or take the pool off the resident holders",
        );
    }
}

/// `V-UNUSED-POOL`: a pool nobody takes is a declaration that means nothing.
pub(crate) fn unused_pool(c: &mut Ctx<'_>) {
    let mut findings = Vec::new();
    for (idx, decl) in c.ir.pools.iter().enumerate() {
        let pool = Pool {
            plan: c.ir.id,
            idx: idx as u32,
        };
        if !child_holds_pool(c.ir, pool) {
            findings.push(format!("pool {:?} has no user", decl.name));
        }
    }
    for detail in findings {
        c.add(
            Rule::UnusedPool,
            Vec::new(),
            Vec::new(),
            detail,
            "use the pool with `.limit(pool)` or `.on(pool)`, or remove it",
        );
    }
}

/// `V-SERVICE-UNBOUNDED`: with `Shutdown::unbounded()` nothing bounds a stop
/// but the service's own `stop_within`, so every service must declare one.
pub(crate) fn service_unbounded(c: &mut Ctx<'_>) {
    if c.ir.shutdown.budget().is_some() {
        return;
    }
    let names: Vec<String> =
        c.ir.nodes
            .iter()
            .filter(|n| n.kind == Kind::Service && n.attrs.stop_within.is_none())
            .map(|n| n.name.clone())
            .collect();
    for name in names {
        c.add(
            Rule::ServiceUnbounded,
            vec![name.clone()],
            Vec::new(),
            format!("service {name:?} has no stop_within and the shutdown budget is unbounded"),
            "declare `.stop_within(d)`, or bound the scope with `Shutdown::within(d)`",
        );
    }
}

/// `V-TRY-UNCONSUMED`: a try-step turns failure into a value; with no
/// dependent, that value is dropped and the failure vanishes.
pub(crate) fn try_unconsumed(c: &mut Ctx<'_>) {
    let unconsumed: Vec<String> =
        c.ir.nodes
            .iter()
            .filter(|n| n.kind == Kind::TryStep)
            .filter(|n| !c.ir.nodes.iter().any(|m| m.needs.contains(&n.key)))
            .map(|n| n.name.clone())
            .collect();
    if unconsumed.is_empty() {
        return;
    }
    let detail = format!("try_step(s) {} have no dependent", unconsumed.join(", "));
    c.add(
        Rule::TryUnconsumed,
        unconsumed,
        Vec::new(),
        detail,
        "let a node need the try_step's result, or make it a plain `step`",
    );
}

/// `V-BUDGET-ORDER`: `stop_within(d) ≤ Shutdown::within(D)`, and a child
/// plan's own budget ≤ the budget of the plan that registers it.
pub(crate) fn budget_order(c: &mut Ctx<'_>) {
    let Some(budget) = c.ir.shutdown.budget() else {
        return;
    };
    let mut findings = Vec::new();
    for n in &c.ir.nodes {
        if let Some(d) = n.attrs.stop_within {
            if d > budget {
                findings.push((
                    n.name.clone(),
                    format!(
                        "service {:?} declares stop_within {} inside a shutdown budget of {}",
                        n.name,
                        crate::view::human(d),
                        crate::view::human(budget)
                    ),
                ));
            }
        }
        if let Some(child) = &n.child {
            if let Some(inner) = child.shutdown.budget() {
                if inner > budget {
                    findings.push((
                        n.name.clone(),
                        format!(
                            "child plan {:?} has a shutdown budget of {} inside a parent budget of {}",
                            n.name,
                            crate::view::human(inner),
                            crate::view::human(budget)
                        ),
                    ));
                }
            } else {
                findings.push((
                    n.name.clone(),
                    format!(
                        "child plan {:?} is unbounded inside a bounded parent",
                        n.name
                    ),
                ));
            }
        }
    }
    for (node, detail) in findings {
        c.add(
            Rule::BudgetOrder,
            vec![node],
            Vec::new(),
            detail,
            "shorten the inner budget, or lengthen the scope's `Shutdown::within`",
        );
    }
}

/// `V-MODE`: `Mode::Finite` says the run ends when every node has settled. A
/// service never settles by itself and a template's instances outlive their
/// spawner, so a finite run would stop them the moment they became ready.
pub(crate) fn mode(c: &mut Ctx<'_>) {
    if c.ir.mode != Mode::Finite {
        return;
    }
    let offenders: Vec<String> =
        c.ir.nodes
            .iter()
            .filter(|n| matches!(n.kind, Kind::Service | Kind::Template))
            .map(|n| n.name.clone())
            .collect();
    for name in offenders {
        c.add(
            Rule::Mode,
            vec![name.clone()],
            Vec::new(),
            format!("{name:?} is long-lived, and this plan was built finite"),
            "build with `Mode::Resident`, or take the service or template out of this plan",
        );
    }
}
