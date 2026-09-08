//! Single-use acquisition authority and poll-atomic registration.
use super::*;

// ---------------------------------------------------------------- held values

/// Proof that the engine owns the release (or compensation) obligation for a
/// value. No public constructor: only [`Cx::hold`] and [`Cx::hold_value`] mint
/// one, and both write the engine's record before handing it back.
pub struct Held<T: ?Sized> {
    arc: Arc<T>,
    node: RawKey,
}

impl<T: ?Sized> Held<T> {
    /// Which node registered this value.
    pub fn node(&self) -> RawKey {
        self.node
    }
}

impl<T: ?Sized> std::ops::Deref for Held<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.arc
    }
}

impl<T: ?Sized> std::fmt::Debug for Held<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Held(node {}/{})", self.node.plan, self.node.idx)
    }
}

impl Cx<Acquire> {
    /// Consume this attempt's acquisition authority and invoke a lazy factory.
    /// The reservation happens before the factory runs. A repeated host call
    /// returns an error without invoking the rejected factory. Keep a `shared()`
    /// context first if the continuation needs clock or cancellation operations.
    ///
    /// The poll that observes `effect` completing writes the engine's record
    /// and *then* returns `Ready`, so no await point exists between the effect
    /// and the obligation. Cancellation can only take effect between polls, so
    /// the X1 window ("acquired, but nobody owns the cleanup") does not exist
    /// in this form. Nothing is registered while the effect is pending, and
    /// nothing is registered when it fails.
    pub fn hold<F, V, E, T>(self, factory: impl FnOnce() -> F) -> Hold<F, T>
    where
        F: Future<Output = Result<V, E>> + Send + 'static,
        V: Into<Arc<T>>,
        E: Into<Error>,
        T: ?Sized + Send + Sync + 'static,
    {
        let effect = self.inner.reserve().map(|()| Box::pin(factory()));
        Hold {
            effect,
            cx: self.inner.clone(),
            _t: PhantomData,
        }
    }

    /// Register a value that carries no external effect — a computed handle, a
    /// boxed adapter, a value the caller already owns.
    ///
    /// Registration happens before this returns. Performing an external effect
    /// outside the wrapper and registering it afterwards re-creates the X1
    /// window by hand; that is escape misuse, not the contract.
    pub fn hold_value<T: ?Sized + Send + Sync + 'static>(self, v: impl Into<Arc<T>>) -> Held<T> {
        self.inner
            .reserve()
            .expect("host reused acquisition authority");
        let arc: Arc<T> = v.into();
        self.inner.register(arc.clone());
        Held {
            arc,
            node: self.inner.node,
        }
    }
}

/// The future returned by [`Cx::hold`]. Registration happens inside `poll`.
pub struct Hold<F, T: ?Sized> {
    effect: Result<Pin<Box<F>>, Error>,
    cx: Arc<CxInner>,
    _t: PhantomData<fn() -> Arc<T>>,
}

impl<F, V, E, T> Future for Hold<F, T>
where
    F: Future<Output = Result<V, E>> + Send + 'static,
    V: Into<Arc<T>>,
    E: Into<Error>,
    T: ?Sized + Send + Sync + 'static,
{
    type Output = Result<Held<T>, Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let effect = match &mut self.effect {
            Ok(effect) => effect,
            Err(_) => return Poll::Ready(Err(Box::new(AlreadyAcquired))),
        };
        match effect.as_mut().poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(e)) => Poll::Ready(Err(e.into())),
            Poll::Ready(Ok(v)) => {
                let arc: Arc<T> = v.into();
                // Same poll as the completion: no await between effect and record.
                self.cx.register(arc.clone());
                let node = self.cx.node;
                Poll::Ready(Ok(Held { arc, node }))
            }
        }
    }
}
