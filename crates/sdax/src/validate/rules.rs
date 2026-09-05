//! The rules about a declaration's **shape**: which keys it names, which names
//! it reuses, which attributes it sets twice, and where a `spawns` may be
//! attached. The rules about policy, pools and budgets are in
//! [`super::budgets`].
//!
//! One function per rule, each with its decision procedure. Every rule reads
//! the recorded declaration and nothing else, so `build` stays pure:
//! inspecting or validating a plan runs no body and performs no effect.

use super::{Ctx, Rule};
use crate::key::RawKey;
use crate::plan::Kind;

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
///
/// A late `spawns(key, &template)` whose key is not a node of this plan is the
/// same mistake reached another way: the builder records it in
/// `PlanIr::foreign_spawns` and it is reported here, after the per-node
/// findings and in the order the declarations were written.
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
    for (key, tpl) in &c.ir.foreign_spawns {
        let template = c.name(*tpl);
        findings.push((
            template.clone(),
            *key,
            format!(
                "key {}/{} was declared to spawn template {template:?}, but it belongs to \
                 plan {} and not to this plan ({})",
                key.plan, key.idx, key.plan, id
            ),
        ));
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
///
/// Locks are checked **per key** rather than per name: `.exclusive(a)` and
/// `.exclusive(b)` are two different locks and are fine, while `.exclusive(a)`
/// twice, or `.exclusive(a).shared(a)`, is one resource claimed twice — the
/// second claim would silently win, and the two modes contradict each other.
pub(crate) fn dup_attr(c: &mut Ctx<'_>) {
    let mut findings: Vec<(String, Vec<RawKey>, String, &'static str)> = Vec::new();
    for n in &c.ir.nodes {
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
        if !twice.is_empty() {
            findings.push((
                n.name.clone(),
                Vec::new(),
                format!("node {:?} sets {} twice", n.name, twice.join(", ")),
                "set the attribute once; the second value silently won",
            ));
        }
        for (mode, list) in [
            ("exclusive", &n.attrs.exclusive),
            ("shared", &n.attrs.shared),
        ] {
            for k in repeats(list) {
                let lock = c.name(k);
                findings.push((
                    n.name.clone(),
                    vec![k],
                    format!("node {:?} takes {lock:?} {mode} twice", n.name),
                    "take the resource once; the second claim is the same lock",
                ));
            }
        }
        for k in dedup(&n.attrs.exclusive) {
            if n.attrs.shared.contains(&k) {
                let lock = c.name(k);
                findings.push((
                    n.name.clone(),
                    vec![k],
                    format!("node {:?} takes {lock:?} both exclusive and shared", n.name),
                    "take the resource in one mode; exclusive and shared cannot both hold",
                ));
            }
        }
    }
    for (name, keys, detail, fix) in findings {
        c.add(Rule::DupAttr, vec![name], keys, detail, fix);
    }
}

/// The keys of `list` in first-seen order, each once.
fn dedup(list: &[RawKey]) -> Vec<RawKey> {
    let mut out: Vec<RawKey> = Vec::new();
    for k in list {
        if !out.contains(k) {
            out.push(*k);
        }
    }
    out
}

/// The keys `list` names more than once, in first-seen order, each once.
fn repeats(list: &[RawKey]) -> Vec<RawKey> {
    let mut seen: Vec<RawKey> = Vec::new();
    let mut twice: Vec<RawKey> = Vec::new();
    for k in list {
        if seen.contains(k) {
            if !twice.contains(k) {
                twice.push(*k);
            }
        } else {
            seen.push(*k);
        }
    }
    twice
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

/// `V-SPAWN-KIND`: for every node N and every template T in `spawns(N)`, N is
/// a service.
///
/// The chain form exists only on `Node<_, _, Service>`, so this is about the
/// late form `PlanBuilder::spawns(key, &template)`, which takes any key. A
/// resource, a step or an effect has no scope of its own to put an instance
/// in, and nothing would ever stop what it spawned.
pub(crate) fn spawn_kind(c: &mut Ctx<'_>) {
    let mut findings = Vec::new();
    for n in &c.ir.nodes {
        if n.kind == Kind::Service {
            continue;
        }
        for tpl in &n.spawns {
            findings.push((n.name.clone(), c.name(*tpl), n.kind, *tpl));
        }
    }
    for (node, template, kind, key) in findings {
        c.add(
            Rule::SpawnKind,
            vec![node.clone(), template.clone()],
            vec![key],
            format!(
                "node {node:?} is a {} and was declared to spawn template {template:?}",
                kind.label()
            ),
            "only a service may spawn: declare it on the service that instantiates \
             the template, or make this node a service",
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
