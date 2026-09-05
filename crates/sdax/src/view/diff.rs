//! Diffing two plan views.
//!
//! Every attribute in a [`NodeView`](super::NodeView) is a *resolved* value, so
//! a diff shows a change of authored intent and a change of engine default the
//! same way. When neither side declared the attribute and the value moved, the
//! change is marked `default_changed`: that is the crate-upgrade case, and it
//! is what makes an upgrade reviewable as a diff.

use super::{NodePath, PlanView};

/// One attribute whose resolved value changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttrChange {
    /// Which node.
    pub node: NodePath,
    /// Which attribute.
    pub attr: &'static str,
    /// Its resolved value on the left.
    pub from: String,
    /// Its resolved value on the right.
    pub to: String,
    /// True when neither side declared it, so the engine's default moved.
    pub default_changed: bool,
}

/// What changed between two plan views.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PlanDiff {
    /// Nodes only the right side has.
    pub added_nodes: Vec<NodePath>,
    /// Nodes only the left side has.
    pub removed_nodes: Vec<NodePath>,
    /// Edges only the right side has.
    pub added_edges: Vec<(NodePath, NodePath)>,
    /// Edges only the left side has.
    pub removed_edges: Vec<(NodePath, NodePath)>,
    /// Attributes whose resolved value changed.
    pub changed: Vec<AttrChange>,
    /// Scope-level changes: policy, shutdown, mode.
    pub scope: Vec<AttrChange>,
}

impl PlanDiff {
    /// Whether the two views are identical.
    pub fn is_empty(&self) -> bool {
        self.added_nodes.is_empty()
            && self.removed_nodes.is_empty()
            && self.added_edges.is_empty()
            && self.removed_edges.is_empty()
            && self.changed.is_empty()
            && self.scope.is_empty()
    }
}

impl std::fmt::Display for PlanDiff {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_empty() {
            return f.write_str("no change");
        }
        for n in &self.added_nodes {
            writeln!(f, "+ node {n}")?;
        }
        for n in &self.removed_nodes {
            writeln!(f, "- node {n}")?;
        }
        for (a, b) in &self.added_edges {
            writeln!(f, "+ edge {a}→{b}")?;
        }
        for (a, b) in &self.removed_edges {
            writeln!(f, "- edge {a}→{b}")?;
        }
        for c in self.scope.iter().chain(self.changed.iter()) {
            let note = if c.default_changed {
                "  [engine default changed]"
            } else {
                ""
            };
            writeln!(f, "~ {}.{}: {} → {}{}", c.node, c.attr, c.from, c.to, note)?;
        }
        Ok(())
    }
}

impl PlanView {
    /// Compare this view with another: added and removed nodes and edges, and
    /// every attribute or resolved default whose value changed.
    pub fn diff(&self, other: &PlanView) -> PlanDiff {
        let mut d = PlanDiff::default();
        for n in &other.nodes {
            if self.node(n.path.to_string().as_str()).is_none() {
                d.added_nodes.push(n.path.clone());
            }
        }
        for n in &self.nodes {
            if other.node(n.path.to_string().as_str()).is_none() {
                d.removed_nodes.push(n.path.clone());
            }
        }
        let left: Vec<(NodePath, NodePath)> = self
            .edges
            .iter()
            .map(|e| (e.from.clone(), e.to.clone()))
            .collect();
        let right: Vec<(NodePath, NodePath)> = other
            .edges
            .iter()
            .map(|e| (e.from.clone(), e.to.clone()))
            .collect();
        for e in &right {
            if !left.contains(e) {
                d.added_edges.push(e.clone());
            }
        }
        for e in &left {
            if !right.contains(e) {
                d.removed_edges.push(e.clone());
            }
        }
        for a in &self.nodes {
            let Some(b) = other.node(a.path.to_string().as_str()) else {
                continue;
            };
            for (attr, from) in &a.attrs {
                let Some(to) = b.attr(attr) else { continue };
                if from != to {
                    d.changed.push(AttrChange {
                        node: a.path.clone(),
                        attr,
                        from: from.clone(),
                        to: to.to_string(),
                        default_changed: !a.declared.contains(attr) && !b.declared.contains(attr),
                    });
                }
            }
        }
        for (attr, from, to) in [(
            "policy",
            super::policy_label(self.policy),
            super::policy_label(other.policy),
        )] {
            if from != to {
                d.scope.push(AttrChange {
                    node: NodePath::root(&self.name),
                    attr,
                    from: from.to_string(),
                    to: to.to_string(),
                    default_changed: false,
                });
            }
        }
        for (attr, from, to) in [
            (
                "shutdown",
                self.shutdown.to_string(),
                other.shutdown.to_string(),
            ),
            ("mode", self.mode.to_string(), other.mode.to_string()),
        ] {
            if from != to {
                d.scope.push(AttrChange {
                    node: NodePath::root(&self.name),
                    attr,
                    from,
                    to,
                    default_changed: false,
                });
            }
        }
        d
    }
}
