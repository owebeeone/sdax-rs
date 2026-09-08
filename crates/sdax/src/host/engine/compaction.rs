//! Reclaim ended execution rows only at a host effects-batch boundary.
use super::history::{HistoricalInstance, HistoricalNode};
use super::state::{Machine, Purpose};
use std::collections::BTreeSet;

impl Machine {
    /// Number of nodes still carrying execution topology and mutable state.
    /// Unlike `nodes()`, this excludes history archived by compaction. Static
    /// nodes remain for the lifetime of the run, even after they finish.
    pub fn execution_node_count(&self) -> usize {
        self.t.nodes.len()
    }

    /// Reclaim execution rows belonging to ended dynamic instances.
    ///
    /// The host must first consume the complete effects batch and all structural
    /// publication acknowledgements, and retire ended instance contexts and body
    /// slots. Calling this in the middle of a batch violates that host protocol.
    /// Active raw keys, historical node/instance readers, and report records are
    /// preserved. Late events cannot address a replacement row; ended instance
    /// IDs cannot be reused, and stopping one remains idempotent while the root
    /// is running. Historical inspection and trace storage still grow with churn.
    ///
    /// Reclamation is batched until retired nodes or instances reach half of
    /// their respective dynamic execution entries. Thus retained execution is
    /// bounded by twice the live dynamic entries (plus the static graph), and
    /// all dynamic execution rows are reclaimed when no instances remain live.
    /// No work is done unless an instance has ended since the previous pass.
    pub fn compact_ended_instances(&mut self) {
        if !self.compaction_pending {
            return;
        }
        let dynamic_nodes = self.t.nodes.len() - self.static_nodes;
        if self.retired_nodes < dynamic_nodes - self.retired_nodes
            && self.retired_instances < self.instances.len() - self.retired_instances
        {
            return;
        }
        self.retired_nodes = 0;
        self.retired_instances = 0;
        assert!(self.fx.is_empty(), "consume effects before compaction");
        self.compaction_pending = false;
        let ended: BTreeSet<_> = self
            .instances
            .iter()
            .filter(|i| i.ended)
            .map(|i| i.id)
            .collect();
        let nodes = mapping(
            self.t
                .nodes
                .iter()
                .map(|n| !n.instance.map(|id| ended.contains(&id)).unwrap_or(false)),
        );
        let mut keep_scopes = Vec::with_capacity(self.t.scopes.len());
        for scope in &self.t.scopes {
            let keep = scope
                .instance
                .map(|(_, id)| !ended.contains(&id))
                .unwrap_or_else(|| scope.parent.map(|p| keep_scopes[p]).unwrap_or(true));
            keep_scopes.push(keep);
        }
        let scopes = mapping(keep_scopes);
        // Resolve every public state before moving any path: a skipped node
        // may name another retiring node as its cause in this same batch.
        let terminal: Vec<_> = self
            .t
            .nodes
            .iter()
            .enumerate()
            .filter(|(i, _)| nodes[*i].is_none())
            .map(|(i, node)| {
                (
                    i,
                    self.state(node.key).expect("known node"),
                    self.deadline_for(node.key),
                )
            })
            .collect();
        let table = std::sync::Arc::make_mut(&mut self.t);
        for (i, state, deadline) in terminal {
            let node = &mut table.nodes[i];
            self.history.nodes.insert(
                node.key,
                HistoricalNode {
                    decl: node.decl,
                    instance: node.instance.expect("only dynamic nodes retire"),
                    path: std::mem::take(&mut node.path),
                    kind: node.kind,
                    state,
                    deadline,
                    order: node.order,
                },
            );
        }
        self.instances.retain_mut(|i| {
            if i.ended {
                self.history.instances.insert(
                    i.id,
                    HistoricalInstance {
                        parent: i.parent,
                        order: i.order,
                        nodes: self
                            .t
                            .nodes
                            .iter()
                            .filter(|n| n.instance == Some(i.id))
                            .map(|n| n.key)
                            .collect(),
                    },
                );
                false
            } else {
                i.template = nodes[i.template].expect("live instance retains its template");
                i.scope = scopes[i.scope].expect("live instance retains its scope");
                true
            }
        });
        self.instances.shrink_to_fit();
        for slot in &mut self.slots {
            slot.blocked_by = slot.blocked_by.and_then(|i| nodes[i]);
        }
        for lock in &mut self.locks {
            lock.exclusive = lock.exclusive.and_then(|i| nodes[i]);
            lock.shared.retain_mut(|i| match nodes[*i] {
                Some(new) => {
                    *i = new;
                    true
                }
                None => false,
            });
        }
        for lock in &mut self.locks {
            if lock.shared.capacity() > lock.shared.len().saturating_mul(2) {
                lock.shared.shrink_to_fit();
            }
        }
        self.timers.retain_mut(|(_, purpose, _)| {
            let (index, map) = match purpose {
                Purpose::Within(n)
                | Purpose::Grace(n)
                | Purpose::Backoff(n)
                | Purpose::StopDeadline(n) => (n, &nodes),
                Purpose::Budget(s) | Purpose::Zero(s) => (s, &scopes),
            };
            match map[*index] {
                Some(new) => {
                    *index = new;
                    true
                }
                None => false,
            }
        });
        self.timers.shrink_to_fit();
        retain_rows(&mut self.slots, &nodes);
        retain_rows(&mut self.locks, &nodes);
        retain_rows(&mut self.rank, &nodes);
        retain_rows(&mut self.scopes, &scopes);
        std::sync::Arc::make_mut(&mut self.t).compact(&nodes, &scopes);
    }
}

fn mapping(keep: impl IntoIterator<Item = bool>) -> Vec<Option<usize>> {
    let mut next = 0;
    keep.into_iter()
        .map(|keep| {
            if keep {
                let n = next;
                next += 1;
                Some(n)
            } else {
                None
            }
        })
        .collect()
}

pub(super) fn retain_rows<T>(rows: &mut Vec<T>, map: &[Option<usize>]) {
    let mut index = 0;
    rows.retain(|_| {
        let keep = map[index].is_some();
        index += 1;
        keep
    });
    rows.shrink_to_fit();
}
