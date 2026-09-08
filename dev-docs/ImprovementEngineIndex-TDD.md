# Engine lookup and admission scan reduction

Date: 2026-09-08. Authoritative checkout: `sdax-rs`, during improvement integration.

## Finding and bounded change

`Table::index_of` scanned the complete flattened node vector for each event's node lookup and for host path, kind, deadline and state lookups. On an N-node chain this introduces quadratic lookup work across its lifecycle, in addition to the scheduler's existing whole-scope scans.

The immutable table now contains one dense declaration-index lookup per run-scope plan identity, held in a `BTreeMap`. A lookup searches scope identities, then indexes its declaration directly: O(log S) in S scopes and independent of the number of nodes in the selected scope. Input/import slots have a sentinel rather than accidentally aliasing another node. Static table construction prepares the index once during plan building. Dynamic flattening creates and fills new scope buckets per instance, and those allocation/indexing costs remain part of dynamic execution. Existing copy-on-write topology keeps one run's dynamic additions out of the cached static definition and other runs.

The additional narrow admission optimization skips `after_settle`'s second `admit_all` call only when the machine has exactly one scope. Its preceding `admit(scope)` already reaches a fixpoint, and `check_steady(scope)` has already run; there is no other scope sharing an imported lock or pool to admit. The multi-scope path is unchanged. This does not introduce an incremental scheduler or weaken cleanup gates.

The broader remaining scans are explicit follow-up candidates: `admit` repeatedly scans all scoped nodes for Pending/Waiting, `check_steady` scans unsettled nodes, and cleanup completions repeatedly scan the scope for open obligations and completion. No claim is made that these changes eliminate quadratic lifecycle work.

## TDD and tests

The index test was first compiled before `RunIndex` existed: E0433 (undeclared RunIndex) was the RED result after correcting fixture API names. The implementation then passed three focused tests:

- Sparse declaration slots, unknown plans, out-of-range indices, and identical indices in different scopes do not collide.
- Repeated mounted components with typed input holes resolve each actual run key and refuse the unmounted declaration key.
- Growing two dynamic instances preserves static and previous-instance keys; the cached static table does not acquire a run's new identity.

These are semantic regressions, not a measured performance proof. Complexity follows from the lookup structure; benchmark timings belong to the performance workstream.

The admission differential fixture compares complete machine effect streams against the original global-sweep route, forced by an inert Planned scope with no nodes, grants, parent or execution. Both fixtures include exclusive locks, a shared pool and a held-resource retry; one also includes an Isolate fault and skipped dependent. The reference adds no production switch or state field. Its before/after execution evidence is recorded below after the reserved host timing window ends.

## Validation status

After indexing, before the admission guard: focused tests 3/3 passed; full `cargo test --offline --workspace --locked` passed. Target directory `/tmp/sdax-engine-index-target` isolates compiler artifacts. No timing runs were performed by this workstream.

Compiler/test work paused when the performance workstream reserved the Mac; only edits continued during that window. Final post-guard validation follows below.


Final execution evidence: the admission differential test passed on the original
unconditional global-sweep implementation, then passed again after the
single-scope guard, with identical complete effect streams. The final offline
workspace test run completed successfully: 408 passed, 0 failed, 2 ignored.
All-target clippy was attempted during root integration; it encountered the
root's intentional E0599 `initialize` RED test in `incomplete_declarations.rs`
and two unrelated `err_expect` findings in `finite_output.rs` lines 71 and 108. Those files belong to the root workstream and were not modified here.
Root owns final combined clippy, MSRV and benchmark validation.
