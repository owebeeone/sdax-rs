//! The flattened declaration the machine runs on.
//!
//! A run is one flat table of nodes over a tree of scopes: the root plan is
//! scope 0, and every component is a scope of its own whose nodes follow the
//! component node. Imports are resolved here, once, to the parent node they
//! stand for, so T1 and T6 never see an `Import` node — a child node that
//! imports `Endpoint` simply *needs* the parent's `Endpoint`, and `Endpoint`'s
//! dependents include that child node (contract § 1, `import`).
//!
//! **Instances grow the table.** A template's nodes exist once per instance,
//! so [`Table::instantiate`] appends a scope and its nodes at run time. Their
//! [`RawKey`]s are minted fresh (one plan identity per instance scope) so that
//! every node of a run has a unique key and an `Event` can name it; the
//! declaration key each was made from is kept in [`Node::decl`], which is what
//! a [`BodySource`](crate::host::BodySource) is addressed by.
//!
//! Everything the machine derives comes from this table and from nothing
//! else (INV-1).

use super::state::EngineError;
use crate::cx::InstanceId;
use crate::key::RawKey;
use crate::plan::{next_plan_id, Attrs, Kind, NodeDecl, PlanIr, PoolDecl};
use crate::policy::{Mode, Policy, Shutdown};
use crate::view::NodePath;
use std::sync::Arc;

/// One node of the run.
pub(super) struct Node {
    /// This node's identity within the run. Unique: an instance's nodes are
    /// re-keyed, so two instances of one template never share a key.
    pub key: RawKey,
    /// The key the declaration gave it, which addresses its body.
    pub decl: RawKey,
    /// The instance this node belongs to, if it is not part of the static
    /// graph.
    pub instance: Option<InstanceId>,
    pub path: NodePath,
    /// The report-order prefix: one `(declaration index, instance)` per level.
    pub steps: Vec<(u32, Option<InstanceId>)>,
    pub kind: Kind,
    /// The scope this node belongs to.
    pub scope: usize,
    /// For a component: the scope its inner nodes form.
    pub inner: Option<usize>,
    /// For a template: the plan an instance of it runs.
    pub child: Option<Arc<PlanIr>>,
    /// Templates this node declared it may instantiate, as declaration keys
    /// (F1). Resolved against the scope's map, so a template declared after
    /// the service that spawns it still resolves.
    pub spawns: Vec<RawKey>,
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

/// One scope of the run: the root plan, a component's inner plan, or one
/// instance of a template.
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
    /// The template node and instance identity this scope runs for.
    pub instance: Option<(usize, InstanceId)>,
    /// The node this scope exports, if it exports one. A component becomes
    /// `Ready` on its export, so an inner fault that kills the export kills the
    /// component whatever the child's fail policy.
    pub export: Option<usize>,
    /// The key lookup an instance of a template declared *here* resolves
    /// against. `None` on a static scope, which uses the table's own.
    pub map: Option<Vec<(RawKey, usize)>>,
}

/// The whole run, flattened.
pub(super) struct Table {
    pub nodes: Vec<Node>,
    pub scopes: Vec<Scope>,
    /// The static graph's key lookup, in flatten order.
    pub map: Vec<(RawKey, usize)>,
}

/// The state one flatten pass threads through the tree.
struct Pass<'a> {
    /// Key lookup, searched from the end.
    map: Vec<(RawKey, usize)>,
    /// Plans already flattened, so one child plan used twice is refused.
    seen: Vec<(u64, NodePath)>,
    /// `Some(id)` while flattening an instance: every scope made is that
    /// instance's, and every key is re-minted.
    instance: Option<InstanceId>,
    /// The nodes this pass appended.
    added: &'a mut Vec<usize>,
}

fn path_of(prefix: &NodePath, n: &NodeDecl) -> NodePath {
    if prefix.segments().is_empty() {
        NodePath::root(&n.name)
    } else {
        prefix.child(&n.name)
    }
}

fn lookup(map: &[(RawKey, usize)], k: RawKey) -> Option<usize> {
    map.iter().rev().find(|(key, _)| *key == k).map(|(_, i)| *i)
}

/// Whether the caller starting this run supplies the plan's per-run input.
///
/// The declaration alone cannot say: one plan value is both a root a caller
/// starts with `start(rt, input)` and a template `cx.spawn` instantiates. What
/// decides is the entry point, so the entry point is what tells the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RootInput {
    /// A value is in the run's slot for the input node before the first body
    /// is built, so a need on it is already satisfied.
    Supplied,
    /// Nobody supplies one: a plan that declares an input is refused, because
    /// the node that needs it could never be satisfied.
    Absent,
}

/// Check the entire declaration tree before admission, including templates
/// that may never be instantiated. Only a template registration supplies its
/// child's declared input; a component never does, even for an unused `()`.
fn check_component_inputs(ir: &PlanIr, prefix: &NodePath) -> Result<(), EngineError> {
    for node in &ir.nodes {
        let Some(child) = &node.child else { continue };
        let path = path_of(prefix, node);
        if node.kind == Kind::Component {
            if let Some(input) = child.nodes.iter().find(|n| n.kind == Kind::Input) {
                return Err(EngineError::TemplateAsScope(path.child(&input.name)));
            }
        }
        check_component_inputs(child, &path)?;
    }
    Ok(())
}

impl Table {
    /// Flatten a root plan. Refuses a plan whose declared input nothing
    /// supplies a value for ([`RootInput::Absent`]), a root with unresolved
    /// imports (`L-IMPORTS`), and one child plan used as two components: the
    /// engine addresses a *declaration* by [`RawKey`], which a plan used twice
    /// cannot keep unique. Also refuses declared component inputs throughout
    /// the declaration tree, including uninstantiated templates.
    pub fn build(ir: &PlanIr, input: RootInput) -> Result<Table, EngineError> {
        if input == RootInput::Absent {
            if let Some(n) = ir.nodes.iter().find(|n| n.kind == Kind::Input) {
                return Err(EngineError::TemplateAsScope(NodePath::root(&n.name)));
            }
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
        check_component_inputs(ir, &NodePath::default())?;
        let mut t = Table {
            nodes: Vec::new(),
            scopes: Vec::new(),
            map: Vec::new(),
        };
        let mut added = Vec::new();
        let mut pass = Pass {
            map: Vec::new(),
            seen: Vec::new(),
            instance: None,
            added: &mut added,
        };
        t.flatten(ir, None, None, NodePath::default(), Vec::new(), &mut pass)?;
        t.map = std::mem::take(&mut pass.map);
        t.link(&added);
        Ok(t)
    }

    /// Fill in `dependents` for the nodes just added.
    ///
    /// A template's own instances are **not** its dependents: the template's
    /// obligation *is* stopping them (contract § 1), so gating its start on
    /// their end would be a cycle. An instance node that needs the per-instance
    /// input therefore has no edge at all — the input exists from the spawn.
    fn link(&mut self, added: &[usize]) {
        for &i in added {
            for j in 0..self.nodes[i].needs.len() {
                let need = self.nodes[i].needs[j];
                if !self.nodes[need].dependents.contains(&i) {
                    self.nodes[need].dependents.push(i);
                }
            }
        }
    }

    /// The key lookup a template declared in `scope` resolves against: the
    /// nearest enclosing instance's, or the static graph's.
    pub fn map_of(&self, scope: usize) -> &[(RawKey, usize)] {
        let mut s = scope;
        loop {
            if let Some(m) = &self.scopes[s].map {
                return m;
            }
            match self.scopes[s].parent {
                Some(p) => s = p,
                None => return &self.map,
            }
        }
    }

    /// The flat index of a template a node named, resolved in its own scope.
    pub fn template_in(&self, scope: usize, decl: RawKey) -> Option<usize> {
        lookup(self.map_of(scope), decl).filter(|&i| self.nodes[i].kind == Kind::Template)
    }

    /// One instance of the template at `t`: a scope of its own, its nodes
    /// appended to the table with fresh keys, and the indices of both.
    ///
    /// Only the caller knows whether the instance may be created; this is the
    /// table's half of it.
    pub fn instantiate(&mut self, t: usize, id: InstanceId) -> Result<(usize, Vec<usize>), ()> {
        let Some(child) = self.nodes[t].child.clone() else {
            return Err(());
        };
        let parent = self.nodes[t].scope;
        let prefix = self.nodes[t].path.clone();
        let mut steps = self.nodes[t].steps.clone();
        if let Some(last) = steps.last_mut() {
            last.1 = Some(id);
        }
        let scope = self.scopes.len();
        let mut added = Vec::new();
        let mut pass = Pass {
            map: self.map_of(parent).to_vec(),
            seen: Vec::new(),
            instance: Some(id),
            added: &mut added,
        };
        let flattened = self.flatten(&child, Some(parent), None, prefix, steps, &mut pass);
        let map = std::mem::take(&mut pass.map);
        if flattened.is_err() {
            return Err(());
        }
        self.scopes[scope].instance = Some((t, id));
        self.scopes[scope].map = Some(map);
        self.link(&added);
        Ok((scope, added))
    }

    fn flatten(
        &mut self,
        ir: &PlanIr,
        parent: Option<usize>,
        component: Option<usize>,
        prefix: NodePath,
        steps: Vec<(u32, Option<InstanceId>)>,
        pass: &mut Pass<'_>,
    ) -> Result<(), EngineError> {
        if let Some((_, first)) = pass.seen.iter().find(|(id, _)| *id == ir.id) {
            return Err(EngineError::DuplicateComponent {
                plan: ir.name.clone(),
                first: first.clone(),
                second: prefix,
            });
        }
        pass.seen.push((ir.id, prefix.clone()));
        let scope = self.scopes.len();
        // An instance re-keys its nodes, so one plan identity per scope keeps
        // every key of the run unique without touching the declaration.
        let plan_id = match pass.instance {
            Some(_) => next_plan_id(),
            None => ir.id,
        };
        self.scopes.push(Scope {
            name: ir.name.clone(),
            policy: ir.policy,
            shutdown: ir.shutdown,
            mode: ir.mode,
            nodes: Vec::new(),
            pools: ir.pools.clone(),
            parent,
            component,
            instance: None,
            export: None,
            map: None,
        });
        // The record's step is the node's position among the nodes the plan
        // *shows* — `PlanView` skips imports and the per-instance input, and F4
        // order is the order an author can read off the view (`RecordOrder`,
        // `RawKey`). The builder's own `key.idx` counts them too, so it is not
        // it.
        let mut pos: u32 = 0;
        for n in &ir.nodes {
            if n.kind == Kind::Import {
                // The parent key this import stands for is already flat.
                if let Some(src) = n.source.and_then(|s| lookup(&pass.map, s)) {
                    pass.map.push((n.key, src));
                }
                continue;
            }
            if n.kind == Kind::Input {
                // The per-run input is not a node of the run: its value is in
                // the run's slots from the moment the run started — the spawn
                // for an instance, `start(rt, input)` for a root — so a need on
                // it is already satisfied and is dropped here.
                continue;
            }
            let idx = self.nodes.len();
            let mut node_steps = steps.clone();
            node_steps.push((pos, None));
            pos += 1;
            let resolve = |keys: &[RawKey]| -> Vec<usize> {
                keys.iter().filter_map(|k| lookup(&pass.map, *k)).collect()
            };
            let mut needs = resolve(&n.needs);
            needs.dedup();
            self.nodes.push(Node {
                key: RawKey {
                    plan: plan_id,
                    idx: n.key.idx,
                },
                decl: n.key,
                instance: pass.instance,
                path: path_of(&prefix, n),
                steps: node_steps.clone(),
                kind: n.kind,
                scope,
                inner: None,
                child: if n.kind == Kind::Template {
                    n.child.clone()
                } else {
                    None
                },
                spawns: n.spawns.clone(),
                needs,
                dependents: Vec::new(),
                exclusive: resolve(&n.attrs.exclusive),
                shared: resolve(&n.attrs.shared),
                pool: n.attrs.limit.or(n.attrs.pool).map(|p| p.index() as usize),
                attrs: n.attrs.clone(),
            });
            self.scopes[scope].nodes.push(idx);
            pass.added.push(idx);
            pass.map.push((n.key, idx));
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
                        pass,
                    )?;
                }
            }
        }
        // Resolved last: an export names a node of this scope, which is flat
        // by now (and, for a component, so is its whole subtree).
        self.scopes[scope].export = ir.export.and_then(|k| lookup(&pass.map, k));
        Ok(())
    }

    /// The flat index behind a run key.
    pub fn index_of(&self, key: RawKey) -> Option<usize> {
        self.nodes.iter().position(|n| n.key == key)
    }

    /// The flat index behind a path, in the static graph.
    ///
    /// Several live instances share one path, so this answers for the
    /// declaration a path names and never for an instance's copy of it.
    pub fn index_of_path(&self, path: &str) -> Option<usize> {
        self.nodes
            .iter()
            .position(|n| n.instance.is_none() && n.path == *path)
    }
}
