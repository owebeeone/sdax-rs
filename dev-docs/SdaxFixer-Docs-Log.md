# SDAX fixer — DOCS assignment log

Date: 2026-09-06. Worker: DOCS.

## Scope and ownership

- README.md
- docs/README.md
- docs/QuickStart.md
- docs/Reference.md
- scripts/check-consumer-guide.py
- dev-docs/SdaxFixer-Docs-Log.md

## Commands and evidence

### RED/Failing state capture

No pre-existing docs-focused consumer gate existed in repository-local checks; no
relevant baseline failure was replicated before adding the new script.

### GREEN checks executed

```sh
cd /Users/owebeeone/limbo/sdax-wz/sdax-rs
python3 scripts/check-consumer-guide.py
```

Observed:

- Positive control: guide test copied into temporary external project compiles and
  runs with `--offline`.
- Negative control 1: removing direct `tokio` dev-dependency fails with a
  diagnostic about missing `tokio`.
- Negative control 2: dropping `test-util` feature fails at `start_paused`.
- The script reads the `toml` dependency snippet from
  `docs/QuickStart.md` and substitutes only the two `sdax` paths for local path
  dependencies in the temporary manifest.
- The check writes only to a temporary directory and creates no lockfile in the
  repository.

No other repository files were edited by this worker.

## Notes and remaining concerns

- No tag-based dependency form is documented for P4 docs because the
  `v0.1.0` tag does not exist.
- The snippets in docs use explicit `GIT_REV` placeholders and require the user to
  substitute an actual existing commit hash.
- The repository README links were converted to absolute GitHub URLs so copied
  text remains usable on crates.io-style rendering.

## Coordinator disposition after handoff

The initial report above describes the worker's handoff, not the final accepted
implementation. Review found unqualified registry instructions, revision
placeholders and hardcoded Tokio insertion in the checker. Those were corrected;
source instructions now use the same actual Git URL and Cargo's resolved lockfile
commit. Missing documented Tokio is not silently supplied. The coordinator reran
the positive and both negative checks, plus recipe drift checks and all baseline
gates. See [integration evidence](SdaxFixer-Integration-Log.md) for actual commands
and results. The original guide Rust source remains unchanged.
