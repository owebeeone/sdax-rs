//! Data bindings, including supplied inputs omitted from the lifecycle view.

use super::{Edge, NodePath, Reason};
use crate::key::RawKey;
use crate::plan::{Kind, Plan, PlanIr};
use std::collections::BTreeMap;

/// One declaration in the data-provenance view. Inputs and imports are data
/// sources; appearing here does not give them a cleanup obligation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataNode {
    /// Fully qualified declaration path.
    pub path: NodePath,
    /// Declaration category, including inputs and imports.
    pub kind: Kind,
}

/// Complete declared data bindings. This view does not prove what user bodies
/// do with values, nor whether a returned value contains a live capability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataflowView {
    /// Nodes in declaration traversal order, including supplied inputs.
    pub nodes: Vec<DataNode>,
    /// Declared needs and explicit import bindings, with the consumer first.
    pub edges: Vec<Edge>,
}

fn paths(
    ir: &PlanIr,
    prefix: &NodePath,
    names: &mut BTreeMap<RawKey, NodePath>,
    nodes: &mut Vec<DataNode>,
) {
    for node in &ir.nodes {
        let path = prefix.child(&node.name);
        names.insert(node.key, path.clone());
        nodes.push(DataNode {
            path: path.clone(),
            kind: node.kind,
        });
        if let Some(child) = &node.child {
            paths(child, &path, names, nodes);
        }
    }
}

fn edges(ir: &PlanIr, names: &BTreeMap<RawKey, NodePath>, out: &mut Vec<Edge>) {
    for node in &ir.nodes {
        let from = names[&node.key].clone();
        for key in &node.needs {
            if let Some(to) = names.get(key) {
                out.push(Edge {
                    from: from.clone(),
                    to: to.clone(),
                    reason: Reason::DeclaredNeed,
                });
            }
        }
        if let Some(source) = node.source {
            if let Some(to) = names.get(&source) {
                out.push(Edge {
                    from,
                    to: to.clone(),
                    reason: Reason::Import,
                });
            }
        }
        if let Some(child) = &node.child {
            edges(child, names, out);
        }
    }
}

impl<Out, In> Plan<Out, In> {
    /// Inspect data provenance, including input and import bindings. Unlike
    /// [`inspect`](Self::inspect), this includes sources with no lifecycle.
    pub fn inspect_dataflow(&self) -> DataflowView {
        let mut view = DataflowView {
            nodes: Vec::new(),
            edges: Vec::new(),
        };
        let mut names = BTreeMap::new();
        paths(self.ir(), &NodePath::default(), &mut names, &mut view.nodes);
        edges(self.ir(), &names, &mut view.edges);
        view
    }
}

impl std::fmt::Display for DataflowView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "data bindings (inputs and imports are not cleanup obligations)"
        )?;
        for node in &self.nodes {
            writeln!(f, "{}: {}", node.path, node.kind.label())?;
        }
        for edge in &self.edges {
            writeln!(f, "{} <- {} ({})", edge.from, edge.to, edge.reason)?;
        }
        Ok(())
    }
}
