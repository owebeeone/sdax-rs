//! Remap every topology reference in one transaction; raw keys never change.
use super::Table;
use crate::host::engine::compaction::retain_rows;

impl Table {
    pub(in crate::host::engine) fn compact(
        &mut self,
        nodes: &[Option<usize>],
        scopes: &[Option<usize>],
    ) {
        retain_rows(&mut self.nodes, nodes);
        retain_rows(&mut self.scopes, scopes);
        let required =
            |i: usize| nodes[i].expect("active topology cannot depend on a retired instance");
        for node in &mut self.nodes {
            node.scope = scopes[node.scope].expect("active node scope");
            node.inner = node
                .inner
                .map(|s| scopes[s].expect("active component scope"));
            node.needs.iter_mut().for_each(|i| *i = required(*i));
            node.exclusive.iter_mut().for_each(|i| *i = required(*i));
            node.shared.iter_mut().for_each(|i| *i = required(*i));
            remap_list(&mut node.dependents, nodes);
        }
        for scope in &mut self.scopes {
            scope.nodes.iter_mut().for_each(|i| *i = required(*i));
            scope.parent = scope
                .parent
                .map(|s| scopes[s].expect("active parent scope"));
            scope.component = scope.component.map(required);
            scope.instance = scope.instance.map(|(t, id)| (required(t), id));
            scope.export = scope.export.map(required);
            if let Some(map) = &mut scope.map {
                map.retain_mut(|(_, i)| remap(i, nodes));
                map.shrink_to_fit();
            }
        }
        self.map.retain_mut(|(_, i)| remap(i, nodes));
        self.map.shrink_to_fit();
        self.index.0.retain(|_, entries| {
            let mut any = false;
            for i in entries {
                *i = if *i == usize::MAX {
                    usize::MAX
                } else {
                    nodes[*i].unwrap_or(usize::MAX)
                };
                any |= *i != usize::MAX;
            }
            any
        });
    }
}

fn remap(i: &mut usize, map: &[Option<usize>]) -> bool {
    match map[*i] {
        Some(new) => {
            *i = new;
            true
        }
        None => false,
    }
}
fn remap_list(list: &mut Vec<usize>, map: &[Option<usize>]) {
    list.retain_mut(|i| remap(i, map));
    if list.capacity() > list.len().saturating_mul(2) {
        list.shrink_to_fit();
    }
}
