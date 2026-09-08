# Improvement safety: implementation and TDD record

Date: 2026-09-08. Implementation baseline: `bdea94200ae743fc94ea76987cfd4f6927e0ff8d`.
Work performed in the isolated `artifacts/sdax-improvements-20260908/safety` checkout. No dependencies added; no commits, pushes, inference calls, or other checkouts modified.

## Implemented contract decisions

- `Cx<Acquire>` is single-use and not cloneable. `hold(self, factory)` reserves the attempt before invoking the lazy factory. `hold_value(self, value)` consumes the same authority. Call `shared()` before consumption for a cloneable `Cx<Run>` with clock, cancellation, node and attempt identity, but no acquisition or raw-context access.
- Explicit host construction is `CxInner::acquire()` or `CxInner::context::<P>()` for the sealed shared phases. Multiple host authorities share a mutex-protected unused/reserved/held/discharged cell. Repeated asynchronous acquisitions return an error before invoking their factory. Repeated `hold_value` host misuse panics after rejecting reservation, outside the mutex; the original value survives. A discharged or pending attempt cannot reserve again. `put_output` also refuses replacement. The normal path retains one stored obligation, not a vector.
- Replace misleading `Ambiguity::Compensate` with `Ambiguity::Recover`. An effect requests automatic recovery only with its existing `.idempotent()` safety assertion, an explicit typed identity and a handler. `V-RECOVERY-MISSING` replaces the old contradictory persistent-compensation rule.
- `.on_ambiguous(Ambiguity::Recover).identified_by(operation_key).perform(|cx, (deps, operation)| ...).recover_unknown(|cx, operation| ...).compensate(receipt_handler)` is the public route; `.persistent()` is also supported. Identity is an ordinary typed per-run dependency slot, initialized before effect execution and retained through cleanup. The same slot is read across retries; attempt numbers remain distinct. It is never reconstructed from a success receipt or a captured mutable identity.
- Recovery runs after the interrupted attempt is joined, under the ordinary cleanup shielding, dependency lifetimes and budget. `Recovery::Resolved` discharges the unresolved record; `Recovery::StillUnknown` produces the typed `UnresolvedRecovery` error. Errors and panics preserve uncertainty and the recovery failure. Budget expiry records a recovery timeout, incomplete obligation and unresolved record. Original ambiguity remains in history through distinct `RecoveryStart`, `RecoveryOk`, `RecoveryFail` events and `Phase::Recover` failures. Cancellation remains `Outcome::Cancelled` after recovery.
- Receipt compensation still gets only a real recorded receipt, and recovery never triggers subsequent receipt compensation. Persistence suppresses compensation for known success, while explicit unknown recovery remains meaningful.
- Resource/effect prepare and cleanup factories execute inside the guarded future poll. Synchronous panics while constructing those futures are contained just like panics when polling them; an already-registered value is still cleaned up and a failed cleanup does not suppress upstream releases.

## RED and GREEN evidence

The original adversarial report, N8 source and `experiments/logs/07-ambiguous-compensation.log` were read. That log contains the desired failed assertions: N8 expected all completed acquisitions cleaned up (actual active count 1, expected 0); N13 expected the callback even without a receipt (actual callback skipped with "no cleanup body"). The final adverse-behavior witnesses were not mistaken for successful fixes.

| Behavior | RED observed here | GREEN and retained witness |
|---|---|---|
| Preserve first obligation under repeated registration | `cargo test --offline -p sdax safety_second_registration`: expected 7, stored 9 | Reservation state preserves 7; `tests::seam::safety_second_registration_preserves_first_obligation` |
| Recovery without fabricated receipt | `cargo test --offline -p sdax-tokio --test safety_recovery`: E0599 for Recover/identified_by/Recover phase and E0433 for Recovery | `unknown_outcome_uses_identity_without_a_receipt`; real simulated external state both occurred and did not occur, resolution clears uncertainty and preserves Ambiguous trace |
| Recovery expiry preserves failure detail | Focused adapter run: cleanup_failures length 0, expected 1 at expiry | Phase::Recover Timeout alongside incomplete and ambiguous; failure matrix covers StillUnknown, error, polled panic, expiry |
| Recovery factory panic containment | Synchronous recovery factory panic left adapter.tracked() at 1, expected 0 | Factory called inside guarded poll; same failure matrix includes synchronous factory panic |
| Prepare and cleanup factory panic containment | `safety_factories`: registered 7 never released; cleanup factory panic left tracked task at 1 | Both resource and effect variants preserve first receipt/release upstream and record Panic |
| Invariant checker's recovery distinction | Mutating a real receipt-compensation start to RecoveryStart was incorrectly accepted | Negative fixture now rejected unless there is matching ambiguity, Recover policy and no Held |

Additional regression guards (not separate pre-implementation RED claims): rejected factories never run while another authority is reserved; failed acquisition registers nothing; completing-poll registration; known receipt never calls recovery; concurrent canceled persistent operations keep distinct identities and settle interrupted work before recovery; retry identity stays stable and known compensation sees the real receipt; all existing acquisition cancellation/retry and cleanup-order suites remain green.

Four new compile-fail witnesses were added after the behavioral implementation and checked against specific compiler errors: repeated consumption E0382; clone E0599; shared-context acquisition E0599; shared raw-inner access E0599. They are regression witnesses, not a claim that those exact compiler checks were run RED on the original surface.

## Validation

Commands use `CARGO_TARGET_DIR=target-safety`, offline Cargo, and the installed stable toolchain `rustc 1.96.0 (ac68faa20 2026-05-25)` on macOS/aarch64.

- `cargo test --offline --workspace`: 385 passed, 0 failed, 2 ignored, across 31 test result groups. Includes 7 focused adapter safety tests, pure/scripted recovery conformance and adapter differential suites, Monte Carlo regression suites and doctests.
- `scripts/compile-fail.sh`: 19 witnesses passed with their declared compiler codes.
- `cargo clippy --offline --workspace --all-targets -- -D warnings`: passed.
- `RUSTDOCFLAGS='-D warnings' cargo doc --offline --workspace --no-deps`: passed.
- `scripts/check-architecture.sh`: passed.
- `cargo fmt --all`: applied in the isolated checkout only; subsequent checks verify formatting.
- Full `python3 -B scripts/check_all.py --allow-dirty`: passed, including formatting, clippy, lockfile, tests, architecture, compile-fail, guide quotes, rustdoc, script tests, external consumers and archives.
- Rust 1.75 is not installed locally (`rustup toolchain list` inspected). Changed library syntax stays within the existing MSRV surface, but actual Rust 1.75 execution remains pending; a newer toolchain result is not substituted.
- Runtime benchmarks are owned by the performance workstream. This work makes no latency, allocation or throughput claim. The new normal-path cost is defensive reservation plus one extra shared factory reference per invoked guarded terminal; plan factory storage is built once.

## Integration notes

No Build layout changes. `NeedsCompensate` now carries an optional erased recovery handler, and its declaration field is crate-visible to the new typed recovery builder. `Attrs::recovery` records installed handler capability for pure validation/scheduling. The existing cleanup scheduling state is reused internally with a recovery flag; host Effect::Recover is distinct. New source modules are `cx/acquisition.rs` and `recovery.rs`. The simulator and invariant checker were updated with the real driver.

Root owns normative contract/semantics-tag updates, incomplete-builder tracking, general documentation, and final combined validation. Service work must implement the sealed shared-phase trait for its new serve context. Consumers migrate `.hold(async {...})` to `.hold(|| async {...})`; ordinary clock/cancellation work after acquisition takes a `shared()` context first. Raw host users migrate Cx::new to the explicit CxInner constructors. Compatibility aliases were not retained.
