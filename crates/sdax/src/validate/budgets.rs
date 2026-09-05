//! The rules about **policy, pools and budgets**: what re-execution costs,
//! which pool grants can never arrive, and which budget does not fit inside
//! the one that contains it. The rules about a declaration's shape are in
//! [`super::rules`].
//!
//! One function per rule, each with its decision procedure, under the same
//! purity rule as [`super::rules`].

use super::{Ctx, Rule};
use crate::plan::{Kind, PlanIr, Pool};
use crate::policy::{Ambiguity, Mode};

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
