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

use crate::contracts::{BoxFuture, Error};
use crate::cx::CxInner;
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
/// One source serves one run: the slot tables (INV-13) live behind it.
pub trait BodySource: Send + Sync + 'static {
    /// The prepare, run or start body for this node's next attempt.
    ///
    /// `None` means the node has no body — a join or a component, for which
    /// the machine pushes no spawn at all.
    fn body(&self, node: RawKey, cx: &Arc<CxInner>) -> Option<Task>;

    /// The release or compensation body for this node.
    fn cleanup(&self, node: RawKey, cx: &Arc<CxInner>) -> Option<Task>;

    /// Record what a finished body registered, so dependents can read it.
    fn store(&self, node: RawKey, value: Box<dyn Any + Send + Sync>);

    /// The plan's exported value, erased: `Arc<Out>` in a box, if the export
    /// slot is filled.
    fn export(&self) -> Option<Box<dyn Any + Send + Sync>>;
}

/// The bodies one plan recorded, keyed by declaration index, with the bodies
/// of every component plan hanging off it.
///
/// A component's child plan is a plan of its own: its nodes have their own
/// [`RawKey`] space and their own bodies, which the parent's declaration
/// alone does not carry.
pub struct Bodies {
    pub(crate) plan: u64,
    pub(crate) prepare: Vec<Option<ErasedPrepare>>,
    pub(crate) release: Vec<Option<ErasedRelease>>,
    pub(crate) blocking: Vec<Option<ErasedBlocking>>,
    /// One entry per import node: which plan the value comes from, and how to
    /// copy it into this plan's slots.
    pub(crate) imports: Vec<(u64, ErasedImport)>,
    /// The bodies of every component plan this plan uses, in declaration
    /// order.
    pub(crate) children: Vec<Arc<Bodies>>,
}

impl Bodies {
    /// Which plan these bodies belong to.
    pub fn plan(&self) -> u64 {
        self.plan
    }

    /// The bodies of the component plans this plan uses.
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
    slots: Mutex<Slots>,
}

/// The [`BodySource`] a plan carries: its own bodies, its components' bodies,
/// and one slot table per scope.
struct PlanBodies {
    scopes: Vec<ScopeBodies>,
    export: Option<(usize, RawKey, ErasedExport)>,
}

/// The [`BodySource`] behind `plan` — one per run, because the slot tables it
/// owns are per run (INV-13).
///
/// Host API: a driver calls this, an author never does.
pub fn bodies_of<Out: Send + Sync + 'static, In>(plan: &Plan<Out, In>) -> Arc<dyn BodySource> {
    let mut scopes = Vec::new();
    collect(plan.ir(), plan.bodies_ref(), &mut scopes);
    let export = plan.ir().export.map(|key| {
        let reader: ErasedExport = Box::new(move |slots: &Slots| {
            slots
                .get::<Out>(key)
                .map(|a| Box::new(a) as Box<dyn Any + Send + Sync>)
        });
        (0usize, key, reader)
    });
    Arc::new(PlanBodies { scopes, export })
}

/// Walk the declaration and the bodies together, one [`ScopeBodies`] per plan.
fn collect(ir: &PlanIr, bodies: &Arc<Bodies>, out: &mut Vec<ScopeBodies>) {
    out.push(ScopeBodies {
        bodies: bodies.clone(),
        slots: Mutex::new(Slots::new(ir.nodes.len())),
    });
    let mut child = 0usize;
    for n in &ir.nodes {
        if n.kind == Kind::Component {
            if let (Some(inner), Some(b)) = (&n.child, bodies.children.get(child)) {
                collect(inner, b, out);
            }
            child += 1;
        }
    }
}

impl PlanBodies {
    fn scope_of(&self, plan: u64) -> Option<usize> {
        self.scopes.iter().position(|s| s.bodies.plan == plan)
    }

    /// Copy every import of `scope` down from the scope that declared it.
    ///
    /// Cheap (each copy is an `Arc::clone`) and always correct: T1 starts a
    /// node only once every need — imports included — is `Ready`, so the
    /// source slot is filled by the time a body of this scope is built.
    fn refresh_imports(&self, scope: usize) {
        for (src_plan, copy) in &self.scopes[scope].bodies.imports {
            let Some(from) = self.scope_of(*src_plan) else {
                continue;
            };
            if from == scope {
                continue;
            }
            self.refresh_imports(from);
            let parent = self.scopes[from].slots.lock().expect("slots poisoned");
            let mut mine = self.scopes[scope].slots.lock().expect("slots poisoned");
            copy(&parent, &mut mine);
        }
    }
}

impl BodySource for PlanBodies {
    fn body(&self, node: RawKey, cx: &Arc<CxInner>) -> Option<Task> {
        let s = self.scope_of(node.plan)?;
        self.refresh_imports(s);
        let scope = &self.scopes[s];
        let slots = scope.slots.lock().expect("slots poisoned");
        let idx = node.idx as usize;
        if let Some(Some(b)) = scope.bodies.blocking.get(idx) {
            return b(cx.clone(), &slots).map(Task::Blocking);
        }
        let f = scope.bodies.prepare.get(idx)?.as_ref()?;
        f(cx.clone(), &slots).map(Task::Async)
    }

    fn cleanup(&self, node: RawKey, cx: &Arc<CxInner>) -> Option<Task> {
        let s = self.scope_of(node.plan)?;
        let scope = &self.scopes[s];
        let slots = scope.slots.lock().expect("slots poisoned");
        let f = scope.bodies.release.get(node.idx as usize)?.as_ref()?;
        f(cx.clone(), &slots).map(Task::Async)
    }

    fn store(&self, node: RawKey, value: Box<dyn Any + Send + Sync>) {
        if let Some(s) = self.scope_of(node.plan) {
            let mut slots = self.scopes[s].slots.lock().expect("slots poisoned");
            slots.set_erased(node, value);
        }
    }

    fn export(&self) -> Option<Box<dyn Any + Send + Sync>> {
        let (scope, _, read) = self.export.as_ref()?;
        let slots = self.scopes[*scope].slots.lock().expect("slots poisoned");
        read(&slots)
    }
}
