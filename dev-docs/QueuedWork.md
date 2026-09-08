# Queued work — agreed, not yet started

One line per item: what, why, who decided, and what it is waiting on. Delete a row when it
lands. This file exists so an agreed decision is not carried only in a conversation.

The guide-input update is complete: all eight compiled guide scenarios use
`start(rt, input)` and the quotation gate passes. Component input admission
and imported-input inspection are also fixed; see `SdaxFixer-Input-Log.md`.

The whole-tree shutdown validator and Linux/MSRV workflow fixes are implemented;
see [Release readiness](ReleaseReadiness-2026-09-08.md) for current verification
and publication prerequisites. Older stage/fixer logs describe historical checkpoints.

## Open, needing an owner decision (not queued work)

- `AGENTS.md`'s fast-loop budget (~0.55 s) against a measured ~4.8 s.
- `S-02`, exhaustive schedule enumeration, has never run.
- `panic = "abort"` behaviour is stated but not executed (needs a profile rebuild).
- Three substrate probes deferred: a body whose poll never returns, a service restart racing
  its own `ServeEnded`, `R-05` under CPU load.
