//! The Monte Carlo suite: a deterministic random walk over the plan space.
//!
//! [`prng`] is splitmix64 — no dependency, and the same sequence for the same
//! seed everywhere, so one printed number replays one case. [`gen`] builds a
//! random plan through the author surface (sometimes an invalid one, on
//! purpose, so the validator is under test too), over the key and attribute
//! bookkeeping of the private `keys` module and the catalogue of declaration
//! mistakes in the private `mutate` module. [`script`] builds a random script
//! and schedule for the plan that came out.
//!
//! The runner is `tests/monte_carlo.rs`: it drives every case through the
//! same [`ScriptedDriver`](crate::ScriptedDriver) the conformance suite uses
//! and checks every trace with the independent
//! [`invariants`](crate::invariants) checker.

pub mod gen;
mod keys;
mod mutate;
pub mod prng;
pub mod script;
