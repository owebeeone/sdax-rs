//! `sdax-testkit` — the harness for [`sdax`].
//!
//! Development-only (`publish = false`, LBT-005): it must never appear on a
//! production dependency path, and `scripts/check-architecture.sh` fails the
//! build if it does.
//!
//! A clock a test drives by hand, an observer that records what it is told,
//! a static checker over a [`PlanView`](sdax::PlanView), the scripted driver
//! ([`ScriptedDriver`]: the core's simulator with recording and checking
//! around it), the trace-level invariant checker ([`invariants`]), trace
//! queries in the canonical tests' vocabulary ([`eol`]), and the Monte Carlo
//! suite ([`mc`]).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod eol;
pub mod invariants;
pub mod mc;

mod clock;
mod driver;
mod recorder;

pub use clock::FakeClock;
pub use driver::{Driven, ScriptedDriver};
pub use recorder::TraceRecorder;
