//! Declared alias disposal for RAII cleanup. Opaque user values are not scanned.
use super::*;
use crate::plan::ReleaseStyle;
use std::collections::{BTreeMap, BTreeSet};

/// Cache transitive aliases of RAII resources while building the plan.
pub(super) fn alias_layout(ir: &PlanIr) -> BTreeMap<RawKey, Vec<RawKey>> {
    fn declarations(
        ir: &PlanIr,
        sources: &mut BTreeMap<RawKey, RawKey>,
        owners: &mut BTreeSet<RawKey>,
    ) {
        for node in &ir.nodes {
            if node.attrs.release == ReleaseStyle::Drop {
                owners.insert(node.key);
            }
            if let Some(source) = node.source {
                sources.insert(node.key, source);
            }
            if let Some(child) = &node.child {
                if node.kind == Kind::Component {
                    if let Some(export) = child.export {
                        sources.insert(node.key, export);
                    }
                }
                declarations(child, sources, owners);
            }
        }
    }
    let mut sources = BTreeMap::new();
    let mut owners = BTreeSet::new();
    declarations(ir, &mut sources, &mut owners);
    let mut aliases: BTreeMap<RawKey, Vec<RawKey>> = BTreeMap::new();
    for (&destination, &source) in &sources {
        let mut owner = source;
        // Valid bindings are acyclic. The bound keeps this defensive even if
        // the host constructs an invalid declaration outside the author API.
        for _ in 0..sources.len() {
            match sources.get(&owner) {
                Some(next) => owner = *next,
                None => break,
            }
        }
        if owners.contains(&owner) {
            aliases.entry(owner).or_default().push(destination);
        }
    }
    aliases
}

impl PlanBodies {
    /// Move declared aliases into the cleanup task, including exact dynamic
    /// copies. Taking values never runs user destructors under these locks.
    pub(super) fn take_drop_aliases(
        &self,
        owner: RawKey,
        instance: Option<InstanceId>,
        owner_slots: &Arc<Mutex<Slots>>,
    ) -> Vec<Box<dyn Any + Send + Sync>> {
        let Some(aliases) = self.layout.drop_aliases.get(&owner) else {
            return Vec::new();
        };
        let instances = self.instances.lock().expect("instances poisoned").clone();
        let mut removed = Vec::new();
        for &alias in aliases {
            if instance.is_none() {
                if let Some((_, slots, _)) = self.find_exact(alias.plan, None) {
                    if let Some(value) = slots.lock().expect("slots poisoned").take_erased(alias) {
                        removed.push(value);
                    }
                }
            }
            for candidate in &instances {
                let Some(scope) = candidate
                    .scopes
                    .iter()
                    .find(|s| s.bodies.plan == alias.plan)
                else {
                    continue;
                };
                let Some((_, origin, _)) = self.find(owner.plan, Some(candidate.id)) else {
                    continue;
                };
                if Arc::ptr_eq(owner_slots, &origin) {
                    if let Some(value) = scope
                        .slots
                        .lock()
                        .expect("slots poisoned")
                        .take_erased(alias)
                    {
                        removed.push(value);
                    }
                }
            }
        }
        removed
    }
}
