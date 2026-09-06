# Queued work — agreed, not yet started

One line per item: what, why, who decided, and what it is waiting on. Delete a row when it
lands. This file exists so an agreed decision is not carried only in a conversation.

## 1. Guide tests need the unit input (manager, follows the root-input change)

`start` becomes `start(rt, input)`. The user-docs agent's `crates/sdax-tokio/tests/guide/*`
scenarios were written against `start(rt)` and will not compile. Add `()`, then re-run
`scripts/check-guide-quotes.sh` so the quoted fences in `doc/` still equal their sources.

## 2. Open, needing an owner decision (not queued work)

- `V-SERVICE-UNBOUNDED` is per-plan and does not reach a child plan's services
  (`dev-docs/Stage3Report.md` § 8 question 1).
- `AGENTS.md`'s fast-loop budget (~0.55 s) against a measured ~4.8 s. Wait for the docs agent,
  which is editing that file.
- `S-02`, exhaustive schedule enumeration, has never run.
- `panic = "abort"` behaviour is stated but not executed (needs a profile rebuild).
- Three substrate probes deferred: a body whose poll never returns, a service restart racing
  its own `ServeEnded`, `R-05` under CPU load.
