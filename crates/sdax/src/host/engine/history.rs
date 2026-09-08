//! Immutable inspection records, separate from the active execution graph.
use super::state::{Machine, NodeState, RunState};
use crate::contracts::Time;
use crate::cx::InstanceId;
use crate::key::RawKey;
use crate::plan::Kind;
use crate::view::NodePath;
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct History {
    pub nodes: BTreeMap<RawKey, HistoricalNode>,
    pub instances: BTreeMap<InstanceId, HistoricalInstance>,
}

pub(super) struct HistoricalInstance {
    pub parent: Option<InstanceId>,
    pub order: usize,
    pub nodes: Vec<RawKey>,
}

pub(super) struct HistoricalNode {
    pub decl: RawKey,
    pub instance: InstanceId,
    pub path: NodePath,
    pub kind: Kind,
    pub state: NodeState,
    pub deadline: Option<Time>,
    pub order: usize,
}

impl Machine {
    pub(super) fn ordered_nodes(&self) -> Vec<(usize, RawKey, NodePath, Kind)> {
        let mut nodes: Vec<_> = self
            .t
            .nodes
            .iter()
            .map(|n| (n.order, n.key, n.path.clone(), n.kind))
            .chain(
                self.history
                    .nodes
                    .iter()
                    .map(|(&key, n)| (n.order, key, n.path.clone(), n.kind)),
            )
            .collect();
        nodes.sort_unstable_by_key(|n| n.0);
        nodes
    }
}

impl Machine {
    // ------------------------------------------------------------ readers

    /// Every instance of this run and where its scope is.
    pub fn instances(&self) -> Vec<(InstanceId, RunState)> {
        let mut all: Vec<_> = self
            .instances
            .iter()
            .map(|i| (i.order, i.id, self.scopes[i.scope].st))
            .chain(
                self.history
                    .instances
                    .iter()
                    .map(|(&id, i)| (i.order, id, RunState::Ended)),
            )
            .collect();
        all.sort_unstable_by_key(|i| i.0);
        all.into_iter().map(|(_, id, state)| (id, state)).collect()
    }

    /// Borrow the states of instances that have not ended, without allocating
    /// a historical snapshot. Ended identities remain available through
    /// [`Self::instances`]; this iterator scans only execution instances awaiting retirement.
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
        if let Some(instance) = self.history.instances.get(&id) {
            return instance
                .nodes
                .iter()
                .map(|&key| {
                    let n = &self.history.nodes[&key];
                    (key, n.decl, n.path.clone(), n.kind)
                })
                .collect();
        }
        self.t
            .nodes
            .iter()
            .filter(|n| n.instance == Some(id))
            .map(|n| (n.key, n.decl, n.path.clone(), n.kind))
            .collect()
    }

    /// The declaration a run key was made from, and the instance it belongs
    /// to. What a [`BodySource`](crate::host::BodySource) is addressed by.
    pub fn origin(&self, key: RawKey) -> Option<(RawKey, Option<InstanceId>)> {
        self.t
            .index_of(key)
            .map(|i| (self.t.nodes[i].decl, self.t.nodes[i].instance))
            .or_else(|| {
                self.history
                    .nodes
                    .get(&key)
                    .map(|n| (n.decl, Some(n.instance)))
            })
    }

    /// The instance a spawning body itself belongs to, for a nested template.
    pub fn instance_parent(&self, id: InstanceId) -> Option<InstanceId> {
        self.instances
            .iter()
            .find(|i| i.id == id)
            .and_then(|i| i.parent)
            .or_else(|| self.history.instances.get(&id).and_then(|i| i.parent))
    }
}
