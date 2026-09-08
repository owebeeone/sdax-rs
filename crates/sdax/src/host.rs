//! The host surface: what a runtime adapter, a run driver or the engine needs,
//! and an author does not.
//!
//! **Not the author API, and not covered by the crate's stability promise.**
//! Everything here may change in a minor version before 1.0. The crate root
//! and [`prelude`](crate::prelude) are the author API and are what a plan is
//! written against; nothing in this module is needed to declare, validate or
//! inspect a plan.
//!
//! It exists because the pieces below are genuinely cross-crate:
//!
//! - a substrate implements [`Runtime`], [`TaskHandle`], [`Clock`] and
//!   [`Observer`] — `sdax-tokio` is the reference implementation;
//! - the run driver lives in `sdax-tokio`, so it must be able to reach the
//!   plan's erased bodies ([`bodies_of`], [`BodySource`]), build a body
//!   context ([`CxInner`]) and take what the body left in it
//!   ([`CxInner::take_held`], [`CxInner::put_output`],
//!   [`CxInner::hold_count`]);
//! - a run implements [`Scope`] and [`ChildControl`] so that `cx.spawn` and
//!   `Child::ready` mean something;
//! - [`engine`] is the vocabulary a machine, a scripted driver and a trace
//!   checker all speak.
//!
//! [`RawKey`] is here rather than at the root because it is the engine's node
//! address; an author addresses a node by name through
//! [`NodePath`](crate::NodePath). It is still what
//! [`Key::raw`](crate::Key::raw) returns and what a
//! [`Finding`](crate::Finding) carries, so it stays public — just not part of
//! the author's vocabulary.

pub use crate::contracts::{
    BoxFuture, Clock, Joined, NoObserver, Observer, Runtime, TaskHandle, Time,
};
pub use crate::cx::{ChildControl, CxInner, InstanceId, Scope, StopSignal};
pub use crate::key::RawKey;
pub use crate::plan::SEMANTICS;
pub use bodies::{bodies_of, bodies_of_with_input, Bodies, BodySource, Task};

pub mod bodies;
pub mod engine;

/// The stepping simulator behind `Plan::simulate`, for a harness that
/// watches every event and effect.
pub mod sim {
    pub use crate::sim::{ScriptError, SimStep, Simulator, SpawnOutcome, FOREIGN};
}

/// The per-run slot table.
///
/// Hidden rather than `pub(crate)` only because [`Deps`](crate::Deps) — a
/// public trait an author's `needs` tuples implement — names it in
/// `Deps::fetch`, so it must stay publicly reachable for that signature to be
/// legal. Nothing outside the engine has a reason to touch it.
#[doc(hidden)]
pub use crate::key::Slots;
