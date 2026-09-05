//! **Occurrences**: which copy of a declared node a trace event is about.
//!
//! A path names a *declaration*. With templates one declaration has many live
//! copies — one per instance — and they all write to the same path, so a rule
//! that groups events by path alone merges two instances into one node and
//! then reports nonsense about it (`Link/Sock attempt 1 started while attempt
//! 1 was in flight`). Worse, a rule that merged them could never fail on a
//! real instance bug, which is the thing this checker exists to catch.
//!
//! Every record carries the answer already: [`RecordOrder::steps`](sdax::RecordOrder::steps) is one
//! `(declaration index, instance)` pair per level, so the instance chain along
//! the path *is* the identity of the copy. An [`Occ`] is that chain, one entry
//! per path segment, and `(path, occ)` is what every rule below groups by.

use sdax::host::InstanceId;
use sdax::{NodePath, PlanView, Trace, TraceEvent};

/// Which copy of a declared node: one `Option<InstanceId>` per path segment.
/// The static graph is `None` at every level.
pub type Occ = Vec<Option<InstanceId>>;

/// The occurrence an observation is about.
pub fn occ_of(e: &TraceEvent) -> Occ {
    e.order
        .as_ref()
        .map(|o| o.steps.iter().map(|(_, i)| *i).collect())
        .unwrap_or_default()
}

/// The occurrence of a node of the static graph.
pub fn static_occ(path: &NodePath) -> Occ {
    vec![None; path.segments().len()]
}

/// Whether this occurrence is part of the static graph.
pub fn is_static(occ: &Occ) -> bool {
    occ.iter().all(|i| i.is_none())
}

/// The instance a node belongs to: the deepest one named along its path.
pub fn instance(occ: &Occ) -> Option<InstanceId> {
    occ.iter().rev().find_map(|i| *i)
}

/// The occurrence of `target` that the node at `(path, occ)` names.
///
/// Every edge a node can name — a `needs`, an `import` resolved to the
/// ancestor it stands for, a lock, the template its per-instance input comes
/// from — points at a node whose path shares this one's prefix, because
/// INV-16 forbids naming *into* a scope. So the copy it means is this node's
/// own chain truncated to the target's depth, with the target's own level
/// static: an instance's node names its instance's copy of a sibling and the
/// one shared copy of an ancestor.
pub fn occ_of_target(path: &NodePath, occ: &Occ, target: &NodePath) -> Occ {
    let d = target.segments().len();
    if d == 0 {
        return Vec::new();
    }
    let shares =
        target.segments()[..d - 1] == path.segments()[..(d - 1).min(path.segments().len())];
    if !shares || occ.len() < d - 1 {
        return static_occ(target);
    }
    let mut out: Occ = occ[..d - 1].to_vec();
    out.push(None);
    out
}

/// The copy of a **scope** — a pool's, say — that a node inside it belongs to:
/// its chain truncated to the scope's depth. The root scope is the empty
/// chain, so every node of a run shares it.
pub fn occ_of_scope(occ: &Occ, scope_depth: usize) -> Occ {
    occ[..scope_depth.min(occ.len())].to_vec()
}

/// Whether `(path, occ)` lies inside the instance `(root, id)` — the subtree a
/// template's instance owns.
pub fn inside_instance(path: &NodePath, occ: &Occ, root: &NodePath, id: InstanceId) -> bool {
    let d = root.segments().len();
    path.segments().len() > d
        && path.segments()[..d] == root.segments()[..]
        && occ.get(d - 1).copied().flatten() == Some(id)
}

/// Every `(path, occ)` the trace observed, plus the static occurrence of every
/// node the plan declares — so a node with no events at all is still a node
/// the rules consider.
pub fn occurrences(trace: &Trace, view: &PlanView) -> Vec<(NodePath, Occ)> {
    let mut out: Vec<(NodePath, Occ)> = view
        .nodes
        .iter()
        .map(|n| (n.path.clone(), static_occ(&n.path)))
        .collect();
    for e in &trace.events {
        let Some(path) = &e.node else { continue };
        let occ = occ_of(e);
        if !out.iter().any(|(p, o)| p == path && *o == occ) {
            out.push((path.clone(), occ));
        }
    }
    out
}

/// How a violation names one copy: the path alone for the static graph,
/// `Link/Sock#2` for an instance's.
pub fn label(path: &NodePath, occ: &Occ) -> String {
    match instance(occ) {
        None => path.to_string(),
        Some(id) => format!("{path}{id}"),
    }
}
