//! Suite (b): pure planner, validator and seam tests (`cargo test -p sdax --lib`).
//!
//! Every test here is derived from `sdax-v1/B/CanonicalTests.md` § 3 (rows `P-*`)
//! or from the adopted-contract changes (A1..A3, F1..F4). No body is executed by
//! the engine: Stage 0 has no engine.

mod builder;
mod corpus;
mod exec;
mod instances;
mod keys;
mod machine;
mod planner_validate;
mod planner_view;
mod report_order;
mod seam;
mod shorthand;
