# Interrupted Spark handoffs

These patches preserve work from the 2026-09-06 agent run. They are **drafts**,
not accepted fixes. Spark exhausted its quota before four workers returned a
completion report. The coordinator removed these drafts from active code after
review, retaining their exact diffs here. An empty stray shell-fixture output
file, `publish_log.txtnEOFnchmod`, was removed.

Read [the execution plan](../SdaxFixer-Agents.md), especially sections 6–8,
before resuming. The original remediation plan remains unchanged.

| Patch | Contents | Disposition |
|---|---|---|
| `input-view.partial.patch` | Original inspection test draft | Restored, corrected and extended during F2; historical only, do not reapply |
| `release.partial.patch` | Publish workflow, release documentation, selection helper and tests | Incorrect checkout/metadata selection, incompatible CLI wiring and incomplete provenance; requires repair |
| `package-gate.partial.patch` | Archive checker | Staged tarball can masquerade as real package; replace evidence flow |
| `integration.partial.patch` | Shared check entry point, four unit tests, CI and release helper wiring | Prerequisite helpers incomplete; forward dirty allowance and verify the combined gate |

Each patch passed `git apply --check` at the preserved checkpoint. No claim is
made that its tests pass or its behavior is correct. Applicability on a later
checkout must be checked again. Prefer applying one reviewed assignment in an
isolated checkout or under exclusive file ownership; do not apply all patches
into active CI and treat existing baseline tests as proof of completion.

The four license copies and corrected onboarding changes are active source,
not part of these patches. See [integration evidence](../SdaxFixer-Integration-Log.md)
for the retained checkpoint's actual validation.
