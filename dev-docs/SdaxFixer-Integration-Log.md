# SDAX fixer integration evidence

Date: 2026-09-06. Baseline: `8147239ebed2c5a82dc622bdebc39eddfbe9a6de`.

Five GPT-5.3-Codex-Spark workers were dispatched, plus an inherited-model
read-only architecture adviser. DOCS returned a completion report. VIEW, INPUT,
RELEASE and PACKAGE stopped with Spark usage-limit errors. No quota reset or
model switch was performed. ARCH returned the explicit publication handshake
recorded in [SdaxFixer-Agents.md](SdaxFixer-Agents.md).

## Retained source and deferred work

Retained: README and guide installation corrections, the corrected external
consumer check, and canonical MIT/Apache license copies in both crate roots.

Deferred to [fixer-handoffs/](fixer-handoffs/README.md): interrupted VIEW tests,
RELEASE helper/workflow/tests, PACKAGE checker, and the coordinator's dependent
shared-check/CI/release-helper patch. The latter's four mocked tests passed,
but no combined gate or release check was accepted. Its original RED was a
missing `scripts/check-all.py`, exit 1. A mock test pass does not verify a release.

Each deferred patch passed `git apply --check` after its active changes were
removed. Runtime code, CI, publish workflow and release helper remain at their
baseline. The empty malformed shell-fixture artifact was removed. No commit,
tag, push, publication or visibility change was made.

## Onboarding corrections and evidence

Coordinator review found that the DOCS handoff left unqualified registry commands,
a placeholder revision, and a consumer checker that silently inserted its own
Tokio dependency. The first drift probe failed with:

```text
AssertionError: Gate silently inserts Tokio absent from documented recipe
```

The corrected checker substitutes only SDAX Git source coordinates. It checks
that every other parsed manifest option is preserved. The positive case uses the
documented Tokio dependency verbatim; negative controls then remove that direct
dependency or `test-util`. A missing dependency in the docs is never repaired by
the checker. The script requires Python 3.11+ for stdlib `tomllib`.

GREEN, independently rerun by the coordinator:

```text
PASS  documented guide builds and runs
PASS  missing direct Tokio is rejected
PASS  missing test-util is rejected
```

Additional checks confirmed that removing Tokio or `test-util` from a temporary
copy of the documentation preserves that omission in the generated manifest,
that all three published-facing TOML tables agree, and that no `<GIT_REV>`
placeholder remains in those pages. No package version is hardcoded into the
local SDAX source substitution, so the consumer gate does not require `0.1.0`
after a version bump. The guide Rust source was unchanged.

This verifies a fresh external project with local SDAX paths. It does not claim
an unauthenticated Git installation succeeded while the repository is private.

## Final checkpoint checks

All eight baseline gates plus the new consumer check passed:

| Check | Result |
|---|---|
| `cargo fmt --check` | PASS |
| `cargo test --workspace --locked --offline` | PASS: 332 executions, 0 failures, 2 ignored |
| `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` | PASS |
| `cargo package -p sdax --locked --offline --allow-dirty` | PASS; freshly produced archive |
| `./scripts/check-architecture.sh` | PASS |
| `./scripts/compile-fail.sh` | PASS: 15 error-code witnesses |
| `./scripts/check-guide-quotes.sh` | PASS |
| `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --locked --offline` | PASS |
| `python3 scripts/check-consumer-guide.py` | PASS: positive and two negative controls |

Raw local logs:
`/var/folders/02/bn9c9g2x5qj8bb42zb857p7c0000gn/T/sdax-spark-checkpoint-spjqtd9g/`.
The directory includes `results.json` and one log per check. The commands and
results above remain the durable record if the temporary logs are removed.

Rust 1.75 was not installed locally and no new hosted run was triggered. Existing
baseline hosted evidence is not claimed as a run on this changed tree.

## Archive evidence

All four crate-root license copies compare byte-for-byte with the repository
originals. Inspection of the newly Cargo-produced core archive verified both
license texts and the canonical README. Missing and altered in-memory archive
member controls were rejected for each license; these are content-validator
controls, not proof of the deferred package checker's correctness.

`cargo package -p sdax-tokio --list --locked --offline --allow-dirty` passed and
selected both licenses and README. Actual adapter packaging was also attempted:

```text
cargo package -p sdax-tokio --locked --offline --allow-dirty
error: no matching package named `sdax` found
```

That registry-resolution requirement remains open. No fabricated/staged archive
is claimed as successful adapter package verification. The archive audit and
both adapter command outputs are recorded with the raw logs above.

## Original plan integrity and readiness

`SdaxFixer.md` remained byte-for-byte unchanged. Its SHA-256 before and after is
`3b2192c453a55492d254dcb3bd10a89101ce1dfc3e30f149953bb1be65b307be`.

This checkpoint does not close all five findings. F1, both F2 parts, F3, the F5
checker and CI/release parity remain implementation work. F4's documentation and
local consumer check are complete; unauthenticated installation must be checked
when access permits. The remaining stronger-model and Spark assignments are in
the adjacent execution plan, not in the original remediation plan.
