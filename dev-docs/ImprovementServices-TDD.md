# Service recovery improvement TDD log

This log records the stable-handle service recovery change from
`SdaxImprovementPlan-2026-09-08.md` section 5. Initialization attempts and
serving episodes are separate: initialization publishes one handle; recovery
invokes a serving factory again with that same handle.

## Stable handle and long-lived dependent

Wrote `service_recovery::initialization_publishes_one_stable_handle_across_serving_episodes`.
It declares the new `initialize(...).serve(...)` surface, fails two serving
episodes, and requires a dependent that initialized once to observe the third
episode through the same published allocation.

RED: `cargo test -p sdax-tokio --test service_recovery --locked --offline`
failed with `E0599`: `Node<..., Service>` had no method named `initialize`.

GREEN: after adding the split initializer/serving factory and explicit engine
episode effect, the same command passed the stable-handle test.

## Startup retry, exhaustion, terminal completion, shutdown and panics

Added focused cases requiring startup retry to count initialization attempts,
restart exhaustion to report the final 1-based episode without reinitializing,
shutdown to cancel a long restart backoff, a terminal service to end on its
episode's `Ok`, and synchronous initializer/serving factory panics to remain
inside the engine report with no tracked task left behind.

RED: the focused test command compiled the surface and ran six cases;
`restart_exhaustion_reports_the_last_episode_and_does_not_reinitialize` failed
because both the recovered historical failure and the final episode failure
were reported (`faults.len()` was 2 rather than 1). This also exposed that
serve records still carried the initialization attempt instead of the episode.

GREEN: recovery now clears the preceding episode's parked fault when the next
episode starts, without publishing readiness again, and serve/stop trace and
fault records use the episode number. All six focused tests pass offline.

## Latched readiness and cleanup episode ordering

Added `readiness_stays_latched_while_serving_recovers`. A second dependency
becomes ready one second after the service publishes its handle, while the
service is in a 60-second restart backoff. The dependent must start from the
already-published handle and the run must reach its first steady state without
waiting for the recovery timer.

RED: with recovery backoff treated as loss of readiness, the exact focused
test advanced to 60 seconds and invoked episode 2 before the dependent could
start (`episodes` was 2 rather than 1).

GREEN: dependency readiness and the run's first steady-state decision now use
the latched initialization result. The serving state remains observable as
`RestartBackoff { episode, until }`, but it does not retract the published
handle. The focused suite passes 7/7.

The 10,000-case long Monte Carlo pass then exposed a separate ordering bug at
case 2804 (run seed `15343719950732250898`): `StopRequested` used serving
episode 2 while the matching `Abandoned` and incomplete record used
initialization attempt 1. Service interruption and abandonment after
initialization now use the serving episode scope. Replaying all 2,805 cases up
to the failure produced no invariant violation; the regular 3,000-case Monte
Carlo suite and all 76 conformance cases pass offline.
