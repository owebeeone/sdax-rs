//! Inspection: what a plan is, before anything runs.
//!
//! [`Plan::inspect`](crate::Plan::inspect) is pure. It reads the declaration
//! and nothing else, so "no effect" holds by construction: there is no body to
//! run and, in Stage 0, no engine to run it. The view renders as text
//! (`Display`), answers `why` a node waits, lists what is at or past the ship
//! boundary, and diffs against another view so a configuration change or a
//! crate upgrade is reviewed rather than discovered.

use crate::key::RawKey;
use crate::plan::{Kind, NodeDecl, Plan, PlanIr, ReleaseStyle};
use crate::policy::{Ambiguity, Mode, Policy, Shutdown};
use std::time::Duration;

mod diff;
mod model;
mod render;

pub use diff::{AttrChange, PlanDiff};
pub use model::{Edge, Effects, NodePath, NodeView, PoolView, Reason, Why};

/// Render a duration the way `inspect()` prints it: `10s`, `1500ms`.
pub(crate) fn human(d: Duration) -> String {
    if d.subsec_nanos() == 0 {
        format!("{}s", d.as_secs())
    } else if d.as_secs() == 0 {
        format!("{}ms", d.as_millis())
    } else {
        format!("{:.3}s", d.as_secs_f64())
    }
}

/// A plan as data: everything `inspect()` knows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanView {
    /// The plan's name.
    pub name: String,
    /// The semantics tag.
    pub semantics: &'static str,
    /// The declared run mode.
    pub mode: Mode,
    /// The declared fail policy.
    pub policy: Policy,
    /// The declared shutdown budget.
    pub shutdown: Shutdown,
    /// Every node, flattened, in declaration order; a component's or
    /// template's own nodes follow it under its path.
    pub nodes: Vec<NodeView>,
    /// Exactly the declared edges.
    pub edges: Vec<Edge>,
    /// Declared pools and their users.
    pub pools: Vec<PoolView>,
}

impl PlanView {
    /// Build a view from a recorded declaration.
    pub(crate) fn of(ir: &PlanIr) -> PlanView {
        let mut v = PlanView {
            name: ir.name.clone(),
            semantics: ir.semantics,
            mode: ir.mode,
            policy: ir.policy,
            shutdown: ir.shutdown,
            nodes: Vec::new(),
            edges: Vec::new(),
            pools: Vec::new(),
        };
        let mut resolver = Resolver::default();
        if let Some(input) = ir.input {
            resolver.record(input, Resolution::SuppliedInput);
        }
        flatten(ir, &NodePath::default(), &mut v, &mut resolver);
        v
    }

    /// The node at this path.
    pub fn node(&self, path: &str) -> Option<&NodeView> {
        self.nodes.iter().find(|n| n.path == *path)
    }

    /// Earliest-start layering: where a node could start if every body took one
    /// tick. **Not** barriers — nothing waits for a layer, only for its needs.
    pub fn layers(&self) -> Vec<Vec<NodePath>> {
        let depth = self.depths();
        let max = depth.iter().copied().max().map(|m| m + 1).unwrap_or(0);
        let mut out = vec![Vec::new(); max];
        for (i, n) in self.nodes.iter().enumerate() {
            out[depth[i]].push(n.path.clone());
        }
        out
    }

    fn depths(&self) -> Vec<usize> {
        let mut depth = vec![0usize; self.nodes.len()];
        // Nodes are in declaration order and every edge points backwards, so a
        // single forward pass is a longest-path computation.
        for i in 0..self.nodes.len() {
            let mut d = 0;
            for e in self.edges.iter().filter(|e| e.from == self.nodes[i].path) {
                if let Some(j) = self.index_of(&e.to) {
                    d = d.max(depth[j] + 1);
                }
            }
            depth[i] = d;
        }
        depth
    }

    fn index_of(&self, path: &NodePath) -> Option<usize> {
        self.nodes.iter().position(|n| &n.path == path)
    }

    /// The partial order in which obligations are discharged.
    ///
    /// A node's release starts only once every node that needs or imports it
    /// has finished its own cleanup (INV-5); nodes unrelated by that relation
    /// may overlap (INV-6). No total order is promised, and none is implied.
    pub fn release_order(&self) -> ReleaseOrder {
        let n = self.nodes.len();
        // `after[a][b]`: a's release must finish before b's starts.
        let mut after = vec![vec![false; n]; n];
        for e in &self.edges {
            if let (Some(from), Some(to)) = (self.index_of(&e.from), self.index_of(&e.to)) {
                after[from][to] = true;
            }
        }
        // A component's or template's inner nodes are cleaned up as one unit:
        // the unit ends before anything the unit depends on is released.
        for (i, node) in self.nodes.iter().enumerate() {
            for (j, other) in self.nodes.iter().enumerate() {
                if i != j && is_inside(&node.path, &other.path) {
                    after[i][j] = true;
                }
            }
        }
        // Transitive closure (Floyd–Warshall over a small graph).
        for k in 0..n {
            for i in 0..n {
                if !after[i][k] {
                    continue;
                }
                let row_k = after[k].clone();
                for (dst, src) in after[i].iter_mut().zip(row_k) {
                    *dst |= src;
                }
            }
        }
        ReleaseOrder {
            paths: self.nodes.iter().map(|x| x.path.clone()).collect(),
            after,
        }
    }

    /// Why a node waits: its declared needs, its imports, the holders of any
    /// lock it wants, and any pool it must be granted.
    pub fn why(&self, node: &str) -> Option<Why> {
        let target = self.node(node)?;
        let mut waits_on: Vec<(NodePath, Reason)> = self
            .edges
            .iter()
            .filter(|e| e.from == target.path)
            .map(|e| (e.to.clone(), e.reason))
            .collect();
        if let Some(locked) = target.attr("exclusive") {
            for other in self.nodes.iter().filter(|o| o.path != target.path) {
                let conflicts =
                    other.attr("exclusive") == Some(locked) || other.attr("shared") == Some(locked);
                if conflicts {
                    waits_on.push((other.path.clone(), Reason::Exclusive));
                }
            }
        }
        if let Some(pool) = target.attr("pool").or_else(|| target.attr("limit")) {
            waits_on.push((NodePath::root(pool), Reason::Pool));
        }
        Some(Why {
            node: target.path.clone(),
            waits_on,
        })
    }

    /// The nodes at or past the ship boundary, and the earliest layer holding
    /// one.
    pub fn effects(&self) -> Effects {
        let nodes: Vec<NodePath> = self
            .nodes
            .iter()
            .filter(|n| n.kind.can_hold())
            .map(|n| n.path.clone())
            .collect();
        let first_layer = self
            .layers()
            .iter()
            .position(|layer| layer.iter().any(|p| nodes.contains(p)));
        Effects { nodes, first_layer }
    }
}

fn is_inside(inner: &NodePath, outer: &NodePath) -> bool {
    inner.segments().len() > outer.segments().len()
        && inner.segments().starts_with(outer.segments())
}

/// Preserve supplied-input provenance through arbitrarily nested imports.
#[derive(Clone)]
enum Resolution {
    Path(NodePath),
    SuppliedInput,
}

impl Resolution {
    fn path(self) -> Option<NodePath> {
        match self {
            Self::Path(path) => Some(path),
            Self::SuppliedInput => None,
        }
    }
}

#[derive(Default)]
struct Resolver {
    /// An import can name a lifecycle node or an already-supplied input.
    imports: Vec<(RawKey, Resolution)>,
}

impl Resolver {
    fn record(&mut self, key: RawKey, path: Resolution) {
        self.imports.push((key, path));
    }
    fn resolve(&self, key: RawKey) -> Option<Resolution> {
        self.imports
            .iter()
            .rev()
            .find(|(k, _)| *k == key)
            .map(|(_, p)| p.clone())
    }
}

fn flatten(ir: &PlanIr, prefix: &NodePath, out: &mut PlanView, res: &mut Resolver) {
    let path_of = |n: &NodeDecl| -> NodePath {
        if prefix.segments().is_empty() {
            NodePath::root(&n.name)
        } else {
            prefix.child(&n.name)
        }
    };
    // An import node stands for whatever the parent key resolves to. The
    // parent recorded that mapping before recursing; only a root plan's own
    // (unresolved) imports are filled in here.
    for n in &ir.nodes {
        if n.kind == Kind::Import {
            if let Some(src) = n.source {
                if res.resolve(n.key).is_none() {
                    let target = NodePath::root(
                        &ir.node(src)
                            .map(|d| d.name.clone())
                            .unwrap_or_else(|| src.idx.to_string()),
                    );
                    res.record(n.key, Resolution::Path(target));
                }
            }
        }
    }
    // Every scope's pools, not only the root's: a child plan declares its own,
    // and a reader of `inspect()` — or a checker recomputing the pool clause of
    // INV-1 — could not see them at all.
    for (idx, decl) in ir.pools.iter().enumerate() {
        let users = ir
            .nodes
            .iter()
            .filter(|n| {
                n.attrs.limit.map(|p| p.index()) == Some(idx as u32)
                    || n.attrs.pool.map(|p| p.index()) == Some(idx as u32)
            })
            .map(&path_of)
            .collect();
        out.pools.push(PoolView {
            scope: prefix.clone(),
            name: decl.name.clone(),
            limit: decl.limit,
            users,
        });
    }
    for n in &ir.nodes {
        if matches!(n.kind, Kind::Import | Kind::Input) {
            continue;
        }
        let path = path_of(n);
        // Locks resolve the way `needs` do: a lock on an imported resource
        // names the parent's node, not the child's import stub, so a reader —
        // and a checker recomputing INV-1's exclusion clause — can see that two
        // scopes contend for one resource.
        let locks = Locks {
            exclusive: n
                .attrs
                .exclusive
                .iter()
                .filter_map(|k| resolve_need(ir, prefix, res, *k).path())
                .collect(),
            shared: n
                .attrs
                .shared
                .iter()
                .filter_map(|k| resolve_need(ir, prefix, res, *k).path())
                .collect(),
        };
        let mut needs = Vec::new();
        for k in &n.needs {
            // Only supplied input has no lifecycle node. Unknown keys still
            // retain their diagnostic path rather than being silently hidden.
            let Some(to) = resolve_need(ir, prefix, res, *k).path() else {
                continue;
            };
            out.edges.push(Edge {
                from: path.clone(),
                to: to.clone(),
                reason: if ir.node(*k).map(|d| d.kind) == Some(Kind::Import) {
                    Reason::Import
                } else {
                    Reason::DeclaredNeed
                },
            });
            needs.push(to);
        }
        out.nodes.push(NodeView {
            path: path.clone(),
            kind: n.kind,
            needs,
            // `spawns` is a service's declaration (F1). Any other kind
            // carrying one is `V-SPAWN-KIND` and never reaches a built plan,
            // so the view does not show it there either.
            spawns: if n.kind == Kind::Service {
                n.spawns
                    .iter()
                    .filter_map(|k| ir.node(*k).map(&path_of))
                    .collect()
            } else {
                Vec::new()
            },
            attrs: resolved_attrs(ir, n, &locks),
            declared: n.attrs.declared.clone(),
        });
        if let Some(child) = &n.child {
            // Each import of the child stands for the parent key it names, and
            // the child's per-instance input stands for the node that
            // instantiates it.
            for (key, src) in child
                .nodes
                .iter()
                .filter_map(|m| m.source.map(|s| (m.key, s)))
            {
                let target = resolve_need(ir, prefix, res, src);
                res.record(key, target);
            }
            if let Some(input) = child.input {
                res.record(input, Resolution::Path(path.clone()));
            }
            flatten(child, &path, out, res);
        }
    }
}

fn resolve_need(ir: &PlanIr, prefix: &NodePath, res: &Resolver, k: RawKey) -> Resolution {
    if let Some(p) = res.resolve(k) {
        return p;
    }
    Resolution::Path(match ir.node(k) {
        Some(d) if prefix.segments().is_empty() => NodePath::root(&d.name),
        Some(d) => prefix.child(&d.name),
        None => NodePath::root(&format!("{}/{}", k.plan, k.idx)),
    })
}

/// A node's locks, resolved to the paths they really name.
struct Locks {
    exclusive: Vec<NodePath>,
    shared: Vec<NodePath>,
}

fn join_paths(v: &[NodePath]) -> String {
    v.iter()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

fn resolved_attrs(ir: &PlanIr, n: &NodeDecl, locks: &Locks) -> Vec<(&'static str, String)> {
    let a = &n.attrs;
    let mut v: Vec<(&'static str, String)> = Vec::new();
    v.push(("release", a.release.label().to_string()));
    v.push(("within", a.within.map(human).unwrap_or_else(|| "—".into())));
    v.push((
        "retry",
        a.retry
            .map(|r| format!("{} attempts", r.max_attempts()))
            .unwrap_or_else(|| "—".into()),
    ));
    v.push((
        "restart",
        a.restart
            .map(|_| "on error".to_string())
            .unwrap_or_else(|| "—".into()),
    ));
    v.push((
        "idempotent",
        if a.idempotent {
            "yes".into()
        } else {
            "no".into()
        },
    ));
    v.push((
        "terminal",
        if a.terminal {
            "yes".into()
        } else {
            "no".into()
        },
    ));
    v.push(("cancel", a.cancel.to_string()));
    if n.kind == Kind::Service {
        v.push((
            "stop_within",
            match (a.stop_within, ir.shutdown.budget()) {
                (Some(d), Some(b)) => format!("{} (of {})", human(d), human(b)),
                (Some(d), None) => human(d),
                (None, Some(b)) => format!("— (bounded by shutdown {})", human(b)),
                (None, None) => "—".into(),
            },
        ));
    }
    if let Some(amb) = a.on_ambiguous {
        v.push(("ambiguous", ambiguity_label(amb).to_string()));
    }
    if !locks.exclusive.is_empty() {
        v.push(("exclusive", join_paths(&locks.exclusive)));
    }
    if !locks.shared.is_empty() {
        v.push(("shared", join_paths(&locks.shared)));
    }
    if let Some(p) = a.pool {
        v.push(("pool", ir.pools[p.index() as usize].name.clone()));
    }
    if let Some(p) = a.limit {
        v.push(("limit", ir.pools[p.index() as usize].name.clone()));
    }
    v
}

fn ambiguity_label(a: Ambiguity) -> &'static str {
    match a {
        Ambiguity::Report => "report",
        Ambiguity::Compensate => "compensate",
        Ambiguity::Retry => "retry",
    }
}

/// The partial order of cleanup, with the unordered pairs exposed rather than
/// flattened into a sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseOrder {
    paths: Vec<NodePath>,
    after: Vec<Vec<bool>>,
}

impl ReleaseOrder {
    /// Whether `a`'s cleanup must end before `b`'s begins.
    pub fn before(&self, a: &str, b: &str) -> bool {
        match (self.index(a), self.index(b)) {
            (Some(i), Some(j)) => self.after[i][j],
            _ => false,
        }
    }

    /// Whether the two cleanups are unrelated and so may overlap.
    pub fn unordered(&self, a: &str, b: &str) -> bool {
        match (self.index(a), self.index(b)) {
            (Some(i), Some(j)) => i != j && !self.after[i][j] && !self.after[j][i],
            _ => false,
        }
    }

    /// Every pair that may overlap.
    pub fn unordered_pairs(&self) -> Vec<(NodePath, NodePath)> {
        let mut out = Vec::new();
        for i in 0..self.paths.len() {
            for j in i + 1..self.paths.len() {
                if !self.after[i][j] && !self.after[j][i] {
                    out.push((self.paths[i].clone(), self.paths[j].clone()));
                }
            }
        }
        out
    }

    /// Cleanup waves: a rendering of the order, earliest first. Members of one
    /// wave are unordered with each other; members of different waves need not
    /// be ordered, so `unordered` is the authority.
    pub fn waves(&self) -> Vec<Vec<NodePath>> {
        let n = self.paths.len();
        let mut depth = vec![0usize; n];
        for _ in 0..n {
            for i in 0..n {
                for j in 0..n {
                    if self.after[i][j] {
                        depth[j] = depth[j].max(depth[i] + 1);
                    }
                }
            }
        }
        let max = depth.iter().copied().max().map(|m| m + 1).unwrap_or(0);
        let mut out = vec![Vec::new(); max];
        for (i, p) in self.paths.iter().enumerate() {
            out[depth[i]].push(p.clone());
        }
        out
    }

    fn index(&self, name: &str) -> Option<usize> {
        self.paths.iter().position(|p| *p == *name)
    }
}

impl<Out, In> Plan<Out, In> {
    /// The plan as data: pure, and available before anything runs.
    pub fn inspect(&self) -> PlanView {
        PlanView::of(self.ir())
    }

    /// The nodes at or past the ship boundary, and the earliest layer that
    /// holds one.
    pub fn effects(&self) -> Effects {
        self.inspect().effects()
    }
}

impl std::fmt::Display for PlanView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut s = String::new();
        render::render(self, &mut s).map_err(|_| std::fmt::Error)?;
        f.write_str(&s)
    }
}

/// Shared by the renderer.
pub(crate) fn policy_label(p: Policy) -> &'static str {
    match p {
        Policy::FailFast => "fail-fast",
        Policy::Isolate => "isolate",
    }
}

pub(crate) fn kind_label(n: &NodeView) -> String {
    if n.kind == Kind::Effect && n.attr("release") == Some(ReleaseStyle::Persistent.label()) {
        "effect (persistent)".to_string()
    } else {
        n.kind.label().to_string()
    }
}
