//! The programs `docs/` quotes.
//!
//! Each module under `tests/guide/` is one scenario: an ordinary `#[test]`
//! that builds its own tokio runtime (this workspace has no `macros` feature,
//! so there is no `#[tokio::test]`), writes a plan, starts it and asserts on
//! the `Report`. Cargo does not treat files in a subdirectory of `tests/` as
//! targets, so this file is the target and `mod` is what pulls them in. The
//! `#[path]`s are load-bearing: this file is a crate root, so a bare
//! `mod simple_hold;` would look for `tests/simple_hold.rs` — the same reason
//! `tests/conformance.rs` spells its modules out.
//!
//! A `rust,guide:<name>` fence in `docs/` is the **entire** file
//! `tests/guide/<name>.rs`; `scripts/check-guide-quotes.sh` is the gate that
//! says so. Run these with `cargo test -p sdax-tokio --test guide`.

#[path = "guide/blocking_pipeline.rs"]
mod blocking_pipeline;
#[path = "guide/fail_and_release.rs"]
mod fail_and_release;
#[path = "guide/inspect_plan.rs"]
mod inspect_plan;
#[path = "guide/resident_service.rs"]
mod resident_service;
#[path = "guide/simple_hold.rs"]
mod simple_hold;
#[path = "guide/spawn_child.rs"]
mod spawn_child;
#[path = "guide/validate_reject.rs"]
mod validate_reject;
