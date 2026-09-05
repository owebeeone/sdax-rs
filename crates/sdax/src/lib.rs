//! `sdax` — a declarative async lifecycle, as a value.
//!
//! A lifecycle is declared as an **acquisition graph**: typed nodes (resources,
//! steps, services, effects, joins, components, templates) joined by typed
//! `needs` edges whose keys are the values the node constructors return. The
//! engine derives everything about time from that graph — per-node start
//! eligibility with no wave barriers, the release graph (the reverse of `needs`,
//! with concurrency wherever no edge forbids it), cancellation propagation,
//! deadline budgeting, and the explanation of every wait.
//!
//! Three decisions carry the correctness envelope:
//!
//! 1. A resource or effect body must *return* a [`Held<T>`], which only
//!    [`Cx::hold`] and [`Cx::hold_value`] can mint. `hold` registers the value
//!    in the same poll that observes the effect completing, so there is no await
//!    point between "the effect happened" and "the engine owns the cleanup".
//! 2. A service is ready when its `start` body *returns* a [`Serving`]. "Ready
//!    because it was spawned" has no spelling.
//! 3. Policy that is necessary intent — the fail policy, the shutdown budget,
//!    the run mode, and what to do about an effect whose outcome is unknown — is
//!    a required argument or a required typestate step, never a default.
//!
//! # Stage 0 has no execution
//!
//! This version is the **contract, the validator and the seam**. It has no
//! state machine, no `Plan::start` and no `Running` — and no stub standing in
//! for them, because a stub would be a claim the crate cannot make. What it
//! does today: build a plan, reject an invalid one with findings that name the
//! rule, and answer questions about a plan before anything runs.
//! `dev-docs/SdaxContract-v1.md` is normative and marks every deferred row.
//!
//! ```
//! use sdax::*;
//! use std::sync::Arc;
//! use std::time::Duration;
//!
//! struct Transport;
//! struct PeerStore;
//! struct Receipt;
//!
//! let mut p = Plan::builder("Startup");
//! let transport = p
//!     .resource("Transport")
//!     .acquire(|cx, ()| async move { Ok(cx.hold_value(Transport)) })
//!     .release(|_cx, _t| async move { Ok(()) });
//! let peers = p
//!     .resource("PeerStore")
//!     .needs(transport)
//!     .acquire(|cx, _t: Arc<Transport>| async move { Ok(cx.hold_value(PeerStore)) })
//!     .release(release::by_drop());
//! p.effect("Registration")
//!     .needs(peers)
//!     .on_ambiguous(Ambiguity::Report)
//!     .perform(|cx, _s: Arc<PeerStore>| async move { Ok(cx.hold_value(Receipt)) })
//!     .compensate(|_cx, _r| async move { Ok(()) });
//!
//! let plan = p.build(
//!     Policy::FailFast,
//!     Shutdown::within(Duration::from_secs(10)),
//!     Mode::Finite,
//! )?;
//!
//! // Pure: no body runs, and nothing ships.
//! let view = plan.inspect();
//! assert_eq!(view.layers().len(), 3);
//! assert!(view.release_order().before("Registration", "Transport"));
//! println!("{view}");
//! # Ok::<(), Invalid>(())
//! ```
//!
//! # The one rule of the body contract
//!
//! Perform external effects *inside* `cx.hold(..)`. A body that performs the
//! effect itself, awaits something else, and then calls [`Cx::hold_value`] has
//! re-created by hand the window `hold` exists to close. The engine cannot see
//! that; it is the seam's stated residue, not a promise.
//!
//! A service's own acquisitions belong in a resource node rather than in its
//! `start` body: a value a start body creates has no ledger entry and no async
//! release.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod builder;
pub mod compile_fail;
mod contracts;
mod cx;
pub mod engine;
mod key;
mod plan;
mod policy;
mod report;
mod shorthand;
mod terminals;
mod validate;
mod view;

pub use builder::{
    release, Blocking, Effect, NoAmbiguity, NoPool, Node, PlanBuilder, Resource, Service, Step,
    TryStep,
};
pub use contracts::{
    BoxFuture, Clock, Error, Joined, NoObserver, Observer, Runtime, TaskHandle, Time,
};
pub use cx::{
    Acquire, Child, ChildControl, Cx, CxInner, Held, Hold, InstanceId, Release, Run, Scope,
    Serving, SpawnError, Start, Stop, StopSignal, Timeout,
};
pub use engine::{JoinedLabel, TimerId};
pub use key::{Deps, Key, RawKey, Slots};
pub use plan::{Kind, Plan, Pool, ReleaseStyle, Template, SEMANTICS};
pub use policy::{Ambiguity, Backoff, CancelMode, Mode, Policy, Restart, Retry, Shutdown};
pub use report::{
    Fault, FaultKind, FaultLabel, NodeRecord, Outcome, Phase, RecordOrder, Report, Trace,
    TraceEvent, TraceKind,
};
pub use terminals::{NeedsCompensate, NeedsRelease};
pub use validate::{Finding, Invalid, Rule};
pub use view::{
    AttrChange, Edge, Effects, NodePath, NodeView, PlanDiff, PlanView, PoolView, Reason,
    ReleaseOrder, Why,
};

/// Everything an author needs to write a plan, plus `Duration`.
pub mod prelude {
    pub use crate::builder::{release, PlanBuilder};
    pub use crate::contracts::{Clock, Error, Observer, Runtime, Time};
    pub use crate::cx::{Acquire, Child, Cx, Held, Release, Run, Serving, Start};
    pub use crate::key::{Deps, Key};
    pub use crate::plan::{Plan, Pool, Template};
    pub use crate::policy::{Ambiguity, Backoff, Mode, Policy, Restart, Retry, Shutdown};
    pub use crate::report::{Outcome, Report};
    pub use crate::validate::{Finding, Invalid, Rule};
    pub use crate::view::{PlanView, Why};
    pub use std::time::Duration;
}

#[cfg(test)]
mod tests;
