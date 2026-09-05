//! The values a [`PlanView`](super::PlanView) is made of.

use crate::plan::Kind;

/// Where a node lives: its name, prefixed by the components it is nested in.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct NodePath(pub(crate) Vec<String>);

impl NodePath {
    /// A top-level node.
    pub fn root(name: &str) -> Self {
        NodePath(vec![name.to_string()])
    }

    /// This path with `name` appended.
    pub fn child(&self, name: &str) -> Self {
        let mut v = self.0.clone();
        v.push(name.to_string());
        NodePath(v)
    }

    /// The path's segments, outermost first.
    pub fn segments(&self) -> &[String] {
        &self.0
    }

    /// The node's own name, without its prefix.
    pub fn leaf(&self) -> &str {
        self.0.last().map(String::as_str).unwrap_or("")
    }
}

impl std::fmt::Display for NodePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0.join("/"))
    }
}

impl PartialEq<str> for NodePath {
    fn eq(&self, other: &str) -> bool {
        let mut rest = other;
        for (i, seg) in self.0.iter().enumerate() {
            if i > 0 {
                match rest.strip_prefix('/') {
                    Some(r) => rest = r,
                    None => return false,
                }
            }
            match rest.strip_prefix(seg.as_str()) {
                Some(r) => rest = r,
                None => return false,
            }
        }
        rest.is_empty()
    }
}

/// Why a node is not started yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reason {
    /// A `needs` edge.
    DeclaredNeed,
    /// A cross-scope `import`.
    Import,
    /// Another node holds the same resource exclusively.
    Exclusive,
    /// Another node holds the same resource, and this one wants it exclusively.
    Shared,
    /// A pool grant is not available.
    Pool,
    /// An earlier waiter for the same lock or pool has not been granted yet,
    /// so T1's FIFO refuses this one (contract § 1, T1).
    QueuedBehind,
    /// The scope has stopped admitting starts.
    ScopeNotAdmitting,
}

impl std::fmt::Display for Reason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Reason::DeclaredNeed => "declared need",
            Reason::Import => "import",
            Reason::Exclusive => "exclusive conflict",
            Reason::Shared => "shared conflict",
            Reason::Pool => "pool",
            Reason::QueuedBehind => "queued behind",
            Reason::ScopeNotAdmitting => "scope not admitting",
        })
    }
}

/// The answer to "why does this node wait?".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Why {
    /// The node asked about.
    pub node: NodePath,
    /// What it waits on, and why.
    pub waits_on: Vec<(NodePath, Reason)>,
}

/// A declared edge in the view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edge {
    /// The dependent.
    pub from: NodePath,
    /// What it depends on.
    pub to: NodePath,
    /// Why the edge exists.
    pub reason: Reason,
}

/// One node, with every attribute resolved to the value the engine will use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeView {
    /// Where the node lives.
    pub path: NodePath,
    /// What kind of node it is.
    pub kind: Kind,
    /// Its declared dependencies.
    pub needs: Vec<NodePath>,
    /// Templates it declared it may instantiate.
    pub spawns: Vec<NodePath>,
    /// The resolved attributes.
    pub attrs: Vec<(&'static str, String)>,
    /// Which of them the author declared explicitly; the rest are the engine's
    /// resolved defaults.
    pub declared: Vec<&'static str>,
}

impl NodeView {
    /// The resolved value of one attribute.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.as_str())
    }
}

/// A declared pool, as the view shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoolView {
    /// The scope that declared it: the empty path for the root plan, the
    /// component's path for a child plan's own pool. Pools are per plan
    /// (`V-FOREIGN-KEY`), so a name alone does not identify one.
    pub scope: NodePath,
    /// The author's name for it.
    pub name: String,
    /// How many nodes may hold it at once.
    pub limit: usize,
    /// Which nodes take it.
    pub users: Vec<NodePath>,
}

/// The nodes at or past the ship boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Effects {
    /// Resources (acquire), effects (perform) and persistent effects, in
    /// declaration order.
    pub nodes: Vec<NodePath>,
    /// The earliest layer containing one — the first thing that would happen.
    pub first_layer: Option<usize>,
}
