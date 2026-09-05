//! The flattened declaration the machine runs on.
//!
//! A run is one flat table of nodes over a tree of scopes: the root plan is
//! scope 0, and every component is a scope of its own whose nodes follow the
//! component node. Imports are resolved here, once, to the parent node they
//! stand for, so T1 and T6 never see an `Import` node — a child node that
//! imports `Endpoint` simply *needs* the parent's `Endpoint`, and `Endpoint`'s
//! dependents include that child node (contract § 1, `import`).
//!
//! Everything the machine derives comes from this table and from nothing
//! else (INV-1).

use super::state::EngineError;
use crate::cx::InstanceId;
use crate::key::RawKey;
use crate::plan::{Attrs, Kind, NodeDecl, PlanIr, PoolDecl};
use crate::policy::{Mode, Policy, Shutdown};
use crate::view::NodePath;

/// One node of the run.
pub(super) struct Node {
    pub key: RawKey,
    pub path: NodePath,
    /// The report-order prefix: one `(declaration index, instance)` per level.
    pub steps: Vec<(u32, Option<InstanceId>)>,
    pub kind: Kind,
    /// The scope this node belongs to.
    pub scope: usize,
    /// For a component: the scope its inner nodes form.
    pub inner: Option<usize>,
    /// Flat indices of everything this node needs, imports resolved.
    pub needs: Vec<usize>,
    /// Flat indices of everything that needs or imports this node.
    pub dependents: Vec<usize>,
    /// Flat indices of the resources this node locks exclusively.
    pub exclusive: Vec<usize>,
    /// Flat indices of the resources this node locks shared.
    pub shared: Vec<usize>,
    /// The pool (index within the scope) this node must be granted.
    pub pool: Option<usize>,
    pub attrs: Attrs,
}

/// One scope of the run: the root plan or a component's inner plan.
pub(super) struct Scope {
    pub name: String,
    pub policy: Policy,
    pub shutdown: Shutdown,
    pub mode: Mode,
    /// This scope's own nodes, in declaration order.
    pub nodes: Vec<usize>,
    pub pools: Vec<PoolDecl>,
    pub parent: Option<usize>,
    /// The component node (in the parent scope) this scope runs for.
    pub component: Option<usize>,
    /// The node this scope exports, if it exports one. A component becomes
    /// `Ready` on its export, so an inner fault that kills the export kills the
    /// component whatever the child's fail policy.
    pub export: Option<usize>,
}

/// The whole run, flattened.
pub(super) struct Table {
    pub nodes: Vec<Node>,
    pub scopes: Vec<Scope>,
}

/// Every template node anywhere in the tree, by path.
fn templates(ir: &PlanIr, prefix: &NodePath, out: &mut Vec<NodePath>) {
    for n in &ir.nodes {
        let path = path_of(prefix, n);
        match n.kind {
            Kind::Template | Kind::Input => out.push(path),
            Kind::Component => {
                if let Some(child) = &n.child {
                    templates(child, &path, out);
                }
            }
            _ => {}
        }
    }
}

fn path_of(prefix: &NodePath, n: &NodeDecl) -> NodePath {
    if prefix.segments().is_empty() {
        NodePath::root(&n.name)
    } else {
        prefix.child(&n.name)
    }
}

impl Table {
    /// Flatten a root plan. Refuses templates (Stage 3), a root with
    /// unresolved imports (`L-IMPORTS`), and one child plan used as two
    /// components: the engine addresses nodes by [`RawKey`], which a plan
    /// used twice cannot keep unique.
    pub fn build(ir: &PlanIr) -> Result<Table, EngineError> {
        let mut tpl = Vec::new();
        templates(ir, &NodePath::default(), &mut tpl);
        if !tpl.is_empty() {
            return Err(EngineError::Templates(tpl));
        }
        let imports: Vec<NodePath> = ir
            .nodes
            .iter()
            .filter(|n| n.kind == Kind::Import)
            .map(|n| NodePath::root(&n.name))
            .collect();
        if !imports.is_empty() {
            return Err(EngineError::UnresolvedImports(imports));
        }
        let mut t = Table {
            nodes: Vec::new(),
            scopes: Vec::new(),
        };
        let mut seen: Vec<(u64, NodePath)> = Vec::new();
        let mut map: Vec<(RawKey, usize)> = Vec::new();
        t.flatten(
            ir,
            None,
            None,
            NodePath::default(),
            Vec::new(),
            &mut seen,
            &mut map,
        )?;
        for i in 0..t.nodes.len() {
            for j in 0..t.nodes[i].needs.len() {
                let need = t.nodes[i].needs[j];
                if !t.nodes[need].dependents.contains(&i) {
                    t.nodes[need].dependents.push(i);
                }
            }
        }
        Ok(t)
    }

    #[allow(clippy::too_many_arguments)]
    fn flatten(
        &mut self,
        ir: &PlanIr,
        parent: Option<usize>,
        component: Option<usize>,
        prefix: NodePath,
        steps: Vec<(u32, Option<InstanceId>)>,
        seen: &mut Vec<(u64, NodePath)>,
        map: &mut Vec<(RawKey, usize)>,
    ) -> Result<(), EngineError> {
        if let Some((_, first)) = seen.iter().find(|(id, _)| *id == ir.id) {
            return Err(EngineError::DuplicateComponent {
                plan: ir.name.clone(),
                first: first.clone(),
                second: prefix,
            });
        }
        seen.push((ir.id, prefix.clone()));
        let scope = self.scopes.len();
        self.scopes.push(Scope {
            name: ir.name.clone(),
            policy: ir.policy,
            shutdown: ir.shutdown,
            mode: ir.mode,
            nodes: Vec::new(),
            pools: ir.pools.clone(),
            parent,
            component,
            export: None,
        });
        let lookup = |map: &Vec<(RawKey, usize)>, k: RawKey| -> Option<usize> {
            map.iter().rev().find(|(key, _)| *key == k).map(|(_, i)| *i)
        };
        // The record's step is the node's position among the nodes the plan
        // *shows* — `PlanView` skips imports, and F4 order is the order an
        // author can read off the view (`RecordOrder`, `RawKey`). The
        // builder's own `key.idx` counts imports too, so it is not it.
        let mut pos: u32 = 0;
        for n in &ir.nodes {
            if n.kind == Kind::Import {
                // The parent key this import stands for is already flat.
                if let Some(src) = n.source.and_then(|s| lookup(map, s)) {
                    map.push((n.key, src));
                }
                continue;
            }
            let idx = self.nodes.len();
            let mut node_steps = steps.clone();
            node_steps.push((pos, None));
            pos += 1;
            let resolve = |keys: &[RawKey]| -> Vec<usize> {
                keys.iter().filter_map(|k| lookup(map, *k)).collect()
            };
            let mut needs = resolve(&n.needs);
            needs.dedup();
            self.nodes.push(Node {
                key: n.key,
                path: path_of(&prefix, n),
                steps: node_steps.clone(),
                kind: n.kind,
                scope,
                inner: None,
                needs,
                dependents: Vec::new(),
                exclusive: resolve(&n.attrs.exclusive),
                shared: resolve(&n.attrs.shared),
                pool: n.attrs.limit.or(n.attrs.pool).map(|p| p.index() as usize),
                attrs: n.attrs.clone(),
            });
            self.scopes[scope].nodes.push(idx);
            map.push((n.key, idx));
            if n.kind == Kind::Component {
                if let Some(child) = &n.child {
                    let inner = self.scopes.len();
                    self.nodes[idx].inner = Some(inner);
                    self.flatten(
                        child,
                        Some(scope),
                        Some(idx),
                        path_of(&prefix, n),
                        node_steps,
                        seen,
                        map,
                    )?;
                }
            }
        }
        // Resolved last: an export names a node of this scope, which is flat
        // by now (and, for a component, so is its whole subtree).
        self.scopes[scope].export = ir.export.and_then(|k| lookup(map, k));
        Ok(())
    }

    /// The flat index behind a key.
    pub fn index_of(&self, key: RawKey) -> Option<usize> {
        self.nodes.iter().position(|n| n.key == key)
    }

    /// The flat index behind a path.
    pub fn index_of_path(&self, path: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.path == *path)
    }
}
