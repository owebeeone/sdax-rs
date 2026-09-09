# Authoring reference and starter integration — 9 September 2026

Baseline member: `5a62e0bf84c1e8341a3cedc1c727c4a6c0e0a26f`.

## Scope and provenance

The focused experiment-2 reference replaces the previous compact reference, with
links to complete starters and clarification of builder terminals, lazy factories,
missing output and recovery faults. Its four complete code blocks are now quoted
guide test files. Existing composition regression tests remain registered.

Four complete starter programs adapt the experiment-3 controls and their 15
behavioral/diagnostic assertions. They use a documented in-memory backend;
the intentionally unfinished experimental scaffolds remain in the remote evidence.
Backend types were renamed, configuration was encapsulated, sources were formatted,
and functions were documented. Library implementation, public API and dependencies
are unchanged. The shipped prose and complete starters are adaptations, not the
byte-identical experimental inputs; their authoring effectiveness has not been
separately measured.

The two experiment reports and API proposal are retained beside this record.
The proposal records possible future changes; it does not claim implementation.

## Test provenance and review

This is an integration of existing tested examples, not a new engine behavior.
Experiment-3 preparation ran the four controls against all 15 assertions, confirmed
that the unfinished scaffolds failed behavioral acceptance, and checked four
panic-on-missing-output negatives. Those are exposed regression guards, not new
held-out evidence. See `D:/sdax-exp2/exp3/evidence/task-preparation/REPORT.md` and
`validation-results-final.json`. No new engine RED/GREEN cycle is claimed.

Independent integration review found one adapter mistake: the service starter
called a helper made private when documenting the backend. Restoring documented
`pub(crate)` visibility fixes that integration issue. Review also clarified that
resolved ambiguity retains its originating timeout fault, and that copied starters
must retain or update their `crate::starter_support` import.

## Verification

Verification uses the retained snapshot at
`D:/sdax-exp2/integration-20260909-authoring/source`, Rust 1.96.0, locked offline
dependencies, and D-drive targets, caches, temporary files and logs. This is a
verification snapshot, not a GWZ workspace or development lane. No Mac build runs.
An isolated Python shim and scoped `sitecustomize` preserve TemporaryDirectory
fixtures instead of deleting them; they do not change assertions or production
source. Their hashes and effects are recorded in `evidence/preparation/environment.json`.

The first inventory run passed formatting but stopped at strict Clippy on nine
unused imports/helpers inherited from the experimental fixtures. Those items were
removed without lint suppressions or behavioral changes; the failed log remains in
`evidence/all-gates-01/clippy.log`.

The second run passed all 11 shared gates: lockfile, formatting, strict Clippy,
workspace tests, architecture, compile-fail witnesses, exact guide quotations,
warning-free rustdoc, Python regression tests, external consumers and fresh archives.
See `evidence/all-gates-02/results.json`. All four quoted guide blocks match their
full executable sources; the four starters contribute 15 regression assertions.
The 842-file verification input inventory matches the local tree byte for byte
before this final evidence-only update. Independent source review approved the
integration conditional on these gates, now satisfied. No new Rust 1.75 run is
claimed; library source is unchanged.
