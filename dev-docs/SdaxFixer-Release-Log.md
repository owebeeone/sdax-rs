# Coordinator status: incomplete, deferred

The worker hit the Spark usage limit before returning a completion report.
The fixture list and GREEN instruction below are intended coverage, not verified
passing evidence. The draft was rejected for integration and preserved in
`fixer-handoffs/release.partial.patch`. See sections 7–8 of
[SdaxFixer-Agents.md](SdaxFixer-Agents.md) for concrete defects and resumption.

# SdaxFixer Release Assignment Log (P3)

## 2026-09-06

### Scope

- `.github/workflows/publish.yml`
- `RELEASE.md`
- `scripts/release_selection.py` (new)
- `scripts/test_release_selection.py` (new)
- `dev-docs/SdaxFixer-Release-Log.md`

### Plan

- Resolve `refs/tags/<tag>^{commit}` once and reuse that SHA for all publish-path
  steps.
- Keep publish after test gates and MSRV (`cargo +1.75 test`).
- Validate package and dependency versions before publishing.
- Refuse tag not matching `vX.Y.Z`, missing tags, and moved tags.
- Distinguish crates.io absence versus error and verify existing
  `sdax-tokio` provenance.
- Keep checks local with temporary git fixtures and stubbed publish commands.

### Red/Green record

- RED: `python3 scripts/test_release_selection.py` (before script wiring) fails because
  helper and resolver were not implemented.
- GREEN: run the same command after implementing `scripts/release_selection.py` and
  populating fixtures.

### Test fixtures covered

- malformed tag
- unknown tag
- annotated tag
- lightweight tag
- newer branch head with same version
- moved tag between selection and verification
- missing tag entry
- crates.io absent/present/error status
- provenance check for existing `sdax-tokio`
- command-recording publish stub

### Command log

- `python3 scripts/test_release_selection.py`
