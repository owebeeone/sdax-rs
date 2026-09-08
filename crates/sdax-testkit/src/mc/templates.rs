//! Templates for the walk: a nested plan a body instantiates at run time,
//! the service that declares it, and the two declaration mistakes only a
//! template can make.
//!
//! A generated template imports up to two of the enclosing scope's keys — the
//! interesting shape, because a live instance then holds those keys' releases
//! shut (INV-16) — takes a `u8` per instance, and has one node that reads it,
//! so the "need on the per-instance input" edge is exercised too.

use super::gen::{fill, Shape};
use super::keys::{secs, Keys, Unit};
use super::mutate::Mutation;
use super::prng::SplitMix64;
use sdax::*;
use std::sync::Arc;

/// One template plan: its imports, a body or two, and a node reading the
/// per-instance input.
fn template_plan(
    g: &mut SplitMix64,
    name: &str,
    parent_units: &[(Key<Unit>, bool)],
    parent: &Shape,
    lines: &mut Vec<String>,
) -> Result<(Plan<(), u8>, usize), Invalid> {
    let parent_budget = parent.budget;
    let mut p = Plan::with_input::<u8>(name);
    let input = p.input();
    let mut keys = Keys::default();
    let mut imported = 0usize;
    for i in g.subset(parent_units.len(), 2) {
        let (parent, res) = parent_units[i];
        let k = p.import(parent);
        keys.units.push((k, res));
        imported += 1;
        lines.push(format!(
            "{name}/import: parent key {i}{}",
            if res { " (resource)" } else { "" }
        ));
    }
    // The per-instance input is a real edge: a node needs it, so INV-16's
    // "started before its instance was spawned" rule has something to check.
    let cfg = p
        .step("Cfg")
        .needs(input)
        .run(|_cx, _v: Arc<u8>| async move { Ok(Unit) });
    keys.units.push((cfg, false));
    lines.push(format!("{name}/Cfg: step needs the per-instance input"));
    let span = secs(g.range(1, parent_budget.map(|b| b.as_secs()).unwrap_or(8).max(1)));
    let shape = Shape {
        policy: *g.pick(&[Policy::FailFast, Policy::Isolate]),
        shutdown: Shutdown::within(span),
        budget: Some(span),
        bounded_root: parent.bounded_root,
        depth: parent.depth + 1,
    };
    let mut none = Mutation::None;
    let count = g.range(1, 3) as usize;
    let has_service = fill(
        g,
        &mut p,
        &format!("{name}/"),
        count,
        &shape,
        &mut keys,
        lines,
        &mut none,
        false,
        &[],
    );
    // A child plan's `Mode` is a declaration only (contract § 2), but
    // `V-MODE` still reads it: `Finite` with a service in it is refused.
    let mode = if has_service {
        Mode::Resident
    } else {
        *g.pick(&[Mode::Finite, Mode::Resident])
    };
    let plan = p.build(shape.policy, shape.shutdown, mode)?;
    Ok((plan, imported))
}

/// Declare a template in `p` and give a node the right to instantiate it.
///
/// Returns whether one was declared. A scope with no service cannot spawn
/// (`V-SPAWN-KIND`), so most scopes get nothing.
#[allow(clippy::too_many_arguments)]
pub(crate) fn add_template<In>(
    g: &mut SplitMix64,
    p: &mut PlanBuilder<(), In>,
    prefix: &str,
    keys: &mut Keys,
    shape: &Shape,
    lines: &mut Vec<String>,
    mutation: &mut Mutation,
    mutated: &mut bool,
) -> bool {
    let deliberate =
        !*mutated && matches!(*mutation, Mutation::SpawnKind | Mutation::SpawnSelfImport);
    if shape.depth >= 2 {
        return false;
    }
    if !deliberate && !g.chance(0.45) {
        return false;
    }
    // `V-SPAWN-KIND`: only a service may spawn, so the mutation takes a
    // non-service key on purpose.
    let spawner = if deliberate && *mutation == Mutation::SpawnKind {
        keys.units
            .iter()
            .map(|(k, _)| *k)
            .find(|k| !keys.services.contains(k))
    } else {
        keys.services.last().copied()
    };
    let Some(spawner) = spawner else {
        return false;
    };
    // `V-SPAWN-SELF-IMPORT`: a template a service spawns must not import that
    // service's key — the instance could never be ready while the service
    // waits for it. Every other case keeps the spawner out of the candidates.
    let self_import = deliberate && *mutation == Mutation::SpawnSelfImport;
    let candidates: Vec<(Key<Unit>, bool)> = if self_import {
        vec![(spawner, false)]
    } else {
        keys.units
            .iter()
            .copied()
            .filter(|(k, _)| k.raw() != spawner.raw())
            .collect()
    };
    if self_import && candidates.is_empty() {
        return false;
    }
    let name = format!("{prefix}Tpl");
    let (plan, imported) = match template_plan(g, &name, &candidates, shape, lines) {
        Ok(v) => v,
        Err(_) => {
            lines.push(format!("{name}: template skipped (child invalid)"));
            return false;
        }
    };
    // `V-SPAWN-SELF-IMPORT` only fires if the import really landed.
    if self_import && imported == 0 {
        lines.push(format!("{name}: template skipped (no self-import landed)"));
        return false;
    }
    let t = p.template(&name, &plan);
    // The late form takes any key, so both mutations are *recorded* and the
    // build decides — which is the point of them.
    p.spawns(spawner, &t);
    lines.push(format!(
        "{name}: template of {} nodes, {imported} imports, spawned by {:?}",
        plan.inspect().nodes.len(),
        spawner.raw()
    ));
    if deliberate {
        *mutated = true;
    }
    true
}
