//! Simulation: the pure machine driven by a [`Script`] on a virtual clock.
//!
//! `Plan::simulate` is the author-facing entry (contract § 9, adoption A3):
//! the trace a plan produces for a scripted schedule, with no body run, no
//! runtime and no effect — the pre-ship counterfactual answered by the same
//! code that drives production. The stepping [`Simulator`] behind it lives in
//! [`host::sim`](crate::host::sim), where a test harness can watch every event
//! and every effect.

mod effects;
mod instances;
mod script;
mod simulator;

pub use instances::{SpawnOutcome, FOREIGN};
pub use script::{At, Body, Cleanup, Ending, Request, Schedule, Script, Serve, SpawnSpec};
pub use simulator::{ScriptError, SimStep, Simulator};
