# Queued work — agreed, not yet started

One line per item: what, why, who decided, and what it is waiting on. Delete a row when it
lands. This file exists so an agreed decision is not carried only in a conversation.

The guide-input update is complete: all eight compiled guide scenarios use
`start(rt, input)` and the quotation gate passes. Component input admission
and imported-input inspection are also fixed; see `SdaxFixer-Input-Log.md`.

## Open, needing an owner decision (not queued work)

- `V-SERVICE-UNBOUNDED` is per-plan and does not reach a child plan's services
  (`dev-docs/Stage3Report.md` § 8 question 1).
- `AGENTS.md`'s fast-loop budget (~0.55 s) against a measured ~4.8 s.
- `S-02`, exhaustive schedule enumeration, has never run.
- `panic = "abort"` behaviour is stated but not executed (needs a profile rebuild).
- Three substrate probes deferred: a body whose poll never returns, a service restart racing
  its own `ServeEnded`, `R-05` under CPU load.
