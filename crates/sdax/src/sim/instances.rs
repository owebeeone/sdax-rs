//! The simulator's half of `cx.spawn`: a scripted body's spawn directives,
//! the readiness an initializer may await (INV-17), and the stop a serve future
//! may ask for.
//!
//! The simulator has no bodies, so a [`SpawnSpec`](super::script::SpawnSpec)
//! stands in for what one would do. The **decision** is not the simulator's:
//! it asks [`Machine::spawn_check`](crate::host::engine::Machine::spawn_check),
//! which is the same procedure a run driver's `Scope::spawn_instance` uses, so
//! a `SpawnError` means the same thing on both.

use super::simulator::{Item, Simulator};
use crate::contracts::Time;
use crate::cx::{InstanceId, SpawnError};
use crate::host::engine::{Event, RunState};
use crate::key::RawKey;

/// The key a template handle from **another** plan presents. No plan can mint
/// it, so the gate answers `ForeignTemplate` for it and nothing else.
pub const FOREIGN: RawKey = RawKey {
    plan: u64::MAX,
    idx: u32::MAX,
};

/// One scripted `cx.spawn` and what it answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnOutcome {
    /// The node whose body asked.
    pub node: String,
    /// The template it named.
    pub template: String,
    /// The instance it got, or why it was refused.
    pub result: Result<InstanceId, SpawnError>,
}

/// What one node's body is waiting for before it may return.
pub(super) struct Awaiting {
    /// The body's own ending event, held back until the wait is over.
    pub end: Event,
    /// The instances `Child::ready()` was called on.
    pub ids: Vec<InstanceId>,
}

impl Simulator {
    fn index_of(&self, key: RawKey) -> Option<usize> {
        self.nodes.iter().position(|n| n.key == key)
    }

    /// Queue the spawn directives of one attempt of `node`'s body.
    ///
    /// A directive whose time falls after the body's own ending is dropped: a
    /// body that has returned cannot call `cx.spawn`, and the scripted body a
    /// run driver executes drops it the same way.
    pub(super) fn arm_spawns(&mut self, node: RawKey, start: Time, end: Option<Time>) {
        let Some(i) = self.index_of(node) else { return };
        self.nodes[i].children.clear();
        self.nodes[i].awaited.clear();
        let ats: Vec<_> = self.nodes[i].spawns.iter().map(|s| s.at).collect();
        for (which, at) in ats.into_iter().enumerate() {
            let at = self.resolve(at, start);
            if end.is_some_and(|e| at > e) {
                continue;
            }
            self.push(at, Item::Spawn(node, which));
        }
    }

    /// Perform one scripted `cx.spawn`.
    ///
    /// The gate is the machine's, so a refusal here is the same refusal a body
    /// on a real runtime would be handed.
    pub(super) fn do_spawn(&mut self, node: RawKey, which: usize) -> Option<Event> {
        let i = self.index_of(node)?;
        let spec = self.nodes[i].spawns.get(which)?.clone();
        let path = self.nodes[i].path.clone();
        // A path this plan does not declare as a template stands for a handle
        // from another plan: the key is synthetic and the gate answers
        // `ForeignTemplate` for it, exactly as it would for a real one (C-51).
        let template = self
            .templates
            .iter()
            .find(|(p, _)| *p == spec.template)
            .map(|(_, k)| *k)
            .unwrap_or(FOREIGN);
        let result = match self.machine.spawn_check(node, template) {
            Ok(()) => {
                self.next_id += 1;
                Ok(InstanceId(self.next_id))
            }
            Err(e) => Err(e),
        };
        self.spawns.push(SpawnOutcome {
            node: path,
            template: spec.template.clone(),
            result,
        });
        let id = result.ok()?;
        self.nodes[i].children.push(id);
        if spec.await_ready {
            self.nodes[i].awaited.push(id);
        }
        Some(Event::InstanceSpawned {
            spawner: node,
            template,
            id,
        })
    }

    /// A body reached its ending while it still awaits an instance: hold the
    /// ending back until every awaited instance is ready or has ended.
    pub(super) fn hold_for_ready(&mut self, node: RawKey, end: &Event) -> bool {
        let Some(i) = self.index_of(node) else {
            return false;
        };
        let ids = self.nodes[i].awaited.clone();
        if ids.is_empty() || self.all_resolved(&ids) {
            return false;
        }
        let Event::NodeOk(k) = end else { return false };
        self.parked.push((
            node,
            Awaiting {
                end: Event::NodeOk(*k),
                ids,
            },
        ));
        true
    }

    /// After a step: release every body whose awaited instances have resolved.
    pub(super) fn release_awaits(&mut self) {
        let mut freed = Vec::new();
        let mut i = 0;
        while i < self.parked.len() {
            if self.all_resolved(&self.parked[i].1.ids) {
                let (node, a) = self.parked.remove(i);
                freed.push((node, a.end));
            } else {
                i += 1;
            }
        }
        let now = self.now;
        for (node, end) in freed {
            self.push(now, Item::Body(node, end));
        }
    }

    /// Whether every instance in `ids` has reached steady state or ended, so a
    /// `Child::ready()` on it has answered one way or the other.
    fn all_resolved(&self, ids: &[InstanceId]) -> bool {
        let live = self.machine.instances();
        ids.iter().all(|id| {
            live.iter()
                .find(|(i, _)| i == id)
                .map(|(_, st)| matches!(st, RunState::Steady | RunState::Ended))
                .unwrap_or(true)
        })
    }

    /// A serving episode began: queue the stops this node's directives asked
    /// for, against the instances its initializer actually created.
    pub(super) fn arm_stops(&mut self, node: RawKey, start: Time) {
        let Some(i) = self.index_of(node) else { return };
        let pairs: Vec<(InstanceId, crate::sim::At)> = self.nodes[i]
            .spawns
            .iter()
            .enumerate()
            .filter_map(|(w, s)| Some((*self.nodes[i].children.get(w)?, s.stop?)))
            .collect();
        for (id, at) in pairs {
            let at = self.resolve(at, start);
            self.push(at, Item::Event(Event::StopInstance(id)));
        }
    }

    /// A new instance's nodes get scripts of their own, by the path their
    /// declaration has in the view (`Link/Sock`), so one script line covers
    /// every instance of a template.
    pub(super) fn open_instance(&mut self, id: InstanceId) {
        for (key, _decl, path, kind) in self.machine.instance_nodes(id) {
            let path = path.to_string();
            let ns = self.node_script(key, kind, path);
            self.nodes.push(ns);
        }
    }
}
