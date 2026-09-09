# API authoring proposals 1 and 2 — implementation

Baseline: `711004f739e71fd437bbb8474109a9099099b13e`.

## Changes

`IdentifiedEffect` forwards `needs`, `within`, `retry` and `idempotent` to the
existing node builder. Replacing ordinary dependencies reconstructs the typed
identity wrapper; its raw-key logic retains identity and avoids duplicating it.
Recovery and terminal type states, validation and engine behavior are unchanged.

`Report::into_required_output` returns the existing output Arc or
`RequiredOutputError::{Failed, MissingOutput}`. Each error owns the full original
report, accessible through `report` and `into_report`. Failed runs take precedence
over missing output. The optional-output `into_result` is unchanged. Error and
Debug implementations require no Debug bound on the output. The four starter
boundaries now use this helper. No new dependency was added.

Proposal 3 is deferred: another mounting API is not yet justified by the scaffold
experiment. The proposal records the criteria for reconsideration.

## Test-first record

Before implementation, `required_output` failed with missing method/type errors;
`identified_configuration` failed with missing `IdentifiedEffect::needs`.
Both logs remain under `D:/sdax-exp2/api-proposals-20260909/evidence/red-*.log`.
The initial adapter fixture also had an unrelated plain-value return; it was
corrected to the existing step Result contract. First post-implementation checks
found missing error annotations in two test factories and an incorrect assertion
that the inspection view counts input keys as node edges. Fixtures now use an
explicit identity step, allowing exact edge-count comparison.

The focused green run passes five core tests and the adapter order-equivalence
test. These cover equivalent inspection/configuration, replacement and identity
edge deduplication, retry safety and unfinished declaration rejection, actual
timeouts/retries/recovery with stable Arc identity, required and optional output,
missing output without panic, complete error report retention and non-Debug output.
A compile-fail witness additionally requires recovery before persistence.
These are regression guards, not evidence of improved model authoring performance.

## Verification

All compilation uses retained D-drive directories on dabeest with Git/MinGW Bash,
locked offline dependencies and Rust 1.96.0. Rust 1.75 builds of both libraries
passed (`evidence/msrv-01/results.json`). The scoped verification-only Python
retention wrapper from the preceding integration keeps temporary fixtures; no
production scripts or assertions are modified by it.

The first full inventory stopped on an io-error construction lint in a new test;
that expression was simplified without suppressing the lint. The second passed
the Rust gates but its Python fixtures were refused by the retention wrapper's
old destination guard. A separate copy of the wrapper now names this verification
directory; its hash is retained in `evidence/retention-wrapper.sha256`.

All 11 gates passed in `evidence/all-gates-03/results.json`: lockfile, formatting,
strict Clippy, workspace tests, architecture, compile-fail witnesses, guide quotes,
rustdoc, Python script tests, external consumers and fresh package archives.
All 847 local files matched the remote snapshot before this evidence-only update;
the inventory is `evidence/verified-source.json`. No library source changed after
the Rust 1.75 builds. No inference was run and no Mac build outputs were created.

## Documentation follow-up

The README, authoring guide, API reference and error guide now distinguish
required from optional output and describe both error variants and report accessors.
The reference limits configuration-order freedom to the four forwarded methods.
The compact guide demonstrates post-identification configuration and required
output; the composition boundary also uses the new helper. An executable guide
example covers success, clean missing output and cancellation. This documents
existing behavior, not a new engine feature or authoring-effectiveness experiment.

The first check stopped on the new example's unnecessary large-error wrapper;
the example now calls the API directly. All 11 shared checks then passed on
Rust 1.96.0 at `D:/sdax-exp2/doc-pass-20260909/evidence/all-gates-02/results.json`.
All 848 local files matched the verified snapshot before this evidence-only
append. Guide quotations match their entire executable files. Library source is
unchanged from the Rust 1.75-verified implementation; no new MSRV run was needed.
