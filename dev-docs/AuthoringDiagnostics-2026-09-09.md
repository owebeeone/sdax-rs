# Authoring diagnostics lane B

Baseline: `ce339a59eefcd136562b16f4f20035c29ef0c988`.
Workspace: `/Users/owebeeone/limbo/sdax-wz-diagnostics`, member `sdax-rs`.

## Result

Added five external integration tests in
`crates/sdax-tokio/tests/report_boundary.rs`. They use only public APIs, create
real nested component mounts and run the Tokio adapter with paused time. No
library code, dependencies or public API changed. Existing report formatting
already retains the tested details; this is consumer regression coverage, not a
formatter repair.

The consumer retains `Report` until its declared string boundary, then uses
`into_result().map_err(|report| report.to_string())` and extracts the completed
output. Successful execution returns the expected value, 18. Failing execution
preserves both body and cleanup errors, each with two nested `Error::source`
levels, the complete mounted node path and its phase. A separate typed-report
consumer downcasts both original errors and their original nested sources.

Separate scenarios cover obligations that cannot naturally coexist here:

- A pending resource release exhausts the shutdown budget. `Outcome::Ok` is
  accompanied by `incomplete: {region/reservation/Lease}` and the boundary still
  returns `Err`.
- An identified effect times out and its recovery returns a typed domain error
  containing the original operation ID. For IDs 417 and 918, the boundary retains
  the prepare timeout, recovery phase, ambiguity path and the corresponding ID;
  the recovery error also remains downcastable before conversion.

Every runtime scenario asserts zero tracked tasks after the report returns. All
clock progression is injected; no test sleeps or makes network requests.

## Test-first evidence and honest labels

1. Inspected `FaultKind::Display` before editing: it already traverses up to 16
   source links, preserving the typed error internally. No missing formatter
   behavior was inferred.
2. Wrote the five tests without changing library code. The first focused run
   compiled but failed 0/5 due to fixture mistakes: expecting the plan name in
   `NodePath` (`Request/region/...` instead of `region/...`); assuming one fault
   instead of the inner fault plus two mount propagation faults; and attempting
   to export a live resource from a finite plan (correctly rejected as
   `LiveExport`). These are **fixture failures, not engine RED evidence**.
3. Corrected the fixture to select the actual `Phase::Run` fault, retain mount
   propagation faults, expect complete mounted paths and export completed step
   data. All five passed with the baseline library unchanged. Label: **regression
   guards**, not a library bug fix.
4. Executed a negative consumer that returns only the outer body error
   (`submit request`). The same detail assertions reject it. Also executed five
   omission controls, each removing one required outcome/path/phase/source-chain
   detail from the complete report. Every control is rejected with a captured
   assertion failure. These controls validate the tested diagnostic oracle; they
   are not proof of all report states or arbitrary user error implementations.
5. Focused clippy initially rejected an `io::Error::new(Other, ...)` and a redundant
   closure. Changed them to `io::Error::other` (available since Rust 1.74) and the
   function item. No behavioral or library changes were needed.

Final executed checks, all passing:

```text
cargo test -p sdax-tokio --test report_boundary --locked --offline
  5 passed; 0 failed
cargo fmt --check
cargo clippy -p sdax-tokio --test report_boundary --locked --offline -- -D warnings
```

## Guidance and limits

Lane A owns the public guide/reference. The integration owner was told to show
this same boundary and preserve structured `Report` internally. String conversion
cannot retain typed objects or downcasting. An arbitrary identity type has no
`Display` bound; `Recovery::StillUnknown` does not print its value. A domain
recovery error must carry printable identity context when the application needs
that context at a text boundary. The test demonstrates that supported path; it
makes no automatic identity-formatting claim.

The optional `IdentifiedEffect.needs` forwarding API is deferred by agreement with
the integration owner. Canonical `.needs(...).identified_by(...)` remains the
supported ordering. No separate API surface was edited.

The tests are external Rust integration consumers, not a new independently
packaged consumer workspace. Full shared gates, independent external-package
checks, native gates and Rust 1.75 verification remain the integration owner's
responsibility and are not claimed as executed in this lane. No remote activity,
inference, cleanup, tag, push or root commit was performed.
