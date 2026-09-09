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
#[path = "guide/components.rs"]
mod components;
#[path = "guide/current_thread_start.rs"]
mod current_thread_start;
#[path = "guide/fail_and_release.rs"]
mod fail_and_release;
#[path = "guide/inspect_plan.rs"]
mod inspect_plan;
#[path = "guide/readme_cleanup.rs"]
mod readme_cleanup;
#[path = "guide/resident_service.rs"]
mod resident_service;
#[path = "guide/simple_hold.rs"]
mod simple_hold;
#[path = "guide/spawn_child.rs"]
mod spawn_child;
#[path = "guide/unknown_recovery.rs"]
mod unknown_recovery;
#[path = "guide/validate_reject.rs"]
mod validate_reject;

#[path = "guide/ai_acquisition.rs"]
mod ai_acquisition;

#[path = "guide/ai_component.rs"]
mod ai_component;

#[path = "guide/ai_recovery.rs"]
mod ai_recovery;

#[path = "guide/ai_service.rs"]
mod ai_service;

#[path = "guide/starter_composition.rs"]
mod starter_composition;

#[path = "guide/starter_retry_cleanup.rs"]
mod starter_retry_cleanup;

#[path = "guide/starter_recovery.rs"]
mod starter_recovery;

#[path = "guide/starter_service.rs"]
mod starter_service;

#[path = "guide/starter_support.rs"]
mod starter_support;

#[path = "guide/required_output.rs"]
mod required_output;
