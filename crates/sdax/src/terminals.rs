//! The terminal methods, one per kind.
//!
//! A terminal completes a reserved declaration and hands back its [`Key`].
//! Type state prevents using an incomplete chain as a key: a resource without
//! `release` never yields a `Key`, an effect without `on_ambiguous` has no
//! `perform`, and a blocking step without `on` has no `run`. Discarding an
//! incomplete chain leaves a structured finding that prevents plan build.

use crate::builder::Build;
use crate::builder::{
    release, Blocking, Effect, NoAmbiguity, NoPool, Node, Resource, Service, Step, TryStep,
};
use crate::contracts::Error;
use crate::cx::{Acquire, Cx, Held, Release, Run, ServingPhase, Start};
use crate::host::bodies::{ErasedBlocking, ErasedPrepare, ErasedRelease, ErasedServe};
use crate::key::{Deps, Key};
use crate::plan::{NodeDecl, Pool, ReleaseStyle};
use crate::policy::Ambiguity;
use std::any::TypeId;
use std::future::Future;
use std::marker::PhantomData;
use std::sync::Arc;

fn is_by_drop<Fut: 'static>() -> bool {
    TypeId::of::<Fut>() == TypeId::of::<release::DropRelease>()
}

// ---------------------------------------------------------------- resource

impl<'b, D: Deps> Node<'b, D, Resource> {
    /// Acquire the resource. The body must return `Held<T>`, which only
    /// `cx.hold(..)` or `cx.hold_value(..)` can mint, so "acquired but never
    /// registered" does not type-check.
    pub fn acquire<F, Fut, T>(self, f: F) -> NeedsRelease<'b, T>
    where
        F: Fn(Cx<Acquire>, D::Out) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Held<T>, Error>> + Send + 'static,
        T: ?Sized + Send + Sync + 'static,
    {
        let deps = self.deps;
        let f = Arc::new(f);
        let prepare: ErasedPrepare = Box::new(move |inner, slots| {
            let d = deps.fetch(slots)?;
            let f = f.clone();
            Some(Box::pin(async move {
                let fut = f(Cx::<Acquire>::new(inner), d);
                // The engine already owns the Arc: `hold` registered it.
                let _held = fut.await?;
                Ok(())
            }))
        });
        NeedsRelease {
            b: self.b,
            decl: self.decl,
            prepare,
            _t: PhantomData,
        }
    }
}

/// A resource with an acquire body and no release yet. Not a [`Key`].
#[must_use = "complete this resource with `.release(..)` before building the plan"]
pub struct NeedsRelease<'b, T: ?Sized> {
    b: &'b mut Build,
    decl: NodeDecl,
    prepare: ErasedPrepare,
    _t: PhantomData<fn() -> Arc<T>>,
}

impl<'b, T: ?Sized + Send + Sync + 'static> NeedsRelease<'b, T> {
    /// Release the resource. `release::by_drop()` is the explicit RAII choice
    /// and is recorded as `release: drop`.
    pub fn release<F, Fut>(self, f: F) -> Key<T>
    where
        F: Fn(Cx<Release>, Arc<T>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), Error>> + Send + 'static,
    {
        let me = self.decl.key;
        let mut decl = self.decl;
        decl.attrs.release = if is_by_drop::<Fut>() {
            ReleaseStyle::Drop
        } else {
            ReleaseStyle::Async
        };
        let f = Arc::new(f);
        let release: ErasedRelease = Box::new(move |inner, slots| {
            let arc = slots.get::<T>(me)?;
            let f = f.clone();
            Some(Box::pin(
                async move { f(Cx::<Release>::new(inner), arc).await },
            ))
        });
        self.b.commit(decl, Some(self.prepare), Some(release), None)
    }
}

// ---------------------------------------------------------------- steps

impl<'b, D: Deps> Node<'b, D, Step> {
    /// Run finite work. Returning `Ok` is readiness; there is nothing to
    /// release.
    pub fn run<F, Fut, T>(self, f: F) -> Key<T>
    where
        F: Fn(Cx<Run>, D::Out) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<T, Error>> + Send + 'static,
        T: Send + Sync + 'static,
    {
        let deps = self.deps;
        let f = Arc::new(f);
        let prepare: ErasedPrepare = Box::new(move |inner, slots| {
            let d = deps.fetch(slots)?;
            let f = f.clone();
            Some(Box::pin(async move {
                let out = f(Cx::<Run>::new(inner.clone()), d).await?;
                inner.put_output(Box::new(Arc::new(out)));
                Ok(())
            }))
        });
        self.b.commit(self.decl, Some(prepare), None, None)
    }
}

impl<'b, D: Deps> Node<'b, D, TryStep> {
    /// Run finite work whose failure is a value: dependents receive
    /// `Arc<Result<T, Error>>` and the run does not fault.
    ///
    /// A try-step with no dependent would swallow its failure, which
    /// [`Rule::TryUnconsumed`](crate::Rule::TryUnconsumed) rejects.
    pub fn run<F, Fut, T>(self, f: F) -> Key<Result<T, Error>>
    where
        F: Fn(Cx<Run>, D::Out) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<T, Error>> + Send + 'static,
        T: Send + Sync + 'static,
    {
        let deps = self.deps;
        let f = Arc::new(f);
        let prepare: ErasedPrepare = Box::new(move |inner, slots| {
            let d = deps.fetch(slots)?;
            let f = f.clone();
            Some(Box::pin(async move {
                let out = f(Cx::<Run>::new(inner.clone()), d).await;
                inner.put_output(Box::new(Arc::new(out)));
                Ok(())
            }))
        });
        self.b.commit(self.decl, Some(prepare), None, None)
    }
}

impl<'b, D: Deps> Node<'b, D, Blocking<NoPool>> {
    /// Put this blocking step on a pool. Required: an unbounded blocking step
    /// is a thread leak, so `run` does not exist before this.
    pub fn on(self, pool: Pool) -> Node<'b, D, Blocking<Pool>> {
        let mut decl = self.decl;
        decl.attrs.pool = Some(pool);
        Node {
            b: self.b,
            decl,
            deps: self.deps,
            _k: PhantomData,
        }
    }
}

impl<'b, D: Deps> Node<'b, D, Blocking<Pool>> {
    /// Run synchronous work on the declared pool.
    pub fn run<F, T>(self, f: F) -> Key<T>
    where
        F: Fn(Cx<Run>, D::Out) -> Result<T, Error> + Send + Sync + 'static,
        T: Send + Sync + 'static,
    {
        let deps = self.deps;
        let f = Arc::new(f);
        let blocking: ErasedBlocking = Box::new(move |inner, slots| {
            let d = deps.fetch(slots)?;
            let cx = Cx::<Run>::new(inner.clone());
            let f = f.clone();
            Some(Box::new(move || {
                let out = f(cx, d)?;
                inner.put_output(Box::new(Arc::new(out)));
                Ok(())
            }))
        });
        self.b.commit(self.decl, None, None, Some(blocking))
    }
}

// ---------------------------------------------------------------- service

impl<'b, D: Deps> Node<'b, D, Service> {
    /// Initialize the service's stable handle once per run.
    ///
    /// The successful return publishes one `Arc<H>` to dependents. A declared
    /// [`Retry`](crate::Retry) may repeat this body before that first success;
    /// serving recovery never invokes it again.
    pub fn initialize<F, Fut, H>(self, f: F) -> NeedsServe<'b, H>
    where
        F: Fn(Cx<Start>, D::Out) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<H, Error>> + Send + 'static,
        H: Send + Sync + 'static,
    {
        let deps = self.deps;
        let f = Arc::new(f);
        let mut decl = self.decl;
        decl.attrs.release = ReleaseStyle::Stop;
        let prepare: ErasedPrepare = Box::new(move |inner, slots| {
            let d = deps.fetch(slots)?;
            let f = f.clone();
            Some(Box::pin(async move {
                let handle = f(Cx::<Start>::new(inner.clone()), d).await?;
                inner.put_output(Box::new(Arc::new(handle)));
                Ok(())
            }))
        });
        NeedsServe {
            b: self.b,
            decl,
            prepare,
            _h: PhantomData,
        }
    }
}

/// A service with an initializer whose restartable serving factory is not yet
/// declared. It joins the plan only when [`NeedsServe::serve`] is called.
#[must_use = "a service joins the plan only when `.serve(..)` is called"]
pub struct NeedsServe<'b, H> {
    b: &'b mut Build,
    decl: NodeDecl,
    prepare: ErasedPrepare,
    _h: PhantomData<fn() -> Arc<H>>,
}

impl<'b, H: Send + Sync + 'static> NeedsServe<'b, H> {
    /// Declare the restartable serving episode factory.
    ///
    /// Episode 1 starts after initialization publishes the handle. Recovery
    /// invokes this factory again with a fresh context and the same `Arc<H>`;
    /// it never re-runs initialization or dependent bodies.
    pub fn serve<F, Fut>(self, f: F) -> Key<H>
    where
        F: Fn(Cx<ServingPhase>, Arc<H>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), Error>> + Send + 'static,
    {
        let me = self.decl.key;
        let f = Arc::new(f);
        let serve: ErasedServe = Box::new(move |inner, slots| {
            let handle = slots.get::<H>(me)?;
            let f = f.clone();
            Some(Box::pin(async move {
                f(Cx::<ServingPhase>::new(inner), handle).await
            }))
        });
        self.b.commit_service(self.decl, self.prepare, serve)
    }
}

// ---------------------------------------------------------------- effect

impl<'b, D: Deps> Node<'b, D, Effect<NoAmbiguity>> {
    /// Say what happens when this effect's outcome is unknown. Required:
    /// `perform` does not exist before it.
    pub fn on_ambiguous(self, a: Ambiguity) -> Node<'b, D, Effect<Ambiguity>> {
        let mut decl = self.decl;
        decl.attrs.declared.push("on_ambiguous");
        decl.attrs.on_ambiguous = Some(a);
        Node {
            b: self.b,
            decl,
            deps: self.deps,
            _k: PhantomData,
        }
    }
}

impl<'b, D: Deps> Node<'b, D, Effect<Ambiguity>> {
    /// Perform the effect. Like `acquire`, the body must return `Held<R>`: the
    /// receipt is registered in the poll that observes the effect completing.
    pub fn perform<F, Fut, R>(self, f: F) -> NeedsCompensate<'b, R>
    where
        F: Fn(Cx<Acquire>, D::Out) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Held<R>, Error>> + Send + 'static,
        R: ?Sized + Send + Sync + 'static,
    {
        let deps = self.deps;
        let f = Arc::new(f);
        let prepare: ErasedPrepare = Box::new(move |inner, slots| {
            let d = deps.fetch(slots)?;
            let f = f.clone();
            Some(Box::pin(async move {
                let fut = f(Cx::<Acquire>::new(inner), d);
                let _held = fut.await?;
                Ok(())
            }))
        });
        NeedsCompensate {
            b: self.b,
            decl: self.decl,
            prepare,
            recovery: None,
            _t: PhantomData,
        }
    }
}

/// An effect with a `perform` body that has not yet said what happens to the
/// record at shutdown. Not a [`Key`].
#[must_use = "an effect joins the plan only when `.compensate(..)` or `.persistent()` is called"]
pub struct NeedsCompensate<'b, R: ?Sized> {
    pub(crate) b: &'b mut Build,
    pub(crate) decl: NodeDecl,
    prepare: ErasedPrepare,
    pub(crate) recovery: Option<ErasedRelease>,
    _t: PhantomData<fn() -> Arc<R>>,
}

impl<'b, R: ?Sized + Send + Sync + 'static> NeedsCompensate<'b, R> {
    /// Undo the effect at cleanup. Reported distinctly from a release.
    pub fn compensate<F, Fut>(self, f: F) -> Key<R>
    where
        F: Fn(Cx<Release>, Arc<R>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), Error>> + Send + 'static,
    {
        let me = self.decl.key;
        let mut decl = self.decl;
        decl.attrs.release = ReleaseStyle::Compensate;
        let recovery = self.recovery;
        let f = Arc::new(f);
        let release: ErasedRelease = Box::new(move |inner, slots| {
            if inner.is_recovery() {
                return recovery.as_ref()?(inner, slots);
            }
            let arc = slots.get::<R>(me)?;
            let f = f.clone();
            Some(Box::pin(
                async move { f(Cx::<Release>::new(inner), arc).await },
            ))
        });
        self.b.commit(decl, Some(self.prepare), Some(release), None)
    }

    /// Declare that this effect carries no compensation obligation: the record
    /// stands, and shutdown never undoes it (F2).
    ///
    /// The node is still an effect. It is listed by
    /// [`Plan::effects`](crate::Plan::effects) and shown by `inspect()` as
    /// `effect (persistent)`, which is the point: a persisting write that was
    /// modelled as a step used to vanish from the ship-boundary listing
    /// (`AdversarialReview.md` F-A3/F-B3). `on_ambiguous` is still required.
    pub fn persistent(self) -> Key<R> {
        let mut decl = self.decl;
        decl.attrs.release = ReleaseStyle::Persistent;
        self.b.commit(decl, Some(self.prepare), self.recovery, None)
    }
}
