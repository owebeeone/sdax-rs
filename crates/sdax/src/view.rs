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
        flatten(ir, &NodePath::default(), &mut v, &mut resolver);
        for (idx, decl) in ir.pools.iter().enumerate() {
            let users = ir
                .nodes
                .iter()
                .filter(|n| {
                    n.attrs.limit.map(|p| p.index()) == Some(idx as u32)
                        || n.attrs.pool.map(|p| p.index()) == Some(idx as u32)
                })
                .map(|n| NodePath::root(&n.name))
                .collect();
            v.pools.push(PoolView {
                name: decl.name.clone(),
                limit: decl.limit,
                users,
            });
        }
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

#[derive(Default)]
struct Resolver {
    /// Maps a child plan's import node to the parent path it stands for.
    imports: Vec<(RawKey, NodePath)>,
}

impl Resolver {
    fn record(&mut self, key: RawKey, path: NodePath) {
        self.imports.push((key, path));
    }
    fn resolve(&self, key: RawKey) -> Option<NodePath> {
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
                    res.record(n.key, target);
                }
            }
        }
    }
    for n in &ir.nodes {
        if matches!(n.kind, Kind::Import | Kind::Input) {
            continue;
        }
        let path = path_of(n);
        let mut needs = Vec::new();
        for k in &n.needs {
            let to = resolve_need(ir, prefix, res, *k);
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
            spawns: n
                .spawns
                .iter()
                .filter_map(|k| ir.node(*k).map(&path_of))
                .collect(),
            attrs: resolved_attrs(ir, n),
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
                res.record(input, path.clone());
            }
            flatten(child, &path, out, res);
        }
    }
}

fn resolve_need(ir: &PlanIr, prefix: &NodePath, res: &Resolver, k: RawKey) -> NodePath {
    if let Some(p) = res.resolve(k) {
        return p;
    }
    match ir.node(k) {
        Some(d) if prefix.segments().is_empty() => NodePath::root(&d.name),
        Some(d) => prefix.child(&d.name),
        None => NodePath::root(&format!("{}/{}", k.plan, k.idx)),
    }
}

fn resolved_attrs(ir: &PlanIr, n: &NodeDecl) -> Vec<(&'static str, String)> {
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
    if let Some(k) = a.exclusive.first() {
        v.push((
            "exclusive",
            ir.node(*k).map(|d| d.name.clone()).unwrap_or_default(),
        ));
    }
    if let Some(k) = a.shared.first() {
        v.push((
            "shared",
            ir.node(*k).map(|d| d.name.clone()).unwrap_or_default(),
        ));
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
