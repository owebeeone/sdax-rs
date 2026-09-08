//! Templates and dynamic instances: the spawn gate, the instance's own
//! lifecycle, and INV-16 containment.
//!
//! A **template** node has no body. It becomes `Live` when its imports are
//! ready (T1 like any other node) and stays live until its obligation opens —
//! "stop every live instance" (contract § 1). An **instance** is a scope of its
//! own, appended to the table by [`Table::instantiate`], admitted like a
//! component's inner scope and cleaned up as one unit.
//!
//! **INV-16 has three clauses and each is enforced somewhere different.** That
//! an instance's nodes can name only their own, their input and imported
//! parent keys is `V-IMPORT-SCOPE` at `build`. That a parent node can never
//! name an instance node holds by construction: an instance's nodes exist
//! only after the spawn, and nothing outside them is ever added to their
//! `dependents`. That every live instance ends before any key it imports is
//! released is T6, and it is this module's: an instance's importing nodes are
//! `dependents` of the parent node they import, so `gate_open` already waits
//! for them, and the template node itself owes while any instance is live.

use super::state::{Cause, Instance, Machine, St};
use super::table::Table;
use super::{Effect, RunState};
use crate::cx::{InstanceId, SpawnError};
use crate::key::RawKey;
use crate::plan::Kind;
use crate::report::{Outcome, TraceKind};
use crate::view::NodePath;

impl Machine {
    // ------------------------------------------------------------ the gate

    /// Whether `spawner` may instantiate `template` **now** (contract § 5).
    ///
    /// `spawner` is a node of this run; `template` is the declaration key a
    /// [`Template`](crate::Template) handle carries, which is resolved inside
    /// the spawner's own scope so that a template declared in a template's plan
    /// resolves to *that* instance's copy.
    ///
    /// The three refusals are ordered so each says the most specific true
    /// thing: a handle from another plan is `ForeignTemplate` whoever presents
    /// it, a template of this run that this node did not declare is
    /// `UndeclaredTemplate`, and only a node that could otherwise spawn is told
    /// `ScopeStopping`.
    pub fn spawn_check(&self, spawner: RawKey, template: RawKey) -> Result<(), SpawnError> {
        let template = self
            .t
            .index_of(spawner)
            .and_then(|s| self.t.template_in(self.t.nodes[s].scope, template))
            .map(|t| self.t.nodes[t].decl)
            .unwrap_or(template);
        let known = self
            .t
            .nodes
            .iter()
            .any(|n| n.kind == Kind::Template && n.decl == template);
        if !known {
            return Err(SpawnError::ForeignTemplate);
        }
        let Some(s) = self.t.index_of(spawner) else {
            return Err(SpawnError::UndeclaredTemplate);
        };
        if !self.t.nodes[s].spawns.contains(&template) {
            return Err(SpawnError::UndeclaredTemplate);
        }
        let scope = self.t.nodes[s].scope;
        let t = self.t.template_in(scope, template);
        match t {
            // A template whose own imports are not ready yet still admits: the
            // instance's importing node waits for them under T1 like any
            // other node, and refusing here would answer `ScopeStopping` for a
            // scope that is admitting perfectly well.
            Some(t)
                if self.admitting(scope)
                    && matches!(self.slots[t].st, St::Pending | St::Waiting | St::Live) =>
            {
                Ok(())
            }
            Some(_) => Err(SpawnError::ScopeStopping),
            None => Err(SpawnError::UndeclaredTemplate),
        }
    }

    /// Every `(spawner, template declaration)` pair of this run, and whether it
    /// is open right now — the gate as a value, for a driver whose bodies ask
    /// from another task and cannot reach the machine (contract § 5).
    pub fn spawn_table(&self) -> SpawnTable {
        let mut pairs = Vec::new();
        for n in &self.t.nodes {
            for &tpl in &n.spawns {
                let open = self.spawn_check(n.key, tpl).is_ok();
                pairs.push((n.key, tpl, open));
                let scope = &self.t.scopes[n.scope];
                let original = RawKey {
                    plan: scope.origin,
                    idx: tpl.idx,
                };
                if original != tpl {
                    pairs.push((n.key, original, open));
                }
            }
        }
        SpawnTable {
            templates: self
                .t
                .nodes
                .iter()
                .filter(|n| n.kind == Kind::Template)
                .flat_map(|n| {
                    [
                        n.decl,
                        RawKey {
                            plan: self.t.scopes[n.scope].origin,
                            idx: n.decl.idx,
                        },
                    ]
                })
                .collect(),
            pairs,
        }
    }

    // ------------------------------------------------------------ readers

    /// Every instance of this run and where its scope is.
    pub fn instances(&self) -> Vec<(InstanceId, RunState)> {
        self.instances
            .iter()
            .map(|i| (i.id, self.scopes[i.scope].st))
            .collect()
    }

    /// Borrow the states of instances that have not ended, without allocating
    /// a historical snapshot. Ended identities remain available through
    /// [`Self::instances`]; this iterator still scans the retained history.
    pub fn active_instance_states(&self) -> impl Iterator<Item = (InstanceId, RunState)> + '_ {
        self.instances
            .iter()
            .filter(|instance| !instance.ended)
            .map(|instance| (instance.id, self.scopes[instance.scope].st))
    }

    /// All nodes of one instance, including component descendants, in table
    /// order: run key, declaration key, path and kind. Separately spawned
    /// nested instances belong to their own identity and are excluded.
    pub fn instance_nodes(&self, id: InstanceId) -> Vec<(RawKey, RawKey, NodePath, Kind)> {
        self.t
            .nodes
            .iter()
            .filter(|node| node.instance == Some(id))
            .map(|node| (node.key, node.decl, node.path.clone(), node.kind))
            .collect()
    }

    /// The declaration a run key was made from, and the instance it belongs
    /// to. What a [`BodySource`](crate::host::BodySource) is addressed by.
    pub fn origin(&self, key: RawKey) -> Option<(RawKey, Option<InstanceId>)> {
        self.t
            .index_of(key)
            .map(|i| (self.t.nodes[i].decl, self.t.nodes[i].instance))
    }

    /// The instance a spawning body itself belongs to, for a nested template.
    pub fn instance_parent(&self, id: InstanceId) -> Option<InstanceId> {
        self.instances
            .iter()
            .find(|i| i.id == id)
            .and_then(|i| i.parent)
    }

    // ------------------------------------------------------------ spawning

    /// A body instantiated a template (T1 for a whole scope at once).
    pub(super) fn on_instance_spawned(
        &mut self,
        spawner: RawKey,
        template: RawKey,
        id: InstanceId,
    ) -> Result<(), &'static str> {
        if self.instances.iter().any(|i| i.id == id) {
            return Err("InstanceSpawned reused an instance id");
        }
        let Some(sp) = self.t.index_of(spawner) else {
            return Err("InstanceSpawned for an unknown spawner");
        };
        let template = self
            .t
            .template_in(self.t.nodes[sp].scope, template)
            .map(|t| self.t.nodes[t].decl)
            .unwrap_or(template);
        if !self.t.nodes[sp].spawns.contains(&template) {
            return Err("InstanceSpawned for a template this node did not declare");
        }
        let scope = self.t.nodes[sp].scope;
        let Some(t) = self.t.template_in(scope, template) else {
            return Err("InstanceSpawned for a template this scope does not declare");
        };
        // The gate was checked by the host before the `Child` was handed out,
        // but a settle can land between that check and this event. The
        // instance is still created — the host holds a `Child` for it — and is
        // ended at once, so `Child::ready()` answers rather than hanging.
        let admitting = self.admitting(scope)
            && matches!(self.slots[t].st, St::Pending | St::Waiting | St::Live);
        let Ok((inst_scope, added)) = std::sync::Arc::make_mut(&mut self.t).instantiate(t, id)
        else {
            return Err("InstanceSpawned for a node that is not a template");
        };
        self.grow_to(self.t.nodes.len(), self.t.scopes.len());
        let parent = self.t.nodes[sp].instance;
        self.instances.push(Instance {
            id,
            template: t,
            scope: inst_scope,
            parent,
            ended: false,
        });
        self.fx.push(Effect::SpawnInstance {
            template: self.t.nodes[t].decl,
            parent,
            id,
        });
        self.emit(t, TraceKind::InstanceSpawned(id));
        let _ = added;
        if admitting {
            self.scopes[inst_scope].st = RunState::Admitting;
            self.admit(inst_scope);
            self.check_steady(inst_scope);
            self.try_cleanup(inst_scope);
        } else {
            // The scope settled between the body's gate check and this event.
            // The instance's scope is still `Planned`, which `settle` leaves
            // alone, so it is *skipped* — its nodes with it — and ended at
            // once: the `Child` the body is holding then answers, and nothing
            // holds the run open for a scope that never admitted.
            self.scopes[inst_scope].cause = Some(Cause::Parent(None));
            self.skip_scope(inst_scope, None);
            self.end_instance(inst_scope);
            self.sweep();
        }
        Ok(())
    }

    /// `Child::stop()`: this instance settles, and its own release graph runs.
    pub(super) fn on_stop_instance(&mut self, id: InstanceId) -> Result<(), &'static str> {
        let Some(inst) = self.instances.iter().find(|i| i.id == id) else {
            return Err("StopInstance for an unknown instance");
        };
        let scope = inst.scope;
        match self.scopes[scope].st {
            RunState::Ended => Ok(()),
            _ => {
                self.settle(scope, Cause::Shutdown);
                self.sweep();
                Ok(())
            }
        }
    }

    /// Slots, locks and scope bookkeeping for nodes and scopes the table just
    /// grew by.
    fn grow_to(&mut self, nodes: usize, scopes: usize) {
        while self.slots.len() < nodes {
            let n = self.slots.len();
            self.slots.push(super::state::Slot::new());
            self.locks.push(Default::default());
            let rank = self
                .schedule
                .iter()
                .position(|name| self.t.nodes[n].path == *name.as_str())
                .map(|r| r as u32)
                .unwrap_or(u32::MAX);
            self.rank.push(rank);
        }
        while self.scopes.len() < scopes {
            let s = self.scopes.len();
            self.scopes.push(super::state::ScopeRun {
                st: RunState::Planned,
                cause: None,
                deadline: None,
                budget_timer: None,
                zero_timer: None,
                spent: false,
                pools: vec![0; self.t.scopes[s].pools.len()],
                faulted: false,
            });
        }
    }

    // ------------------------------------------------------------ lifecycle

    /// Whether this template ever admitted an instance.
    pub(super) fn ever_spawned(&self, n: usize) -> bool {
        self.instances.iter().any(|i| i.template == n)
    }

    /// A template that has admitted an instance is `Live`, whatever the queue
    /// thought: its obligation is to stop them, and a `Skipped` node runs no
    /// obligation. Answers whether the node must not be skipped.
    pub(super) fn keep_template_live(&mut self, n: usize) -> bool {
        if self.t.nodes[n].kind != Kind::Template || !self.ever_spawned(n) {
            return false;
        }
        if matches!(self.slots[n].st, St::Pending | St::Waiting) {
            self.slots[n].st = St::Live;
            self.slots[n].queued = None;
        }
        true
    }

    /// The live instances of a template node.
    pub(super) fn live_instances(&self, t: usize) -> Vec<usize> {
        self.instances
            .iter()
            .enumerate()
            .filter(|(_, i)| i.template == t && !i.ended)
            .map(|(x, _)| x)
            .collect()
    }

    /// Every live instance of `t` stops admitting (T5, and the template's own
    /// obligation).
    pub(super) fn settle_instances(&mut self, t: usize, because: Option<usize>) {
        for x in self.live_instances(t) {
            let scope = self.instances[x].scope;
            match self.scopes[scope].st {
                RunState::Planned => self.skip_scope(scope, because),
                RunState::Admitting | RunState::Steady => {
                    self.settle(scope, Cause::Parent(because))
                }
                _ => {}
            }
        }
    }

    /// In-flight bodies inside the live instances of a template node (P-17: no
    /// release before the joins).
    pub(super) fn instances_in_flight(&self, t: usize) -> usize {
        self.live_instances(t)
            .into_iter()
            .map(|x| {
                let scope = self.instances[x].scope;
                if matches!(
                    self.scopes[scope].st,
                    RunState::Admitting | RunState::Steady
                ) {
                    1
                } else {
                    self.in_flight(scope)
                }
            })
            .sum()
    }

    /// T5 for a template: its obligation is to stop every live instance.
    pub(super) fn start_template_stop(&mut self, t: usize) {
        self.slots[t].st = St::StoppingInstances;
        self.emit(t, TraceKind::StopRequested);
        self.settle_instances(t, None);
        self.sweep();
        self.finish_template(t);
    }

    /// The template's obligation ends when its last instance has.
    pub(super) fn finish_template(&mut self, t: usize) {
        if self.slots[t].st == St::StoppingInstances && self.live_instances(t).is_empty() {
            self.slots[t].st = St::Stopped;
            self.emit(t, TraceKind::Stopped);
            self.after_cleanup(t);
        }
    }

    /// An instance's scope ended: record it, tell the trace, and re-open the
    /// gates it was holding shut (T6, INV-16).
    pub(super) fn end_instance(&mut self, scope: usize) {
        let Some((t, id)) = self.t.scopes[scope].instance else {
            return;
        };
        let Some(x) = self.instances.iter().position(|i| i.id == id) else {
            return;
        };
        if self.instances[x].ended {
            return;
        }
        self.instances[x].ended = true;
        let outcome = self.instance_outcome(scope);
        self.emit(t, TraceKind::InstanceEnded(id, outcome));
        self.finish_template(t);
        self.after_cleanup(t);
    }

    /// How one instance ended. The run's fault list is the whole run's, so an
    /// instance's own outcome is read from its scope: a fault of its own is
    /// `Failed`, an external `cancel()` that reached it is `Cancelled`, and a
    /// stop — a request, its template's obligation, a `terminal` service
    /// inside it finishing (`C-64`) — is a normal end.
    fn instance_outcome(&self, scope: usize) -> Outcome {
        if self.scopes[scope].faulted {
            return Outcome::Failed;
        }
        if self.cause_is_cancel(scope) {
            return Outcome::Cancelled;
        }
        Outcome::Ok
    }

    fn cause_is_cancel(&self, scope: usize) -> bool {
        let mut s = scope;
        loop {
            match self.scopes[s].cause {
                Some(Cause::Cancel) => return true,
                Some(Cause::Parent(_)) => match self.t.scopes[s].parent {
                    Some(p) => s = p,
                    None => return false,
                },
                _ => return false,
            }
        }
    }
}

/// The spawn gate as a value: what `cx.spawn` may do, as a driver publishes it
/// for the body tasks that cannot reach the machine.
///
/// Recomputed by [`Machine::spawn_table`] whenever the machine has stepped, so
/// a body reads a snapshot and never the machine itself. The decision
/// procedure is the machine's — [`SpawnTable::check`] answers exactly what
/// [`Machine::spawn_check`] answered when the snapshot was taken.
#[derive(Debug, Clone, Default)]
pub struct SpawnTable {
    templates: Vec<RawKey>,
    pairs: Vec<(RawKey, RawKey, bool)>,
}

impl SpawnTable {
    /// Whether `spawner` may instantiate `template`, as of this snapshot.
    pub fn check(&self, spawner: RawKey, template: RawKey) -> Result<(), SpawnError> {
        if !self.templates.contains(&template) {
            return Err(SpawnError::ForeignTemplate);
        }
        match self
            .pairs
            .iter()
            .find(|(s, t, _)| *s == spawner && *t == template)
        {
            Some((_, _, true)) => Ok(()),
            Some(_) => Err(SpawnError::ScopeStopping),
            None => Err(SpawnError::UndeclaredTemplate),
        }
    }
}

impl Table {
    /// Every node the declaration tree holds, template plans included, as
    /// `(declaration key, path, kind)`.
    ///
    /// A harness that supplies bodies needs the template's inner nodes before
    /// any instance exists; the run table has them only per instance.
    pub(super) fn declarations(ir: &crate::plan::PlanIr) -> Vec<(RawKey, NodePath, Kind)> {
        let mut out = Vec::new();
        walk(ir, &NodePath::default(), &mut out);
        out
    }
}

fn walk(ir: &crate::plan::PlanIr, prefix: &NodePath, out: &mut Vec<(RawKey, NodePath, Kind)>) {
    for n in &ir.nodes {
        if matches!(n.kind, Kind::Import | Kind::Input) {
            continue;
        }
        let path = if prefix.segments().is_empty() {
            NodePath::root(&n.name)
        } else {
            prefix.child(&n.name)
        };
        out.push((n.key, path.clone(), n.kind));
        if let Some(child) = &n.child {
            walk(child, &path, out);
        }
    }
}
