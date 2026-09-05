//! `R-01` — suite (c) against the tokio adapter
//! (`cargo test -p sdax-tokio --test conformance`).
//!
//! **The suite is not copied.** Every module below is the file
//! `crates/sdax-testkit/tests/conformance/` already holds, included by path
//! and compiled a second time against a different `Drv` (LBT-009). A row that
//! passes here and there passes on the pure machine *and* on a real runtime;
//! a row that passes only there is a difference between the machine and the
//! adapter, which is the point of running it twice.

#[path = "conformance/tokio_driver.rs"]
mod tokio_driver;

/// The driver the conformance modules run against in this binary.
pub use tokio_driver::TokioDriver as Drv;

#[path = "../../sdax-testkit/tests/conformance/corpus.rs"]
mod corpus;

#[path = "conformance/differential.rs"]
mod differential;

#[path = "conformance/multi_thread.rs"]
mod multi_thread;

#[path = "../../sdax-testkit/tests/conformance/cancel.rs"]
mod cancel;
#[path = "../../sdax-testkit/tests/conformance/cleanup.rs"]
mod cleanup;
#[path = "../../sdax-testkit/tests/conformance/components.rs"]
mod components;
#[path = "../../sdax-testkit/tests/conformance/faults.rs"]
mod faults;
#[path = "../../sdax-testkit/tests/conformance/regressions.rs"]
mod regressions;
#[path = "../../sdax-testkit/tests/conformance/retry.rs"]
mod retry;
#[path = "../../sdax-testkit/tests/conformance/startup.rs"]
mod startup;
