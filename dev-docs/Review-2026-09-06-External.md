# External review of Stage 1 — 2026-09-06

Received by the owner from an external reviewer after commit `8ef4e0a` (Stage 1), relayed to
the manager. Origin not stated. Its thesis: the ~15 machine defects the Monte Carlo suite
found are not fifteen designs but **three rules forgotten on different terminal ramps**, so
the copies should be collapsed into helpers that a new ramp cannot bypass. It asks for a
behaviour-preserving extraction only — no new bugs hunted, no Stage 2, nothing weakened.

The full brief is reproduced in the task record; its substance is the three helpers
(`flush_faults`, `end_component_attempt`, `settle_or_skip_inner`), their call sites, the hard
rules, and two `rg`-based completion criteria.

## Manager's verification, before any edit (at `8ef4e0a`)

| claim | verdict | evidence |
|---|---|---|
| the fault flush is copy-pasted | **confirmed, 9 copies** | `faults.rs` 230, 269, 383, 397; `cleanup.rs` 207, 248; `admit.rs` 254; `settle.rs` 59, 73, 143. A tenth hit, `cleanup.rs:427`, is `report.faults = take(&mut self.faults)` — the final report build, a different operation; excluded. |
| a component's terminal observation is inlined in three places | **confirmed** | Of six `Interrupted { held` emits, exactly three are component sites: `settle.rs:142`, `cleanup.rs:206`, `cleanup.rs:97`. The other three (`faults.rs:229`, `faults.rs:381`, `settle.rs:72`) are ordinary node interrupts that bump `attempt` or handle ambiguity, and are out of scope. |
| the inner-scope decision is special-cased per site | **confirmed** | `skip_scope(` appears in `settle.rs` and `admit.rs`; `faults.rs` and `cleanup.rs` carry their own "if still Admitting\|Steady, settle" arms. |
| no engine file should still say "exactly as it is when" / "as it does when the scope settles" | **manager's verification was WRONG** | The manager grepped line-by-line and found nothing, and told the implementer the criterion was already satisfied. Both phrases did exist, wrapped across comment lines (`cleanup.rs`: "the report's, exactly / // as it is when the node fails or is interrupted"; `admit.rs`: "already failed (INV-9), as / // it does when the scope settles under it"). The implementer caught the error, and removed both comments with the copies they cross-referenced. Lesson: grep multiline for prose in comments (`tr '\n' ' '` first). |
| file sizes justify the move | **confirmed** | `faults.rs` 496 lines (against a ~500 guideline), `cleanup.rs` 434. |

### One wrinkle the manager found, not in the review

The three component sites do **not** emit the same value today. `settle.rs:142` and
`cleanup.rs:206` emit `held: self.slots[c].started`; **`cleanup.rs:97` emits a literal
`held: true`**. Unifying them is behaviour-preserving only if `started` is necessarily true
at that site. The site sits behind a check that the inner scope is `Settling` with
`in_flight == 0`, which suggests it is, but the implementer was required to **prove** it and
record the argument — and, if it cannot be proved, to stop and report the site as a leftover
semantic bug rather than fold it in. See the Stage 1 TDD log row for the outcome.

## Owner decision (2026-09-06)

Apply the extraction as specified. Behaviour-preserving; the existing suite (174 tests, the
21 pinned Monte Carlo seeds included) is the oracle; nothing weakened; no fourth helper
without a verbatim third copy.

## Outcome (2026-09-06)

Applied. Three helpers in a new `crates/sdax/src/host/engine/exits.rs` (81 lines, all
`pub(super)`); ten fault-flush copies and three component-terminal copies collapsed; seven
inner-scope special-cases reduced to six calls. Engine net −86/+36 lines; `faults.rs`
496→488, `cleanup.rs` 434→413, `settle.rs` 233→214.

Both reviewer criteria hold, verified by the manager: exactly **1** `mem::take(… .faults)`
in the engine (inside `flush_faults`) and exactly **4** `Interrupted { held` emits (one in
`end_component_attempt`, three untouched node-interrupt sites). Behaviour preserved: 174
tests before and after, the 21 pinned Monte Carlo seeds replay clean, and a fresh 50 000-case
walk passes.

### The `held: true` wrinkle — proved, and a second one found

The manager asked the implementer to prove `slots[c].started` is necessarily `true` at
`cleanup.rs:97` before unifying it with the literal. It did, by a stronger route than the
scope-state argument: `St::Running` is assigned in exactly one place in the crate
(`admit.rs:164`), the line immediately after `slot.started = true` (`admit.rs:163`), on the
same borrow, unconditionally for every kind; `started` is initialised `false` and never
written elsewhere, so it is monotonic. Hence `st == St::Running ⟹ started == true`, and the
site sits directly behind that guard. Manager re-verified the two assignments.

The implementer then flagged a **second** difference the manager's brief had not: that site
did not flush faults and the helper does. It argued the flush is a no-op because a
component's slot can never hold a fault — `slots[n].faults` is written only by
`fail_attempt` and two kind-guarded sites (`Effect`, `Service`), and `start` pushes no
`Effect::Spawn` for a component, so a contract-conforming host never delivers a body outcome
for one. Recorded rather than buried, with the note that **Stage 2's driver must not give a
component a body outcome**, or the assumption needs revisiting.
