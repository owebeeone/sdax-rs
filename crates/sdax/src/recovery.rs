//! Typed per-run operation identities and receipt-less recovery.
use crate::builder::{Effect, Node};
use crate::host::bodies::ErasedRelease;
use crate::host::Slots;
use crate::terminals::NeedsCompensate;
use crate::{Acquire, Ambiguity, Cx, Deps, Error, Held, Key, Release};
use std::future::Future;
use std::sync::Arc;

/// Result of reconciling an operation whose success was never acknowledged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recovery {
    /// The operation is reconciled and its unknown-outcome obligation is discharged.
    Resolved,
    /// Its external outcome remains uncertain; preserve the unresolved report entry.
    StillUnknown,
}

/// Recovery completed without resolving the operation's external outcome.
#[derive(Debug)]
pub struct UnresolvedRecovery;
impl std::fmt::Display for UnresolvedRecovery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("recovery completed but the operation's outcome remains unknown")
    }
}
impl std::error::Error for UnresolvedRecovery {}

/// Dependency plumbing pairing ordinary dependencies with operation identity.
pub struct Identified<D, I: ?Sized> {
    deps: D,
    identity: Key<I>,
}
impl<D: Copy, I: ?Sized> Copy for Identified<D, I> {}
impl<D: Copy, I: ?Sized> Clone for Identified<D, I> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<D: Deps, I: ?Sized + Send + Sync + 'static> Deps for Identified<D, I> {
    type Out = (D::Out, Arc<I>);
    fn raw_keys(&self) -> Vec<crate::host::RawKey> {
        let mut keys = self.deps.raw_keys();
        if !keys.contains(&self.identity.raw()) {
            keys.push(self.identity.raw());
        }
        keys
    }
    fn fetch(&self, slots: &Slots) -> Option<Self::Out> {
        Some((self.deps.fetch(slots)?, slots.get(self.identity.raw())?))
    }
}

/// An effect with identity recorded in a typed run slot before execution.
#[must_use = "complete the effect with perform, recovery and compensation or persistence"]
pub struct IdentifiedEffect<'b, D: Deps, I: ?Sized + Send + Sync + 'static> {
    node: Node<'b, Identified<D, I>, Effect<Ambiguity>>,
    identity: Key<I>,
}
impl<'b, D: Deps> Node<'b, D, Effect<Ambiguity>> {
    /// Bind an operation identity available before external work. The perform
    /// body receives `(dependencies, Arc<Identity>)`. This key is a dependency,
    /// so it is initialized per run and stays available throughout recovery.
    /// Retries read the same slot and receive separate `cx.attempt()` numbers.
    pub fn identified_by<I: ?Sized + Send + Sync + 'static>(
        self,
        identity: Key<I>,
    ) -> IdentifiedEffect<'b, D, I> {
        let deps = Identified {
            deps: self.deps,
            identity,
        };
        let mut decl = self.decl;
        decl.needs = deps.raw_keys();
        IdentifiedEffect {
            node: Node {
                b: self.b,
                decl,
                deps,
                _k: std::marker::PhantomData,
            },
            identity,
        }
    }
}
impl<'b, D: Deps, I: ?Sized + Send + Sync + 'static> IdentifiedEffect<'b, D, I> {
    /// Perform with single-use acquisition authority and the bound operation identity.
    pub fn perform<F, Fut, R>(self, f: F) -> IdentifiedCompensate<'b, R, I>
    where
        F: Fn(Cx<Acquire>, (D::Out, Arc<I>)) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Held<R>, Error>> + Send + 'static,
        R: ?Sized + Send + Sync + 'static,
    {
        IdentifiedCompensate {
            inner: self.node.perform(f),
            identity: self.identity,
        }
    }
}

/// An identified effect whose explicit recovery handler must still be supplied.
#[must_use = "install recover_unknown before choosing compensation or persistence"]
pub struct IdentifiedCompensate<'b, R: ?Sized, I: ?Sized> {
    inner: NeedsCompensate<'b, R>,
    identity: Key<I>,
}
impl<'b, R: ?Sized + Send + Sync + 'static, I: ?Sized + Send + Sync + 'static>
    IdentifiedCompensate<'b, R, I>
{
    /// Recover using the recorded operation identity, never a fabricated receipt.
    /// Runs shielded by the cleanup budget after the interrupted body is joined.
    /// `StillUnknown`, errors, panics and expiry retain the unresolved record.
    pub fn recover_unknown<F, Fut>(mut self, f: F) -> NeedsCompensate<'b, R>
    where
        F: Fn(Cx<Release>, Arc<I>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Recovery, Error>> + Send + 'static,
    {
        let identity = self.identity;
        let f = Arc::new(f);
        let recovery: ErasedRelease = Box::new(move |inner, slots| {
            let id = slots.get::<I>(identity.raw())?;
            let f = f.clone();
            Some(Box::pin(async move {
                match f(Cx::new(inner), id).await? {
                    Recovery::Resolved => Ok(()),
                    Recovery::StillUnknown => Err(Box::new(UnresolvedRecovery) as Error),
                }
            }))
        });
        self.inner.recovery = Some(recovery);
        self.inner.decl.attrs.recovery = true;
        self.inner
    }
}
