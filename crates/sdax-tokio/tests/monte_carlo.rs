//! The Monte Carlo walk, on the run driver.
//!
//! The generator is the testkit's — the same plans, the same scripts, the same
//! seeds — so this is not a second walk but the same one performed by the
//! adapter instead of the stepping simulator. Trace equality is not asserted
//! (`sdax-testkit`'s own walk owns that); what is asserted is every invariant,
//! every rejection, and that the run terminated.
//!
//! It is deliberately shorter than the pure walk: a case here builds a tokio
//! runtime and spawns a task per attempt, so it costs about a hundred times a
//! pure case. `SDAX_MC_SEED` and `SDAX_MC_CASES` override the defaults, and a
//! failure prints the case seed, which replays it here *and* on the pure
//! driver.

#[path = "conformance/tokio_driver.rs"]
mod tokio_driver;

use sdax_testkit::mc::{gen, prng, script};
use sdax_testkit::quiet_scripted_panics;
use tokio_driver::TokioDriver;

/// The fast loop's run seed. Fixed, so CI is deterministic.
const FIXED_SEED: u64 = 0x5DA0_2026_0906_0002;

/// How many cases the fast loop walks on the adapter.
const DEFAULT_CASES: u64 = 250;

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// The adapter walk: generated plans and scripts, checked by the independent
/// invariant checker.
#[test]
fn monte_carlo_on_the_adapter() {
    quiet_scripted_panics();
    let run_seed = env_u64("SDAX_MC_SEED", FIXED_SEED);
    let cases = env_u64("SDAX_MC_CASES", DEFAULT_CASES);
    let mut ran = 0u64;
    for i in 0..cases {
        let seed = prng::case_seed(run_seed, i);
        let mut g = prng::SplitMix64::new(seed);
        let generated = gen::generate(&mut g);
        // A plan the validator refuses is the pure walk's business; this walk
        // is about what the driver does with a plan that builds.
        let Ok(plan) = generated.plan else { continue };
        if generated.expect_invalid.is_some() {
            continue;
        }
        let view = plan.inspect();
        let bounded = view.shutdown.budget().is_some();
        let script = script::generate(&mut g, &view, bounded);
        // Started the way an author starts a root run: with its input. Most
        // generated plans declare none, and `()` is what those take.
        let d = match TokioDriver::run_with_input(&plan, (), &script) {
            Ok(d) => d,
            Err(e) => panic!("case seed {seed}: the script would not run: {e:?}"),
        };
        ran += 1;
        if let Some(p) = d.problems() {
            panic!("case seed {seed} (SDAX_MC_SEED={run_seed}, index {i})\n{view}\n{p}");
        }
    }
    assert!(
        ran * 2 > cases,
        "only {ran} of {cases} generated plans were runnable"
    );
}
