//! Typed mount bindings and definition-to-mount identity normalization.
use super::*;

/// Give every scope in one mount its own identity without copying bodies.
pub(crate) fn mount_ir(ir: &PlanIr) -> PlanIr {
    fn allocate(ir: &PlanIr, map: &mut Vec<(u64, u64)>) {
        map.push((ir.id, next_plan_id()));
        for child in ir.nodes.iter().filter_map(|n| n.child.as_ref()) {
            allocate(child, map);
        }
    }
    fn remap(ir: &PlanIr, map: &[(u64, u64)]) -> PlanIr {
        let key = |mut k: RawKey| {
            if let Some((_, id)) = map.iter().find(|(old, _)| *old == k.plan) {
                k.plan = *id;
            }
            k
        };
        let mut out = ir.clone();
        out.id = key(RawKey {
            plan: ir.id,
            idx: 0,
        })
        .plan;
        out.input = ir.input.map(key);
        out.export = ir.export.map(key);
        for n in &mut out.nodes {
            n.key = key(n.key);
            n.needs.iter_mut().for_each(|k| *k = key(*k));
            n.source = n.source.map(key);
            n.spawns.iter_mut().for_each(|k| *k = key(*k));
            n.attrs.exclusive.iter_mut().for_each(|k| *k = key(*k));
            n.attrs.shared.iter_mut().for_each(|k| *k = key(*k));
            for p in [&mut n.attrs.limit, &mut n.attrs.pool]
                .into_iter()
                .flatten()
            {
                p.plan = key(RawKey {
                    plan: p.plan,
                    idx: 0,
                })
                .plan;
            }
            if let Some(child) = &n.child {
                n.child = Some(Arc::new(remap(child, map)));
            }
        }
        out
    }
    let mut map = Vec::new();
    allocate(ir, &mut map);
    remap(ir, &map)
}

mod binding_sealed {
    pub trait Sealed<I: ?Sized> {}
    impl<I: ?Sized> Sealed<I> for crate::Key<I> {}
    impl Sealed<()> for () {}
}

/// Explicit input for a static mount: a parent `Key<I>`, or `()` for unit.
/// A key binding retains its parent's dependency and cleanup lifetime.
pub trait InputBinding<I>: binding_sealed::Sealed<I> {
    /// The parent key whose value supplies the input, if nonconstant.
    fn source(self) -> Option<RawKey>;
}
impl<I> InputBinding<I> for crate::Key<I> {
    fn source(self) -> Option<RawKey> {
        Some(self.raw())
    }
}
impl InputBinding<()> for () {
    fn source(self) -> Option<RawKey> {
        None
    }
}
