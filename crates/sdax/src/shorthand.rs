//! Positional constructors: sugar over the chain form (adoption A1).
//!
//! `p.resource_with("T", deps, acquire, release)` is *definitionally*
//! `p.resource("T").needs(deps).acquire(acquire).release(release)` — the same
//! recorded declaration, witnessed by a test that compares the `inspect()`
//! output of both forms. They exist because the chain form costs about eight
//! tokens per node, and most nodes carry no attributes at all.
//!
//! A node that needs an attribute uses the chain form; the two mix freely in
//! one plan.

use crate::builder::PlanBuilder;
use crate::contracts::Error;
use crate::cx::{Acquire, Cx, Held, Release, Run, Serving, Start};
use crate::key::{Deps, Key};
use crate::plan::Pool;
use crate::policy::Ambiguity;
use std::future::Future;
use std::sync::Arc;

impl<Out, In> PlanBuilder<Out, In> {
    /// `resource(name).needs(deps).acquire(a).release(r)`.
    pub fn resource_with<D, A, AFut, R, RFut, T>(
        &mut self,
        name: &str,
        deps: D,
        acquire: A,
        release: R,
    ) -> Key<T>
    where
        D: Deps,
        A: Fn(Cx<Acquire>, D::Out) -> AFut + Send + Sync + 'static,
        AFut: Future<Output = Result<Held<T>, Error>> + Send + 'static,
        R: Fn(Cx<Release>, Arc<T>) -> RFut + Send + Sync + 'static,
        RFut: Future<Output = Result<(), Error>> + Send + 'static,
        T: ?Sized + Send + Sync + 'static,
    {
        self.resource(name)
            .needs(deps)
            .acquire(acquire)
            .release(release)
    }

    /// `step(name).needs(deps).run(f)`.
    pub fn step_with<D, F, Fut, T>(&mut self, name: &str, deps: D, run: F) -> Key<T>
    where
        D: Deps,
        F: Fn(Cx<Run>, D::Out) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<T, Error>> + Send + 'static,
        T: Send + Sync + 'static,
    {
        self.step(name).needs(deps).run(run)
    }

    /// `try_step(name).needs(deps).run(f)`.
    pub fn try_step_with<D, F, Fut, T>(
        &mut self,
        name: &str,
        deps: D,
        run: F,
    ) -> Key<Result<T, Error>>
    where
        D: Deps,
        F: Fn(Cx<Run>, D::Out) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<T, Error>> + Send + 'static,
        T: Send + Sync + 'static,
    {
        self.try_step(name).needs(deps).run(run)
    }

    /// `blocking_step(name).needs(deps).on(pool).run(f)`.
    pub fn blocking_step_with<D, F, T>(&mut self, name: &str, deps: D, pool: Pool, run: F) -> Key<T>
    where
        D: Deps,
        F: Fn(Cx<Run>, D::Out) -> Result<T, Error> + Send + Sync + 'static,
        T: Send + Sync + 'static,
    {
        self.blocking_step(name).needs(deps).on(pool).run(run)
    }

    /// `service(name).needs(deps).start(f)`.
    ///
    /// A service declared this way has no `stop_within`, so the scope's
    /// shutdown budget bounds its stop; `Shutdown::unbounded()` then rejects
    /// the plan ([`Rule::ServiceUnbounded`](crate::Rule::ServiceUnbounded)).
    pub fn service_with<D, F, Fut, H>(&mut self, name: &str, deps: D, start: F) -> Key<H>
    where
        D: Deps,
        F: Fn(Cx<Start>, D::Out) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Serving<H>, Error>> + Send + 'static,
        H: Send + Sync + 'static,
    {
        self.service(name).needs(deps).start(start)
    }

    /// `effect(name).needs(deps).on_ambiguous(a).perform(p).compensate(c)`.
    ///
    /// `on_ambiguous` is positional rather than optional: it is required in
    /// both forms.
    pub fn effect_with<D, P, PFut, C, CFut, R>(
        &mut self,
        name: &str,
        deps: D,
        on_ambiguous: Ambiguity,
        perform: P,
        compensate: C,
    ) -> Key<R>
    where
        D: Deps,
        P: Fn(Cx<Acquire>, D::Out) -> PFut + Send + Sync + 'static,
        PFut: Future<Output = Result<Held<R>, Error>> + Send + 'static,
        C: Fn(Cx<Release>, Arc<R>) -> CFut + Send + Sync + 'static,
        CFut: Future<Output = Result<(), Error>> + Send + 'static,
        R: ?Sized + Send + Sync + 'static,
    {
        self.effect(name)
            .needs(deps)
            .on_ambiguous(on_ambiguous)
            .perform(perform)
            .compensate(compensate)
    }

    /// `effect(name).needs(deps).on_ambiguous(a).perform(p).persistent()`.
    pub fn effect_persistent_with<D, P, PFut, R>(
        &mut self,
        name: &str,
        deps: D,
        on_ambiguous: Ambiguity,
        perform: P,
    ) -> Key<R>
    where
        D: Deps,
        P: Fn(Cx<Acquire>, D::Out) -> PFut + Send + Sync + 'static,
        PFut: Future<Output = Result<Held<R>, Error>> + Send + 'static,
        R: ?Sized + Send + Sync + 'static,
    {
        self.effect(name)
            .needs(deps)
            .on_ambiguous(on_ambiguous)
            .perform(perform)
            .persistent()
    }
}
