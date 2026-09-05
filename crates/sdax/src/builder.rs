//! The authoring surface: [`PlanBuilder`] and the per-node [`Node`] chain.
//!
//! Order-dependence is *explicit and local*: a key must exist before it is
//! named, which is data dependency and is visible in the text. Attribute order
//! is free — `.within(d).needs(k)` and `.needs(k).within(d)` record the same
//! declaration.

use crate::cx::Release;
use crate::host::bodies::{Bodies, ErasedBlocking, ErasedImport, ErasedPrepare, ErasedRelease};
use crate::key::{Deps, Key, RawKey, Slots};
use crate::plan::{
    next_plan_id, Attrs, Kind, NodeDecl, Plan, PlanIr, Pool, PoolDecl, Template, SEMANTICS,
};
use crate::policy::{CancelMode, Mode, Policy, Restart, Retry, Shutdown};
use crate::validate::{validate, Invalid};
use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;

/// Kind marker: a resource.
pub struct Resource;
/// Kind marker: a step.
pub struct Step;
/// Kind marker: a try-step, whose failure is a value.
pub struct TryStep;
/// Kind marker: a blocking step, parameterised by whether its pool is declared.
#[allow(dead_code)]
pub struct Blocking<P>(PhantomData<fn() -> P>);
/// Typestate: a blocking step with no pool yet. Has no `run`.
pub struct NoPool;
/// Kind marker: a service.
pub struct Service;
/// Kind marker: an effect, parameterised by whether its ambiguity policy is set.
#[allow(dead_code)]
pub struct Effect<A>(PhantomData<fn() -> A>);
/// Typestate: an effect with no ambiguity policy yet. Has no `perform`.
pub struct NoAmbiguity;

/// The untyped state a builder accumulates.
pub(crate) struct Build {
    pub(crate) id: u64,
    pub(crate) name: String,
    pub(crate) nodes: Vec<NodeDecl>,
    pub(crate) pools: Vec<PoolDecl>,
    pub(crate) prepare: Vec<Option<ErasedPrepare>>,
    pub(crate) release: Vec<Option<ErasedRelease>>,
    pub(crate) blocking: Vec<Option<ErasedBlocking>>,
    /// One entry per import node: the plan the value comes from, and the copy.
    pub(crate) imports: Vec<(u64, ErasedImport)>,
    /// The bodies of every component plan used, in declaration order.
    pub(crate) children: Vec<Arc<Bodies>>,
    pub(crate) export: Option<RawKey>,
    pub(crate) input: Option<RawKey>,
    pub(crate) foreign_spawns: Vec<(RawKey, RawKey)>,
}

impl Build {
    fn new(name: &str) -> Self {
        Build {
            id: next_plan_id(),
            name: name.to_string(),
            nodes: Vec::new(),
            pools: Vec::new(),
            prepare: Vec::new(),
            release: Vec::new(),
            blocking: Vec::new(),
            imports: Vec::new(),
            children: Vec::new(),
            export: None,
            input: None,
            foreign_spawns: Vec::new(),
        }
    }

    pub(crate) fn next_key(&self) -> RawKey {
        RawKey {
            plan: self.id,
            idx: self.nodes.len() as u32,
        }
    }

    pub(crate) fn decl(&self, name: &str, kind: Kind) -> NodeDecl {
        NodeDecl {
            key: self.next_key(),
            name: name.to_string(),
            kind,
            needs: Vec::new(),
            source: None,
            spawns: Vec::new(),
            child: None,
            attrs: Attrs::default(),
        }
    }

    pub(crate) fn commit<T: ?Sized>(
        &mut self,
        decl: NodeDecl,
        prepare: Option<ErasedPrepare>,
        release: Option<ErasedRelease>,
        blocking: Option<ErasedBlocking>,
    ) -> Key<T> {
        let raw = decl.key;
        self.nodes.push(decl);
        self.prepare.push(prepare);
        self.release.push(release);
        self.blocking.push(blocking);
        Key::from_raw(raw)
    }
}

/// Builds one plan. Obtained from [`Plan::builder`] or [`Plan::template`].
pub struct PlanBuilder<Out = (), In = ()> {
    pub(crate) b: Build,
    pub(crate) _t: PhantomData<fn() -> (Out, In)>,
}

impl Plan<(), ()> {
    /// Start a root plan or a component plan.
    pub fn builder(name: &str) -> PlanBuilder<(), ()> {
        PlanBuilder {
            b: Build::new(name),
            _t: PhantomData,
        }
    }
}

impl Plan {
    /// Start a template: a plan instantiated at run time with a per-instance
    /// input. A template has no `start`; only `cx.spawn` instantiates it.
    pub fn template<In: Send + Sync + 'static>(name: &str) -> PlanBuilder<(), In> {
        let mut b = Build::new(name);
        let decl = b.decl("input", Kind::Input);
        let key: Key<In> = b.commit(decl, None, None, None);
        b.input = Some(key.raw());
        PlanBuilder { b, _t: PhantomData }
    }
}

impl<Out, In> PlanBuilder<Out, In> {
    /// This builder's plan identity, as it appears in findings.
    pub fn id(&self) -> u64 {
        self.b.id
    }

    /// The template's per-instance input.
    ///
    /// Panics if this plan is not a template: `Plan::builder` plans have no
    /// input, and the type parameter `In = ()` says so.
    pub fn input(&self) -> Key<In> {
        Key::from_raw(self.b.input.expect("only a template plan has an input"))
    }

    /// Name a key of an ancestor plan inside this child plan.
    ///
    /// The instance or component receives the parent's `Arc<T>`, and the
    /// parent's node is released only after this child is cleaned up (INV-16).
    pub fn import<T: ?Sized + Send + Sync + 'static>(&mut self, parent: Key<T>) -> Key<T> {
        let name = format!("import#{}", self.b.nodes.len());
        let mut decl = self.b.decl(&name, Kind::Import);
        let src = parent.raw();
        let me = decl.key;
        decl.source = Some(src);
        // The driver reads a body's needs out of *this* plan's slot table, so
        // the ancestor's value has to arrive in this plan's import slot. The
        // copy is recorded where `T` is still known; everywhere else the value
        // is an erased `Arc<T>` that cannot be cloned without it.
        let copy: ErasedImport = Box::new(move |from: &Slots, to: &mut Slots| {
            if let Some(a) = from.get::<T>(src) {
                to.set::<T>(me, a);
            }
        });
        self.b.imports.push((src.plan, copy));
        self.b.commit(decl, None, None, None)
    }

    /// A resource: acquired once, held, released once.
    pub fn resource(&mut self, name: &str) -> Node<'_, (), Resource> {
        self.node(name, Kind::Resource)
    }

    /// Finite work whose completion is readiness.
    pub fn step(&mut self, name: &str) -> Node<'_, (), Step> {
        self.node(name, Kind::Step)
    }

    /// Finite work whose failure is a value, not a fault.
    pub fn try_step(&mut self, name: &str) -> Node<'_, (), TryStep> {
        self.node(name, Kind::TryStep)
    }

    /// Synchronous work; the pool is required before `run` exists.
    pub fn blocking_step(&mut self, name: &str) -> Node<'_, (), Blocking<NoPool>> {
        self.node(name, Kind::BlockingStep)
    }

    /// A long-lived node whose readiness is the return of its `start` body.
    pub fn service(&mut self, name: &str) -> Node<'_, (), Service> {
        self.node(name, Kind::Service)
    }

    /// An externally visible action; the ambiguity policy is required before
    /// `perform` exists.
    pub fn effect(&mut self, name: &str) -> Node<'_, (), Effect<NoAmbiguity>> {
        self.node(name, Kind::Effect)
    }

    fn node<K>(&mut self, name: &str, kind: Kind) -> Node<'_, (), K> {
        let decl = self.b.decl(name, kind);
        Node {
            b: &mut self.b,
            decl,
            deps: (),
            _k: PhantomData,
        }
    }

    /// Synchronisation only: ready when every need is ready.
    pub fn join<D: Deps>(&mut self, name: &str, deps: D) -> Key<()> {
        let mut decl = self.b.decl(name, Kind::Join);
        decl.needs = deps.raw_keys();
        self.b.commit(decl, None, None, None)
    }

    /// Use a plan as one node of this plan, instantiated once per run.
    pub fn component<O>(&mut self, name: &str, plan: &Plan<O>) -> Key<O> {
        let mut decl = self.b.decl(name, Kind::Component);
        decl.needs = plan.ir.imports();
        decl.child = Some(plan.ir.clone());
        decl.attrs.release = crate::plan::ReleaseStyle::Inner;
        self.b.children.push(plan.bodies.clone());
        self.b.commit(decl, None, None, None)
    }

    /// Register a template so bodies can instantiate it.
    pub fn template<I>(&mut self, name: &str, plan: &Plan<(), I>) -> Template<I> {
        let mut decl = self.b.decl(name, Kind::Template);
        decl.needs = plan.ir.imports();
        decl.child = Some(plan.ir.clone());
        decl.attrs.release = crate::plan::ReleaseStyle::Instances;
        // A template's plan is a child plan like a component's: its nodes have
        // their own key space and their own bodies, which this plan's
        // declaration alone does not carry. `Bodies::children` holds both, in
        // the declaration order of the component and template nodes together.
        self.b.children.push(plan.bodies.clone());
        let key: Key<()> = self.b.commit(decl, None, None, None);
        Template {
            node: key.raw(),
            _i: PhantomData,
        }
    }

    /// Declare that an already-committed service may instantiate a template.
    ///
    /// The chain form (`p.service(..).spawns(&t)`) covers the common case, in
    /// which the template was registered first. This late form exists for the
    /// case the chain form cannot express — a template whose `import` names a
    /// key declared *after* the service — and it is exactly that case that
    /// [`Rule::SpawnSelfImport`](crate::Rule::SpawnSelfImport) rejects.
    ///
    /// Unlike the chain form this takes any key, so the declaration is
    /// **recorded whatever it names** and `build` decides: a node that is not
    /// a service is [`Rule::SpawnKind`](crate::Rule::SpawnKind), and a key of
    /// another plan is [`Rule::ForeignKey`](crate::Rule::ForeignKey). Neither
    /// is silently dropped.
    pub fn spawns<H: ?Sized, I>(&mut self, service: Key<H>, template: &Template<I>) {
        let raw = service.raw();
        match self.b.nodes.iter_mut().find(|n| n.key == raw) {
            Some(node) => node.spawns.push(template.node),
            None => self.b.foreign_spawns.push((raw, template.node)),
        }
    }

    /// Declare a per-run concurrency budget.
    pub fn pool(&mut self, name: &str, limit: usize) -> Pool {
        let idx = self.b.pools.len() as u32;
        self.b.pools.push(PoolDecl {
            name: name.to_string(),
            limit,
        });
        Pool {
            plan: self.b.id,
            idx,
        }
    }

    /// Make a node's value this plan's typed output.
    pub fn export<T>(self, key: Key<T>) -> PlanBuilder<T, In> {
        let mut b = self.b;
        b.export = Some(key.raw());
        PlanBuilder { b, _t: PhantomData }
    }

    /// Validate the declaration and freeze it.
    ///
    /// Policy, shutdown budget and run mode are required arguments: they are
    /// necessary intent, never defaulted. Returns every finding at once.
    pub fn build(
        self,
        policy: Policy,
        shutdown: Shutdown,
        mode: Mode,
    ) -> Result<Plan<Out, In>, Invalid> {
        let id = self.b.id;
        let ir = PlanIr {
            id,
            name: self.b.name,
            semantics: SEMANTICS,
            nodes: self.b.nodes,
            pools: self.b.pools,
            policy,
            shutdown,
            mode,
            export: self.b.export,
            input: self.b.input,
            foreign_spawns: self.b.foreign_spawns,
        };
        let checks = validate(&ir);
        if !checks.is_empty() {
            return Err(Invalid { checks });
        }
        Ok(Plan {
            ir: Arc::new(ir),
            bodies: Arc::new(Bodies {
                plan: id,
                prepare: self.b.prepare,
                release: self.b.release,
                blocking: self.b.blocking,
                imports: self.b.imports,
                children: self.b.children,
            }),
            _t: PhantomData,
        })
    }
}

/// A node under construction: `D` is what it needs, `K` is its kind.
///
/// The node joins the plan only when its terminal method runs, which is what
/// makes "a resource with no release" and "an effect with no ambiguity policy"
/// compile errors rather than findings.
pub struct Node<'b, D: Deps, K> {
    pub(crate) b: &'b mut Build,
    pub(crate) decl: NodeDecl,
    pub(crate) deps: D,
    pub(crate) _k: PhantomData<fn() -> K>,
}

impl<'b, D: Deps, K> Node<'b, D, K> {
    fn declare(&mut self, attr: &'static str) {
        self.decl.attrs.declared.push(attr);
    }

    /// Declare this node's dependencies: ordering **and** typed dataflow.
    pub fn needs<D2: Deps>(self, deps: D2) -> Node<'b, D2, K> {
        let mut decl = self.decl;
        decl.needs = deps.raw_keys();
        Node {
            b: self.b,
            decl,
            deps,
            _k: PhantomData,
        }
    }

    /// Bound the prepare/run body.
    pub fn within(mut self, d: Duration) -> Self {
        self.declare("within");
        self.decl.attrs.within = Some(d);
        self
    }

    /// Re-execute the prepare body on failure.
    pub fn retry(mut self, r: Retry) -> Self {
        self.declare("retry");
        self.decl.attrs.retry = Some(r);
        self
    }

    /// Assert that re-execution of this node is safe. Trusted, not verified.
    pub fn idempotent(mut self) -> Self {
        self.declare("idempotent");
        self.decl.attrs.idempotent = true;
        self
    }

    /// Take a resource this node already needs exclusively.
    pub fn exclusive<T: ?Sized>(mut self, res: Key<T>) -> Self {
        self.decl.attrs.exclusive.push(res.raw());
        self
    }

    /// Take a resource this node already needs in shared mode.
    pub fn shared<T: ?Sized>(mut self, res: Key<T>) -> Self {
        self.decl.attrs.shared.push(res.raw());
        self
    }

    /// Bound this node's concurrency with a pool.
    pub fn limit(mut self, pool: Pool) -> Self {
        self.declare("limit");
        self.decl.attrs.limit = Some(pool);
        self
    }

    /// Cancel by signal-then-deadline instead of an immediate drop, so an
    /// in-flight `hold` can still complete and register.
    pub fn cooperative(mut self, grace: Duration) -> Self {
        self.declare("cancel");
        self.decl.attrs.cancel = CancelMode::Cooperative(grace);
        self
    }
}

impl<'b, D: Deps> Node<'b, D, Service> {
    /// Bound this service's stop.
    pub fn stop_within(mut self, d: Duration) -> Self {
        self.declare("stop_within");
        self.decl.attrs.stop_within = Some(d);
        self
    }

    /// Restart this service when its serve future returns `Err`.
    pub fn restart(mut self, r: Restart) -> Self {
        self.declare("restart");
        self.decl.attrs.restart = Some(r);
        self
    }

    /// This service finishing ends the scope.
    pub fn terminal(mut self) -> Self {
        self.declare("terminal");
        self.decl.attrs.terminal = true;
        self
    }

    /// Declare a template this service may instantiate (F1).
    ///
    /// A service may spawn only templates it declares, and the declaration is
    /// visible in `inspect()`.
    pub fn spawns<I>(mut self, template: &Template<I>) -> Self {
        self.decl.spawns.push(template.node);
        self
    }
}

/// Explicit RAII release.
pub mod release {
    use super::*;
    use std::future::Future;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    /// The future `by_drop` returns. Its type is how the builder records that
    /// the author chose RAII rather than an async body.
    pub struct DropRelease;

    impl Future for DropRelease {
        type Output = Result<(), crate::Error>;
        fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
            Poll::Ready(Ok(()))
        }
    }

    /// "Release by dropping the value": the one-token way to say RAII.
    ///
    /// `inspect()` prints `release: drop` for a node declared this way, so the
    /// choice is visible rather than implied by an omission.
    pub fn by_drop<T: ?Sized>(
    ) -> impl Fn(Cx<Release>, Arc<T>) -> DropRelease + Send + Sync + 'static {
        |_cx, _v| DropRelease
    }

    use crate::cx::Cx;
}
