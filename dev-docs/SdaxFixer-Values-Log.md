# F1 — structural value publication

Date: 2026-09-06. Status: **implemented and locally verified**.
Baseline: `8147239ebed2c5a82dc622bdebc39eddfbe9a6de`.

## Outcome

Joins now provide their unit values, and components publish their actual child
exports before consumers are constructed or readiness is observed. The original
probes now produce outputs 7, 99, 100 and 42. No-export unit components and
explicitly exported unit values take distinct paths; a missing declared export
fails the run and still discharges held cleanup obligations.

The engine requests publication and waits for a non-body acknowledgement.
Both drivers finish each effects batch and drain a publication FIFO before
external messages and readiness snapshots. A pending transfer remains outstanding
after cancellation or another failure, preventing premature cleanup/end/instance
closure. Exact instance lookup and complete instance-node enumeration prevent
component descendants from reading another run's slots.

The host API changes: `BodySource::publish_ready` is required, with
`Effect::PublishReady`, `Event::ReadyPublished` and `NodeState::Publishing`.
Custom host sources must implement the operation explicitly. This is documented
in contract section 10 and host rustdoc; author join/component semantics are
unchanged. No dependencies or manifests changed.

## Ownership and review

The coordinator owned engine state, admission, cancellation, faults, cleanup,
instance mapping, protocol regressions, contract updates and integration. Three
bounded inherited-model workers owned the typed bridge, drivers/custom sources,
and real-value regressions respectively. All handed ownership back before final
integration. Spark was not retried and no credit reset was used.

A separate read-only engine review found no confirmed blocker. Requested
component cancellation, budget expiry and nested cleanup cases were added.
The engine tests also prove an instance containing a pending component descendant
cannot close before its acknowledgement, while another instance stays independent.

The review noted that delivering an unrelated external event between publication
and acknowledgement would violate the required driver ordering. Both drivers
now drain the FIFO before taking such events; the contract states that obligation.

## RED evidence

- The real-value worker copied only its new tests into an isolated archive of
  baseline HEAD. Seven basic value tests failed, reproducing missing outputs and
  failed consumers. Four more edge witnesses failed; the existing failed/skipped
  export refusal control passed. See `SdaxFixer-Values-Tests-Log.md`.
- The bridge test initially failed with E0599 because `publish_ready` did not
  exist. Five direct bridge tests now check actual Arc identity, missing/wrong
  exports, unit cases, imported exports, and separate/closed instance scopes.
  See `SdaxFixer-Bridge-Log.md`.
- `cargo test -p sdax --test fixer_publication --locked --offline` initially
  failed with E0599 for missing `PublishReady`, `ReadyPublished` and `Publishing`.
  Six engine tests now cover ordering, invalid/duplicate acknowledgements,
  failure, cancellation, expired budgets, components and instance closure.
- The simulator FIFO regression initially observed zero acknowledgements rather
  than two. Its final form includes a third join admitted by the first
  acknowledgement, proving it queues behind the second outstanding result.
- A deliberately incorrect immediate-acknowledgement driver mutation failed
  under paused time because it stranded an independent spawned task. The correct
  FIFO implementation was restored and the tests rerun. This is a negative
  control, not a historical baseline result. See `SdaxFixer-Drivers-Log.md`.

Initial authoring mistakes in the additional instance fixture (spawning from a
step instead of a service, then omitting the service stop budget) were corrected
without weakening its pending-publication assertions. Final Clippy also rejected
three redundant clones of Copy template handles; those clones were removed.

## Final verification

**359 test/doctest executions passed, zero failures, two ignored**: 27 more
executions than the baseline. New coverage includes 12 actual-value runtime
regressions, five bridge tests, six engine protocol tests, three driver failure
fixtures, and the simulator FIFO regression.

| Gate | Final result |
|---|---|
| `cargo fmt --check` | PASS |
| `cargo test --workspace --locked --offline` | PASS: 359 passed, 2 ignored |
| `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` | PASS |
| `cargo package -p sdax --locked --offline --allow-dirty` | PASS |
| `./scripts/check-architecture.sh` | PASS |
| `./scripts/compile-fail.sh` | PASS: 15 error-code witnesses |
| `./scripts/check-guide-quotes.sh` | PASS |
| `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --locked --offline` | PASS |
| `python3 scripts/check-consumer-guide.py` | PASS: positive and both negative controls |
| `git diff --check` | PASS |

The missing-export driver fixture was strengthened during integration to acquire
and release a real child resource; both missing u32 and missing explicit-unit
exports fail without constructing a consumer and release that resource exactly
once. The final workspace test run includes this version.

Logs: `/var/folders/02/bn9c9g2x5qj8bb42zb857p7c0000gn/T/sdax-f1-final-hp_bcvl_/`.
`results.json` records the initial pass, including the fixture/Clippy failures;
`recheck-results.json` and `*-final.log` record their corrected passing runs and
fresh package. The other gates passed and their inputs were unaffected by the
test-only corrections. The table above is the durable record if temporary logs
are later removed.

Rust 1.75 is not installed locally; its claimed MSRV was not reverified on this
changed tree. No hosted CI run was triggered. Actual adapter packaging remains
blocked by the unpublished core package's registry resolution, as recorded in
the prior integration log. These are not claimed as new passing release evidence.

## Remaining work

F1 is complete locally. F2 input inspection/admission, F3 immutable release
selection, and F5 package-checker/CI-release parity remain open. The earlier
onboarding and license changes are retained. The repository has not yet passed
a final all-findings public-readiness review.

`SdaxFixer.md` remains unchanged, SHA-256
`3b2192c453a55492d254dcb3bd10a89101ce1dfc3e30f149953bb1be65b307be`.
The adjacent execution plan records this continuation. No commit, tag, push,
publication or repository visibility change was performed.
