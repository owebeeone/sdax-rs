//! The plan value and the declaration it records.
//!
//! A [`Plan`] is immutable, `Send + Sync` and reusable: it is built once and
//! (from Stage 1) started any number of times, each start being a run with its
//! own slots, locks and pools (INV-13). Everything the engine derives — start
//! eligibility, the release graph, layers, arbitration — is derived from the
//! declaration recorded here and from nothing else (INV-1).

use crate::cx::{Child, Cx, SpawnError};
use crate::host::bodies::Bodies;
use crate::key::RawKey;
use crate::policy::{Ambiguity, CancelMode, Mode, Policy, Restart, Retry, Shutdown};
use crate::view::NodePath;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// The semantics tag every plan carries. A change of meaning changes this tag.
pub const SEMANTICS: &str = "sdax/1";

pub(crate) static PLAN_IDS: AtomicU64 = AtomicU64::new(1);

/// What a node is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// Acquired once, held, released once.
    Resource,
    /// Finite work; completion is readiness; nothing to release.
    Step,
    /// Finite work whose failure is a value, not a fault.
    TryStep,
    /// Synchronous work on a declared pool.
    BlockingStep,
    /// Long-lived; readiness is the return of `start` with a `Serving`.
    Service,
    /// An externally visible action with a compensation, or declared persistent.
    Effect,
    /// Synchronisation only.
    Join,
    /// A nested plan, instantiated once per parent run.
    Component,
    /// A nested plan factory, instantiated at run time by a body.
    Template,
    /// A cross-scope need: this child node carries a parent plan's key.
    Import,
    /// A template's per-instance input.
    Input,
}

impl Kind {
    /// The word `inspect()` prints.
    pub fn label(&self) -> &'static str {
        match self {
            Kind::Resource => "resource",
            Kind::Step => "step",
            Kind::TryStep => "try_step",
            Kind::BlockingStep => "blocking_step",
            Kind::Service => "service",
            Kind::Effect => "effect",
            Kind::Join => "join",
            Kind::Component => "component",
            Kind::Template => "template",
            Kind::Import => "import",
            Kind::Input => "input",
        }
    }

    /// Whether a node of this kind can carry a release obligation at all.
    pub fn can_hold(&self) -> bool {
        matches!(self, Kind::Resource | Kind::Effect)
    }
}

/// How a node's obligation is discharged at cleanup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReleaseStyle {
    /// Nothing to discharge (steps, joins, inputs, imports).
    None,
    /// An async `release` body.
    Async,
    /// `release::by_drop()`: the value's own `Drop` is the release.
    Drop,
    /// An async `compensate` body for an effect.
    Compensate,
    /// A declared-persistent effect: the record stands and is never compensated
    /// (F2, `AdversarialReview.md` F-B3).
    Persistent,
    /// Signal, wait up to `stop_within`, then abort.
    Stop,
    /// The child's own release graph, run as one unit.
    Inner,
    /// Every live instance is stopped.
    Instances,
}

impl ReleaseStyle {
    /// The word `inspect()` prints after `release:`.
    pub fn label(&self) -> &'static str {
        match self {
            ReleaseStyle::None => "—",
            ReleaseStyle::Async => "async",
            ReleaseStyle::Drop => "drop",
            ReleaseStyle::Compensate => "compensate",
            ReleaseStyle::Persistent => "persistent",
            ReleaseStyle::Stop => "stop",
            ReleaseStyle::Inner => "inner graph",
            ReleaseStyle::Instances => "instances",
        }
    }
}

/// A concurrency budget declared on a plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Pool {
    pub(crate) plan: u64,
    pub(crate) idx: u32,
}

impl Pool {
    /// Which plan declared this pool.
    pub fn plan(&self) -> u64 {
        self.plan
    }
    /// The pool's index within that plan.
    pub fn index(&self) -> u32 {
        self.idx
    }
}

/// A declared pool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoolDecl {
    /// The author's name for it.
    pub name: String,
    /// How many nodes may hold it at once.
    pub limit: usize,
}

/// Everything the author said about one node.
#[derive(Debug, Clone, PartialEq)]
pub struct Attrs {
    /// Prepare/run deadline.
    pub within: Option<Duration>,
    /// Re-execution of the prepare body.
    pub retry: Option<Retry>,
    /// Re-running a faulted service.
    pub restart: Option<Restart>,
    /// The author's assertion that re-execution is safe. Trusted, not verified.
    pub idempotent: bool,
    /// A service's stop budget.
    pub stop_within: Option<Duration>,
    /// A service whose finishing ends the scope.
    pub terminal: bool,
    /// What to do about an ambiguous effect. Required on effects.
    pub on_ambiguous: Option<Ambiguity>,
    /// How an in-flight body is cancelled.
    pub cancel: CancelMode,
    /// Resources this node takes exclusively. Several different resources may
    /// be taken; one resource twice, or in both modes, is `V-DUP-ATTR`.
    pub exclusive: Vec<RawKey>,
    /// Resources this node takes shared, under the same rule as `exclusive`.
    pub shared: Vec<RawKey>,
    /// A pool bounding this node's concurrency.
    pub limit: Option<Pool>,
    /// The required pool of a blocking step.
    pub pool: Option<Pool>,
    /// How the node's obligation is discharged.
    pub release: ReleaseStyle,
    /// Every attribute the author set explicitly, in the order set. A name
    /// appearing twice is `V-DUP-ATTR`; a name absent means the value shown by
    /// `inspect()` is the engine's, which is what makes a default change
    /// diffable. Locks are not listed here — they have no engine default and
    /// are checked per key rather than per name.
    pub declared: Vec<&'static str>,
}

impl Default for Attrs {
    fn default() -> Self {
        Attrs {
            within: None,
            retry: None,
            restart: None,
            idempotent: false,
            stop_within: None,
            terminal: false,
            on_ambiguous: None,
            cancel: CancelMode::Drop,
            exclusive: Vec::new(),
            shared: Vec::new(),
            limit: None,
            pool: None,
            release: ReleaseStyle::None,
            declared: Vec::new(),
        }
    }
}

/// One node's declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeDecl {
    /// This node's key.
    pub key: RawKey,
    /// The author's name for it, unique within the scope.
    pub name: String,
    /// What kind of node it is.
    pub kind: Kind,
    /// Declared dependencies, in the order written.
    pub needs: Vec<RawKey>,
    /// For an `Import` node: the parent key it names.
    pub source: Option<RawKey>,
    /// Templates this service declared it may instantiate (F1).
    pub spawns: Vec<RawKey>,
    /// For a component or template: the child plan.
    pub child: Option<Arc<PlanIr>>,
    /// Everything else the author said.
    pub attrs: Attrs,
}

/// A validated plan's declaration, as data.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanIr {
    /// Identity of the builder that produced this plan.
    pub id: u64,
    /// The author's name for the plan.
    pub name: String,
    /// The semantics tag, always [`SEMANTICS`] for this crate version.
    pub semantics: &'static str,
    /// Nodes in declaration order.
    pub nodes: Vec<NodeDecl>,
    /// Declared pools.
    pub pools: Vec<PoolDecl>,
    /// The scope's fail policy.
    pub policy: Policy,
    /// The scope's shutdown budget.
    pub shutdown: Shutdown,
    /// How the run ends by itself.
    pub mode: Mode,
    /// The exported node, if any.
    pub export: Option<RawKey>,
    /// The template input node, if this plan is a template.
    pub input: Option<RawKey>,
    /// Late `spawns(key, &template)` declarations whose key is not a node of
    /// this plan, as `(the key named, the template node)`. They are recorded
    /// rather than dropped so [`Rule::ForeignKey`](crate::Rule::ForeignKey)
    /// can report them; a declaration that vanishes is worse than one that is
    /// refused.
    pub foreign_spawns: Vec<(RawKey, RawKey)>,
}

impl PlanIr {
    /// The node behind a key of this plan.
    pub fn node(&self, key: RawKey) -> Option<&NodeDecl> {
        self.nodes.get(key.idx as usize).filter(|n| n.key == key)
    }

    /// The parent keys this plan imports.
    pub fn imports(&self) -> Vec<RawKey> {
        self.nodes.iter().filter_map(|n| n.source).collect()
    }
}

/// An immutable, reusable lifecycle declaration.
///
/// `Out` is the plan's exported value; `In` is a template's per-instance input.
/// A plan with `In = ()` is a root or a component; a template is
/// `Plan<Out, In>` and has no `start`.
#[allow(clippy::type_complexity)] // the phantom encodes variance, not data
pub struct Plan<Out = (), In = ()> {
    pub(crate) ir: Arc<PlanIr>,
    pub(crate) bodies: Arc<Bodies>,
    pub(crate) _t: PhantomData<fn() -> (Arc<Out>, In)>,
}

impl<Out, In> Clone for Plan<Out, In> {
    fn clone(&self) -> Self {
        Plan {
            ir: self.ir.clone(),
            bodies: self.bodies.clone(),
            _t: PhantomData,
        }
    }
}

impl<Out, In> std::fmt::Debug for Plan<Out, In> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Plan")
            .field("name", &self.ir.name)
            .field("nodes", &self.ir.nodes.len())
            .finish()
    }
}

impl<Out, In> Plan<Out, In> {
    /// The recorded declaration. Host API, like
    /// [`bodies_ref`](Self::bodies_ref): the engine and the run driver read
    /// it, an author does not.
    pub(crate) fn ir(&self) -> &PlanIr {
        &self.ir
    }

    /// The erased bodies this plan recorded.
    ///
    /// Host API, reached through
    /// [`host::bodies_of`](crate::host::bodies_of) rather than named here by
    /// an author.
    pub(crate) fn bodies_ref(&self) -> &Arc<Bodies> {
        &self.bodies
    }

    /// The plan's name.
    pub fn name(&self) -> &str {
        &self.ir.name
    }

    /// The semantics tag this plan was built under.
    pub fn semantics(&self) -> &'static str {
        self.ir.semantics
    }

    /// The import nodes of this plan, which a root run could not resolve.
    ///
    /// `L-IMPORTS` in the gate inventory: starting a plan with unresolved
    /// imports as a root is refused before any spawn. Stage 0 exposes the
    /// decision procedure; Stage 1's `start` calls it.
    ///
    /// Each path names an import node **of this plan** — the node
    /// `import(parent_key)` created — rather than the ancestor key behind it,
    /// which this plan cannot name: the key belongs to a plan whose
    /// declaration is not in scope here. Paths are what the report, the trace
    /// and `inspect()` address nodes by, so a refusal reads in the same
    /// vocabulary as everything else.
    pub fn unresolved_imports(&self) -> Vec<NodePath> {
        self.ir
            .nodes
            .iter()
            .filter(|n| n.kind == Kind::Import)
            .map(|n| NodePath::root(&n.name))
            .collect()
    }
}

/// A registered template: what a body instantiates through `cx.spawn`.
pub struct Template<I> {
    pub(crate) node: RawKey,
    pub(crate) _i: PhantomData<fn(I)>,
}

impl<I> Clone for Template<I> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<I> Copy for Template<I> {}

impl<I> Template<I> {
    /// The template node this handle names.
    pub fn node(&self) -> RawKey {
        self.node
    }
}

impl<I> std::fmt::Debug for Template<I> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Template({}/{})", self.node.plan, self.node.idx)
    }
}

impl<P> Cx<P> {
    /// Ask the scope to instantiate a template with a per-instance input.
    ///
    /// The input lands in the instance's own slot table, which every body of
    /// the instance reads, so `I` is `Send + Sync` — which
    /// `Plan::template::<In>` already requires, so no `Template` that can be
    /// built is excluded (`OD-SPAWN-INPUT`).
    ///
    /// Only a node that declared `spawns(&template)` may do this, and only
    /// while the scope is still admitting starts. From a `Cx<Start>` body the
    /// returned [`Child`] can be awaited with
    /// [`Child::ready`](crate::Child::ready), so a service's readiness can
    /// include its instances (F1).
    pub fn spawn<I: Send + Sync + 'static>(
        &self,
        template: &Template<I>,
        input: I,
    ) -> Result<Child, SpawnError> {
        // `Arc<I>`, not `I`: a slot holds `Arc<T>` for every node, and the
        // instance's input is a slot like any other, so a body that `needs` it
        // reads `Arc<I>` back out (`OD-SPAWN-INPUT`).
        self.spawn_raw(template.node, Box::new(Arc::new(input)))
    }
}

/// The next plan identity.
pub(crate) fn next_plan_id() -> u64 {
    PLAN_IDS.fetch_add(1, Ordering::SeqCst)
}
