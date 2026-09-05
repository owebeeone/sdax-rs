//! Suite (c) — conformance tests on the scripted driver
//! (`cargo test -p sdax-testkit --test conformance`).
//!
//! Rows C-* are `sdax-v1/B/CanonicalTests.md` § 4, adapted to the adopted
//! contract as the file-level docs of each module say. The invariant checker
//! runs on every trace produced here, in addition to the listed expectations.

mod cancel;
mod cleanup;
mod components;
mod corpus;
mod faults;
mod regressions;
mod retry;
mod startup;
