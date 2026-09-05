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
//! suite ([`mc`]), and a [`BodySource`](sdax::host::BodySource) that runs a
//! `Script` on a real runtime ([`ScriptedBodies`]) so suite (c) can be
//! re-run against an adapter without a second copy of the suite.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod eol;
pub mod invariants;
pub mod mc;

mod clock;
mod driver;
mod recorder;
mod scripted;

pub use clock::FakeClock;
pub use driver::{Driven, Recorded, ScriptedDriver, WhyAt};
pub use recorder::{ReportSummary, TraceRecorder};
pub use scripted::{quiet_scripted_panics, ScriptedBodies};
