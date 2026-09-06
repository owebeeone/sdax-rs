//! What a run driver executes: the erased bodies a plan recorded, and the
//! slot tables their outputs live in.
//!
//! **Host API, not author API**, and outside the crate's stability promise
//! like the rest of [`host`](crate::host): an author never names anything
//! here. It is public because the run driver lives in `sdax-tokio`, a
//! different crate, and cannot reach a `pub(crate)` field.
//!
//! [`BodySource`] is the driver's whole view of "what code belongs to this
//! node". [`bodies_of`] gives the one a plan carries; a harness can supply
//! another — the testkit's scripted source is how suite (c) re-runs on a real
//! runtime with the machine's semantics unchanged.
//!
//! **Instances (Stage 3).** A template's plan has one slot table *per
//! instance*, so every signature carries an [`InstanceId`]: `None` is the
//! static graph, `Some(id)` is that instance's copy. The node is always
//! addressed by its **declaration** key — two instances of one template share
//! it — and [`open_instance`](BodySource::open_instance) is what makes the
//! per-instance table exist, with the per-instance input already in it.

use crate::contracts::{BoxFuture, Error};
use crate::cx::{CxInner, InstanceId};
use crate::key::{RawKey, Slots};
use crate::plan::{Kind, Plan, PlanIr};
use std::any::Any;
use std::sync::{Arc, Mutex};

/// A prepare body, with its dependency plumbing erased.
pub type ErasedPrepare = Box<
    dyn Fn(Arc<CxInner>, &Slots) -> Option<BoxFuture<'static, Result<(), Error>>> + Send + Sync,
>;

/// A release or compensation body, with its plumbing erased.
pub type ErasedRelease = ErasedPrepare;

/// A blocking body, run on a pool thread.
pub type ErasedBlocking = Box<
    dyn Fn(Arc<CxInner>, &Slots) -> Option<Box<dyn FnOnce() -> Result<(), Error> + Send>>
        + Send
        + Sync,
>;

/// Copy one import node's value down from the scope that declared it.
pub type ErasedImport = Box<dyn Fn(&Slots, &mut Slots) + Send + Sync>;

/// Read the exported slot of a finished run, still erased.
pub type ErasedExport = Box<dyn Fn(&Slots) -> Option<Box<dyn Any + Send + Sync>> + Send + Sync>;

/// The value a structural node installs before it becomes ready.
pub(crate) enum ReadyValue {
    Unit,
    Export { source: RawKey, read: ErasedExport },
}

#[derive(Debug)]
struct PublicationError {
    node: RawKey,
    instance: Option<InstanceId>,
    source: Option<RawKey>,
    reason: &'static str,
}

impl std::fmt::Display for PublicationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "cannot publish {:?} in {:?} from {:?}: {}",
            self.node, self.instance, self.source, self.reason
        )
    }
}

impl std::error::Error for PublicationError {}

/// What the driver got for a node: a future to poll, or a closure for the
/// blocking pool.
///
/// The *source* decides which, not the driver: the plan's own bodies always
/// answer a blocking step with [`Task::Blocking`], while a harness whose
/// bodies are scripted on the engine clock answers with [`Task::Async`], so
/// that virtual time still governs the attempt. The driver honours whichever
/// it is given and says so in its trace either way.
pub enum Task {
    /// Poll this on the async runtime.
    Async(BoxFuture<'static, Result<(), Error>>),
    /// Run this on the blocking pool.
    Blocking(Box<dyn FnOnce() -> Result<(), Error> + Send>),
}

impl std::fmt::Debug for Task {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Task::Async(_) => "Task::Async",
            Task::Blocking(_) => "Task::Blocking",
        })
    }
}

/// Where a run driver gets the code for one node, and where it puts what a
/// body registered.
///
/// One source serves one run: the slot tables (INV-13) live behind it. Every
/// node is named by its **declaration** key plus the instance it belongs to,
/// because a template's nodes have one declaration and one table per instance.
pub trait BodySource: Send + Sync + 'static {
    /// The prepare, run or start body for this node's next attempt.
    ///
    /// `None` means the node has no body — a join, a component or a template,
    /// for which the machine pushes no spawn at all.
    fn body(&self, node: RawKey, instance: Option<InstanceId>, cx: &Arc<CxInner>) -> Option<Task>;

    /// The release or compensation body for this node.
    fn cleanup(
        &self,
        node: RawKey,
        instance: Option<InstanceId>,
        cx: &Arc<CxInner>,
    ) -> Option<Task>;

    /// Record what a finished body registered, so dependents can read it.
    fn store(&self, node: RawKey, instance: Option<InstanceId>, value: Box<dyn Any + Send + Sync>);

    /// Install a join's unit value or a component's exported value before
    /// acknowledging the engine's publication request. `node` is a declaration
    /// key and `instance` identifies its exact scope copy. Missing or wrongly
    /// typed values must return an error; success permits dependent bodies to
    /// read the slot. This is not a body execution or completion.
    fn publish_ready(&self, node: RawKey, instance: Option<InstanceId>) -> Result<(), Error>;

    /// The plan's exported value, erased: `Arc<Out>` in a box, if the export
    /// slot is filled.
    fn export(&self) -> Option<Box<dyn Any + Send + Sync>>;

    /// Make one instance's slot tables, with the per-instance input already
    /// in them, before any body of the instance is spawned
    /// ([`Effect::SpawnInstance`](crate::host::engine::Effect::SpawnInstance)).
    ///
    /// `template` is the template node's declaration key and `parent` the
    /// instance the spawning body itself belonged to, which is what a template
    /// declared inside a template's plan is resolved against. `input` is a
    /// boxed `Arc<I>`, because that is what a slot holds and what a body that
    /// `needs` the input reads back (`OD-SPAWN-INPUT`).
    fn open_instance(
        &self,
        template: RawKey,
        parent: Option<InstanceId>,
        id: InstanceId,
        input: Box<dyn Any + Send + Sync>,
    );

    /// Drop one instance's slot tables. The machine calls for this only once
    /// the instance's scope has ended, so nothing can still read them.
    fn close_instance(&self, id: InstanceId);
}

/// The bodies one plan recorded, keyed by declaration index, with the bodies
/// of every child plan — component **and** template — hanging off it.
///
/// A child plan is a plan of its own: its nodes have their own [`RawKey`]
/// space and their own bodies, which the parent's declaration alone does not
/// carry.
pub struct Bodies {
    pub(crate) plan: u64,
    pub(crate) prepare: Vec<Option<ErasedPrepare>>,
    pub(crate) release: Vec<Option<ErasedRelease>>,
    pub(crate) blocking: Vec<Option<ErasedBlocking>>,
    /// One entry per import node: which plan the value comes from, and how to
    /// copy it into this plan's slots.
    pub(crate) imports: Vec<(u64, ErasedImport)>,
    pub(crate) ready_values: Vec<(RawKey, ReadyValue)>,
    /// The bodies of the child plans this plan uses — components and templates
    /// alike — in declaration order.
    pub(crate) children: Vec<Arc<Bodies>>,
}

impl Bodies {
    /// Which plan these bodies belong to.
    pub fn plan(&self) -> u64 {
        self.plan
    }

    /// The bodies of the child plans this plan uses.
    pub fn children(&self) -> &[Arc<Bodies>] {
        &self.children
    }
}

impl std::fmt::Debug for Bodies {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bodies")
            .field("plan", &self.plan)
            .field("nodes", &self.prepare.len())
            .field("children", &self.children.len())
            .finish()
    }
}

/// One scope's bodies and its slot table.
struct ScopeBodies {
    bodies: Arc<Bodies>,
    slots: Arc<Mutex<Slots>>,
}

/// One live instance's scopes: the template's plan and every component inside
/// it, each with a table of its own (INV-13, per instance).
struct InstanceBodies {
    id: InstanceId,
    parent: Option<InstanceId>,
    scopes: Vec<ScopeBodies>,
}

/// A scope resolved from an instance outwards: its bodies, its slot table,
/// and which instance's copy it turned out to be.
type Found = (Arc<Bodies>, Arc<Mutex<Slots>>, Option<InstanceId>);

/// One template's declaration, kept so an instance can be opened at run time.
struct TemplateBodies {
    node: RawKey,
    ir: Arc<PlanIr>,
    bodies: Arc<Bodies>,
}

/// The [`BodySource`] a plan carries: its own bodies, its components' bodies,
/// one slot table per scope, and one set of tables per live instance.
struct PlanBodies {
    scopes: Vec<ScopeBodies>,
    templates: Vec<TemplateBodies>,
    instances: Mutex<Vec<Arc<InstanceBodies>>>,
    export: Option<(usize, RawKey, ErasedExport)>,
}

/// The [`BodySource`] behind `plan` — one per run, because the slot tables it
/// owns are per run (INV-13).
///
/// Host API: a driver calls this, an author never does. A plan that declares a
/// per-run input needs [`bodies_of_with_input`] instead; this source leaves the
/// input slot empty, which is what [`Machine::new`](crate::host::engine::Machine::new)
/// refuses such a plan for.
pub fn bodies_of<Out: Send + Sync + 'static, In>(plan: &Plan<Out, In>) -> Arc<dyn BodySource> {
    build_bodies(plan, None)
}

/// [`bodies_of`], with the plan's per-run input already in the root scope's
/// slots — what `start(rt, input)` builds.
///
/// The seeding happens here, before the source is handed to any driver, so the
/// value is in place before the first body is built: exactly the guarantee
/// [`BodySource::open_instance`] gives an instance at the moment of its spawn.
/// The slot holds `Arc<In>`, like every other slot, so a node that `needs` the
/// input receives `Arc<In>` (`OD-SPAWN-INPUT`).
pub fn bodies_of_with_input<Out: Send + Sync + 'static, In: Send + Sync + 'static>(
    plan: &Plan<Out, In>,
    input: In,
) -> Arc<dyn BodySource> {
    build_bodies(plan, Some(Box::new(Arc::new(input))))
}

fn build_bodies<Out: Send + Sync + 'static, In>(
    plan: &Plan<Out, In>,
    input: Option<Box<dyn Any + Send + Sync>>,
) -> Arc<dyn BodySource> {
    let mut scopes = Vec::new();
    collect(plan.ir(), plan.bodies_ref(), &mut scopes);
    // The root's per-run input, in the root scope's slots before any body is
    // built — the same moment `open_instance` fills an instance's.
    if let (Some(key), Some(v)) = (plan.ir().input, input) {
        scopes[0]
            .slots
            .lock()
            .expect("slots poisoned")
            .set_erased(key, v);
    }
    let mut templates = Vec::new();
    collect_templates(plan.ir(), plan.bodies_ref(), &mut templates);
    let export = plan.ir().export.map(|key| {
        let reader: ErasedExport = Box::new(move |slots: &Slots| {
            slots
                .get::<Out>(key)
                .map(|a| Box::new(a) as Box<dyn Any + Send + Sync>)
        });
        (0usize, key, reader)
    });
    Arc::new(PlanBodies {
        scopes,
        templates,
        instances: Mutex::new(Vec::new()),
        export,
    })
}

/// A child plan's bodies sit in `children` in the declaration order of the
/// component and template nodes together.
fn child_bodies<'a>(ir: &PlanIr, bodies: &'a Arc<Bodies>) -> Vec<(&'a Arc<Bodies>, usize)> {
    let mut out = Vec::new();
    let mut child = 0usize;
    for (i, n) in ir.nodes.iter().enumerate() {
        if matches!(n.kind, Kind::Component | Kind::Template) {
            if let Some(b) = bodies.children.get(child) {
                out.push((b, i));
            }
            child += 1;
        }
    }
    out
}

/// Walk the declaration and the bodies together, one [`ScopeBodies`] per plan.
/// A template's plan is **not** a scope of the run: it has one table per
/// instance, made by `open_instance`.
fn collect(ir: &PlanIr, bodies: &Arc<Bodies>, out: &mut Vec<ScopeBodies>) {
    out.push(ScopeBodies {
        bodies: bodies.clone(),
        slots: Arc::new(Mutex::new(Slots::new(ir.nodes.len()))),
    });
    for (b, i) in child_bodies(ir, bodies) {
        if ir.nodes[i].kind != Kind::Component {
            continue;
        }
        if let Some(inner) = &ir.nodes[i].child {
            collect(inner, b, out);
        }
    }
}

/// Every template anywhere in the declaration tree, templates inside template
/// plans included, so a nested `cx.spawn` finds its bodies.
fn collect_templates(ir: &PlanIr, bodies: &Arc<Bodies>, out: &mut Vec<TemplateBodies>) {
    for (b, i) in child_bodies(ir, bodies) {
        let Some(inner) = &ir.nodes[i].child else {
            continue;
        };
        if ir.nodes[i].kind == Kind::Template {
            out.push(TemplateBodies {
                node: ir.nodes[i].key,
                ir: inner.clone(),
                bodies: b.clone(),
            });
        }
        collect_templates(inner, b, out);
    }
}

impl PlanBodies {
    fn instance(&self, id: InstanceId) -> Option<Arc<InstanceBodies>> {
        self.instances
            .lock()
            .expect("instances poisoned")
            .iter()
            .find(|i| i.id == id)
            .cloned()
    }

    /// The bodies and slot table for `plan`, as seen from `instance`: that
    /// instance's own tables first, then the instance that spawned it, then
    /// the static graph.
    fn find(&self, plan: u64, instance: Option<InstanceId>) -> Option<Found> {
        let mut at = instance;
        while let Some(id) = at {
            let inst = self.instance(id)?;
            if let Some(s) = inst.scopes.iter().find(|s| s.bodies.plan == plan) {
                return Some((s.bodies.clone(), s.slots.clone(), Some(id)));
            }
            at = inst.parent;
        }
        self.scopes
            .iter()
            .find(|s| s.bodies.plan == plan)
            .map(|s| (s.bodies.clone(), s.slots.clone(), None))
    }

    /// Resolve only the requested run or instance, without ancestor fallback.
    fn find_exact(&self, plan: u64, instance: Option<InstanceId>) -> Option<Found> {
        if let Some(id) = instance {
            let inst = self.instance(id)?;
            return inst
                .scopes
                .iter()
                .find(|s| s.bodies.plan == plan)
                .map(|s| (s.bodies.clone(), s.slots.clone(), Some(id)));
        }
        self.scopes
            .iter()
            .find(|s| s.bodies.plan == plan)
            .map(|s| (s.bodies.clone(), s.slots.clone(), None))
    }

    /// Copy every import of this scope down from the scope that declared it.
    ///
    /// Cheap (each copy is an `Arc::clone`) and always correct: T1 starts a
    /// node only once every need — imports included — is `Ready`, so the
    /// source slot is filled by the time a body of this scope is built.
    fn refresh_imports(
        &self,
        bodies: &Arc<Bodies>,
        slots: &Arc<Mutex<Slots>>,
        instance: Option<InstanceId>,
    ) {
        for (src_plan, copy) in &bodies.imports {
            let Some((from_bodies, from_slots, from_inst)) = self.find(*src_plan, instance) else {
                continue;
            };
            if Arc::ptr_eq(&from_slots, slots) {
                continue;
            }
            self.refresh_imports(&from_bodies, &from_slots, from_inst);
            let parent = from_slots.lock().expect("slots poisoned");
            let mut mine = slots.lock().expect("slots poisoned");
            copy(&parent, &mut mine);
        }
    }
}

impl BodySource for PlanBodies {
    fn body(&self, node: RawKey, instance: Option<InstanceId>, cx: &Arc<CxInner>) -> Option<Task> {
        let (bodies, slots, inst) = self.find(node.plan, instance)?;
        self.refresh_imports(&bodies, &slots, inst);
        let slots = slots.lock().expect("slots poisoned");
        let idx = node.idx as usize;
        if let Some(Some(b)) = bodies.blocking.get(idx) {
            return b(cx.clone(), &slots).map(Task::Blocking);
        }
        let f = bodies.prepare.get(idx)?.as_ref()?;
        f(cx.clone(), &slots).map(Task::Async)
    }

    fn cleanup(
        &self,
        node: RawKey,
        instance: Option<InstanceId>,
        cx: &Arc<CxInner>,
    ) -> Option<Task> {
        let (bodies, slots, _) = self.find(node.plan, instance)?;
        let slots = slots.lock().expect("slots poisoned");
        let f = bodies.release.get(node.idx as usize)?.as_ref()?;
        f(cx.clone(), &slots).map(Task::Async)
    }

    fn store(&self, node: RawKey, instance: Option<InstanceId>, value: Box<dyn Any + Send + Sync>) {
        if let Some((_, slots, _)) = self.find(node.plan, instance) {
            slots
                .lock()
                .expect("slots poisoned")
                .set_erased(node, value);
        }
    }

    fn publish_ready(&self, node: RawKey, instance: Option<InstanceId>) -> Result<(), Error> {
        let fail = |source, reason| -> Error {
            Box::new(PublicationError {
                node,
                instance,
                source,
                reason,
            })
        };
        let (bodies, destination, _) = self
            .find_exact(node.plan, instance)
            .ok_or_else(|| fail(None, "destination scope unavailable"))?;
        let value = bodies
            .ready_values
            .iter()
            .find(|(key, _)| *key == node)
            .map(|(_, value)| value)
            .ok_or_else(|| fail(None, "node has no structural value"))?;
        let value: Box<dyn Any + Send + Sync> = match value {
            ReadyValue::Unit => Box::new(Arc::new(())),
            ReadyValue::Export { source, read } => {
                let (source_bodies, source_slots, source_instance) = self
                    .find_exact(source.plan, instance)
                    .ok_or_else(|| fail(Some(*source), "export scope unavailable"))?;
                self.refresh_imports(&source_bodies, &source_slots, source_instance);
                let slots = source_slots.lock().expect("slots poisoned");
                read(&slots)
                    .ok_or_else(|| fail(Some(*source), "export missing or wrongly typed"))?
            }
        };
        // The source lock above is released before taking the destination lock.
        destination
            .lock()
            .expect("slots poisoned")
            .set_erased(node, value);
        Ok(())
    }

    fn export(&self) -> Option<Box<dyn Any + Send + Sync>> {
        let (scope, _, read) = self.export.as_ref()?;
        let slots = self.scopes[*scope].slots.lock().expect("slots poisoned");
        read(&slots)
    }

    fn open_instance(
        &self,
        template: RawKey,
        parent: Option<InstanceId>,
        id: InstanceId,
        input: Box<dyn Any + Send + Sync>,
    ) {
        let Some(t) = self.templates.iter().find(|t| t.node == template) else {
            return;
        };
        let mut scopes = Vec::new();
        collect(&t.ir, &t.bodies, &mut scopes);
        if let Some(key) = t.ir.input {
            scopes[0]
                .slots
                .lock()
                .expect("slots poisoned")
                .set_erased(key, input);
        }
        self.instances
            .lock()
            .expect("instances poisoned")
            .push(Arc::new(InstanceBodies { id, parent, scopes }));
    }

    fn close_instance(&self, id: InstanceId) {
        self.instances
            .lock()
            .expect("instances poisoned")
            .retain(|i| i.id != id);
    }
}
