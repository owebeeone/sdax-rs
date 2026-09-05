//! Suite (c) — conformance tests on the scripted driver
//! (`cargo test -p sdax-testkit --test conformance`).
//!
//! Rows C-* are `sdax-v1/B/CanonicalTests.md` § 4, adapted to the adopted
//! contract as the file-level docs of each module say. The invariant checker
//! runs on every trace produced here, in addition to the listed expectations.

/// The driver the conformance modules run against in this binary. The tokio
/// adapter's own binary (`sdax-tokio`, `tests/conformance.rs`) includes the
/// same modules with its own `Drv`, so the suite has one source (LBT-009).
pub use sdax_testkit::ScriptedDriver as Drv;

mod cancel;
mod cleanup;
mod components;
mod corpus;
mod faults;
mod regressions;
mod retry;
mod startup;
