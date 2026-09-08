# sdax improvement integration log — 8 September 2026

Baseline: bdea942. Owner authorized implementation, breaking API changes and parallel agents. No commit, push or publication authorized.

## Authoring completeness

- RED: `cargo test -p sdax --test incomplete_declarations --offline`: 1 passed, 2 failed. Discarded acquire/perform (including `mem::forget`) and an abandoned initial resource chain were accepted by build.
- GREEN: reserve a placeholder/key and parallel body slots when a chain starts, complete those slots at the terminal, keep an unfinished-key list only in Build. Structured V-INCOMPLETE-DECLARATION findings survive dropped/forgotten builders. 3 focused tests pass; 86 core unit tests pass, 1 ignored.
- No destructor requirement, runtime bookkeeping or per-run allocation added by this check.

## Lossless standard diagnostics

- RED: report_diagnostics test failed because Display printed `Error` instead of the underlying missing-input cause and cleanup error.
- GREEN: Report displays FaultKind's real error/message and bounded source chain, preserving the whole report and typed error via existing into_result. The focused test passes; no new error wrapper or lossy conversion added.

## Finite output boundary

- Initial fixture needed ordinary `async move` annotation; this compilation repair is not product RED evidence.
- RED after repairing fixture: finite resource export accepted; completed-data export passed.
- GREEN: V-LIVE-EXPORT rejects a direct Resource/Service key exported from a finite scope; completed data from a step still passes. It does not claim to prove arbitrary user data contains no cloned resource.
- Early broad run found three existing conformance fixtures exporting live resource handles from finite child scopes. Those need migration to explicit resident scope semantics during integration; not reported as a green complete suite.

## Data provenance

- RED: inspect_dataflow did not exist (E0599).
- GREEN: separate declaration/binding view includes input and import sources without adding them to the lifecycle view. One test passes. Initial test comparison syntax repaired to existing NodePath comparison API. Mounted input binding coverage is required after component integration.

## Parallel ownership

- Safety checkout: acquisition authority, defensive registration, identified unknown-outcome recovery and tests.
- Components checkout: typed mount binding, formal imports, independent mount identity, cached static topology and tests.
- Services checkout: initialize once, stable published handle, restartable serving episodes and tests.
- Performance checkout: independent fixtures, baseline measurements and reproduction tools. Pi reserved for timed runs; generation/build excluded from engine timer.
- Root checkout: common contract, integration, unfinished builders, output boundary, diagnostics, provenance and final validation.

Final full-gate, platform and performance results will be added only after execution against the integrated code.

## Callback factory panic boundary

- Safety agent found recovery handler factory panics occurring before the guarded poll. Root reproduced the same issue in step factories with an upstream resource: the old driver returned Cancelled instead of Failed and missed the intended panic/cleanup path.
- GREEN: invoke Step and TryStep factories inside the guarded async future. The typed output still publishes once; factory and polling panics use the same reporting path. Both variants pass factory_panics with upstream release and host shutdown verified. Safety/services agents handle their own terminal factories.

## Combined safety/component verification

- First combined workspace run: new feature tests and doctests passed; only three legacy conformance cases (both drivers) and a generated Monte Carlo child failed the new finite-output validation.
- Migrated the three fixtures to resident child scopes, retaining their finite parent behavior. The generator retains its random draw but forces resident lifetime when its selected output is a resource.
- GREEN: sdax-testkit conformance and Monte Carlo targets pass after those migrations. No assertion weakened.

## Cross-feature and forwarding guards

- `improvement_integration.rs` passes: four unknown operations across two mounts and two concurrent runs retain distinct identities, join interrupted bodies before recovery, recover before parent-resource release, and leave no tracked tasks.
- `dataflow_mounts.rs` passes: repeated mounts have distinct input/formal-port provenance and retain parent-resource lifecycle edges.
- Added forwarding regression guards for resident component output and a child input bound to a parent resource; both finite-parent exports are rejected. All four finite-output tests pass. These additional guards passed initially and are not new RED evidence.
- Adapter conformance passes all 81 cases after the finite-child fixture migrations.

## Stable services integration

- Combined safety registration and service episode context; added ServingPhase to sealed shared phases. Service body slots align with every reserved declaration and are indexed by the committed key.
- Preserved recovery and service-episode fault absorption in the shared checker. Removed the obsolete NeverReady display branch and migrated the new component/dynamic service test to initialize/serve.
- Added a forgotten service-initializer regression: RED E0599 before service integration; GREEN after integration with a finding explicitly requiring serve.
- Full integrated workspace: 416 passed, 0 failed, 2 ignored before the final docs registrations. Public guide target subsequently passed 11/11 and exact quote gate passed 10/10.
- Rust 1.75 library builds passed on Pi arm64 and native Windows x64 against integrated source snapshot SHA-256 a48b813ded23d146610eb6a427e09295c0cf0d64138a8d6e44e8e7fb5e0bfe3d. Further review fixes require another MSRV check.

## Independent review: foreign export scope

- Review reproduced a foreign key with the same declaration index returning the second plan's different value with a clean report.
- RED: export_scope regression was accepted during build.
- GREEN: ForeignKey validation now requires the exported key to resolve in the current declaration scope and returns a structured export finding. The regression and four finite-output forwarding tests pass.
- Independent review also found delayed RAII destruction and incorrect context deadlines; parallel fixes are in progress. No final acceptance claimed yet.

## Independent review: conflicting blocking pools

- Review showed `.blocking_step(...).limit(broad).on(serial)` admitting two bodies despite serial capacity one.
- RED: blocking_pool regression accepted the conflicting declaration.
- GREEN: V-BLOCKING-LIMIT rejects the combination with an explicit fix to remove limit and use the blocking execution pool. No multi-pool arbitration or implicit precedence was introduced.

## Independent review: deadline completion

- Resource cleanup originally reused acquisition within (1ms rather than 100ms); a stopping service exposed no stop deadline. Both desired assertions were observed RED.
- GREEN: deadline_for reads the active absolute timer according to phase, capped by the scope budget; live contexts refresh before stop. Initializer interruption replaces the old within timer with stop grace.
- A later reviewer witness found retry cleanup already running before shutdown still exposed None. RED reproduced; GREEN via RefreshDeadline metadata effect on scope settlement, preserving cleanup shielding with no Signal/Abort.
- Seven focused deadline cases and the updated post-begin host handoff test pass. Final independent consumer suite passes all eight tests across the four defect families.
