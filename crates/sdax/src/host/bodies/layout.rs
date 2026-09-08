//! Immutable scope/body binding layout, prepared once during plan building.
use super::*;

pub(super) struct BoundBodies {
    pub plan: u64,
    pub ir: Arc<PlanIr>,
    pub original: Arc<Bodies>,
    pub bindings: Vec<(RawKey, usize)>,
}
impl std::ops::Deref for BoundBodies {
    type Target = Bodies;
    fn deref(&self) -> &Bodies {
        &self.original
    }
}

pub(super) struct TemplateBodies {
    pub node: RawKey,
    pub input: Option<RawKey>,
    pub scopes: Vec<Arc<BoundBodies>>,
}

/// Immutable factory references and mounted bindings, shared by every run.
pub(crate) struct BodyLayout {
    pub(super) scopes: Vec<Arc<BoundBodies>>,
    pub(super) templates: Vec<TemplateBodies>,
    /// Declared slot aliases of each RAII resource, across all scope definitions.
    pub(super) drop_aliases: std::collections::BTreeMap<RawKey, Vec<RawKey>>,
}

impl BodyLayout {
    pub(crate) fn build(ir: &Arc<PlanIr>, bodies: &Arc<Bodies>) -> Arc<Self> {
        let mut scopes = Vec::new();
        collect(ir, bodies, &mut scopes);
        let mut templates = Vec::new();
        collect_templates(ir, bodies, &mut templates);
        let drop_aliases = super::disposal::alias_layout(ir);
        Arc::new(Self {
            scopes,
            templates,
            drop_aliases,
        })
    }
}

fn children<'a>(
    ir: &'a Arc<PlanIr>,
    bodies: &'a Arc<Bodies>,
) -> impl Iterator<Item = (&'a crate::plan::NodeDecl, &'a Arc<Bodies>)> {
    ir.nodes
        .iter()
        .filter(|n| matches!(n.kind, Kind::Component | Kind::Template))
        .zip(&bodies.children)
}

fn collect(ir: &Arc<PlanIr>, bodies: &Arc<Bodies>, out: &mut Vec<Arc<BoundBodies>>) {
    let bindings = bodies
        .imports
        .iter()
        .enumerate()
        .filter_map(|(i, (key, _))| ir.nodes[key.idx as usize].source.map(|source| (source, i)))
        .collect();
    out.push(Arc::new(BoundBodies {
        plan: ir.id,
        ir: ir.clone(),
        original: bodies.clone(),
        bindings,
    }));
    for (node, body) in children(ir, bodies) {
        if node.kind == Kind::Component {
            if let Some(inner) = &node.child {
                collect(inner, body, out);
            }
        }
    }
}

fn collect_templates(ir: &Arc<PlanIr>, bodies: &Arc<Bodies>, out: &mut Vec<TemplateBodies>) {
    for (node, body) in children(ir, bodies) {
        let Some(inner) = &node.child else {
            continue;
        };
        if node.kind == Kind::Template {
            let mut scopes = Vec::new();
            collect(inner, body, &mut scopes);
            out.push(TemplateBodies {
                node: node.key,
                input: inner.input,
                scopes,
            });
        }
        collect_templates(inner, body, out);
    }
}

/// Allocate only mutable value slots; topology and body factories are shared.
pub(super) fn open_scopes(layout: &[Arc<BoundBodies>]) -> Vec<ScopeBodies> {
    layout
        .iter()
        .enumerate()
        .map(|(index, body)| {
            let mut slots = Slots::new(body.ir.nodes.len());
            if index > 0 {
                if let Some(input) = body
                    .ir
                    .input
                    .filter(|key| body.ir.nodes[key.idx as usize].source.is_none())
                {
                    slots.set(input, Arc::new(()));
                }
            }
            ScopeBodies {
                bodies: body.clone(),
                slots: Arc::new(Mutex::new(slots)),
            }
        })
        .collect()
}
