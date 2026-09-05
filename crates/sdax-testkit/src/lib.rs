//! `sdax-testkit` — the harness for [`sdax`].
//!
//! Development-only (`publish = false`, LBT-005): it must never appear on a
//! production dependency path, and `scripts/check-architecture.sh` fails the
//! build if it does.
//!
//! Stage 0 ships the pieces that need no engine: a clock a test drives by
//! hand, an observer that records what it is told, and a static checker over a
//! [`PlanView`](sdax::PlanView). The scripted driver (`Script`, `Schedule`,
//! `ScriptedDriver`) and the trace-level invariant checker arrive with the
//! machine in Stage 1.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod invariants;

mod clock;
mod recorder;

pub use clock::FakeClock;
pub use recorder::TraceRecorder;
