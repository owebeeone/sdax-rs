# Current-thread drain exposure — TDD log

`OD-CT-DRAIN`: a `current_thread` runtime handle now goes through a constructor
whose name says what a dropped `Running` cannot do there. Same convention as the
stage logs (LBT-007): one row per step, the test written first, the RED evidence
quoted from what the compiler or the runner actually said, and the change that
made it GREEN.

## What was wrong

Dropping a `Running` cancels the run, aborts its bodies and hands the release
graph to an engine-owned drainer. The drainer is a task. On a `current_thread`
runtime a task makes no progress between `block_on` calls, so a `Running`
dropped inside a `block_on` that then returns releases nothing until the next
one — and a process that never enters another leaks the run's scope **without a
word**: no fault, no trace event, no report line, nothing in `shutdown()` that a
caller is obliged to look at.

Found twice independently. The substrate review raised it as **S-09**
(`dev-docs/Review-Stage2-Substrate.md`) and closed it as documentation only: the
crate docs, `docs/Running.md` and contract § 10 each say the drainer needs a
driver thread. A later doc review reopened it on two grounds, both of which the
owner accepted (2026-09-06):

1. **Documenting a foot-gun that silently leaks is weaker than refusing it.**
   Three pages said the right thing and `TokioRuntime::new` still accepted a
   `current_thread` handle in silence, which is where the reader who never
   reached those pages meets the hazard.
2. **A destructor is the wrong place to signal.** It is invisible during
   teardown, and an unwind out of a `Drop` risks a cascading abort.

What did **not** need fixing is the behaviour. A current-thread run that awaits
`shutdown()` drains inside the caller's own await and is entirely correct, and
that is nearly every run in this workspace: 25 of the 29 adapters it builds are
current-thread, because current-thread plus paused time is how the conformance
suites stay deterministic. The hazard is *drop*, not `current_thread`. So the
answer is an **acknowledgement at the adapter constructor**, not a refusal at
`start` and not a `RunOptions` flag — the flavour is a property of the runtime,
so it is settled once per handle rather than re-derived per run. Reasoning and
rejected alternatives: `OD-CT-DRAIN` in `dev-docs/SdaxContract-v1.md` § 13.

## MSRV

`Handle::runtime_flavor()` and `tokio::runtime::RuntimeFlavor` are available at
**MSRV 1.75** with the pinned `tokio =1.53.1`, checked before anything was built
on them: `runtime_flavor` is an inherent method on `Handle`
(`src/runtime/handle.rs:461`) reachable under the `rt` feature alone — it needs
no `rt-multi-thread`, which is a **dev**-dependency feature here — and
`RuntimeFlavor` is re-exported unconditionally from `src/runtime/mod.rs:617`.
Neither uses a language feature newer than the crate itself, whose own
`rust-version` is **1.71**. `RuntimeFlavor` is `#[non_exhaustive]`, so every
`match` on it carries a wildcard arm.

## Log

| # | test written first | RED | GREEN |
|---|---|---|---|
| 1 | `crates/sdax-tokio/tests/adapter.rs`: `new_refuses_a_current_thread_handle_and_names_the_constructor_that_takes_one` (`#[should_panic(expected = "current_thread_no_background_drain")]`), `new_takes_a_multi_thread_handle_in_silence`, `the_acknowledging_constructor_also_takes_a_multi_thread_handle`, and `a_current_thread_adapter_is_the_same_adapter_it_always_was` — the tracking, clock, observer and `shutdown` behaviour of the rows above it, on the constructor a current-thread handle now has to go through | `error[E0599]: no associated function or constant named 'current_thread_no_background_drain' found for struct 'TokioRuntime'` (2×), with rustc's own `note: if you're trying to build a new 'TokioRuntime', consider using 'TokioRuntime::new'` | `TokioRuntime::new` reads `handle.runtime_flavor()` and panics on `CurrentThread` with a message naming `TokioRuntime::current_thread_no_background_drain`, which is the constructor such a handle goes through; both delegate to a private `build`, so there is exactly one way to make the struct and every public way in states its terms. The acknowledging constructor is **total** — a multi-threaded handle is accepted, because the promise it asks for (always await `shutdown()`) is correct on every flavour, and refusing the safe direction would refuse legitimate use for no gain |
| 2 | `a_current_thread_adapter_is_the_same_adapter_it_always_was`, now compiling | **`assertion left == right failed / left: 3000000000 / right: 2000000000`** — the clock reading | not a defect in the change: `start_paused` auto-advances to the next deadline as well as to the 2 s advanced by hand, so the exact figure is not this row's to assert. Rewritten as a **bound** (`>= 2 s`); the exact reading stays where it already was, in `the_tokio_clock_only_moves_when_tokio_time_moves` |
| 3 | the existing suites, converted | — | 25 construction sites now name the acknowledging constructor and 4 keep `new`. Straight one-line renames in `tests/{adapter,driver,review_edges,review_observer,root_input,spike}.rs`, `tests/conformance/tokio_driver.rs`, the five `tests/guide/*.rs` scenarios and the `#[cfg(test)]` runtime in `src/driver/observer.rs`; `tests/conformance/multi_thread.rs` keeps `new`. **`tests/substrate.rs` is the one site that could not be renamed**: its single `adapter()` helper is handed `paused()` and `live()` (current-thread) *and* `R-07`'s two-worker runtime, so it now dispatches on `handle.runtime_flavor()` — the same three-line `match` the rustdoc offers a caller who does not know its flavour statically, which is what makes the panic in `new` trap nobody. That every one of these sites is correct usage, and stays green unchanged, is the evidence that nothing about correct usage changed |
| 4 | the quote gate | `FAIL docs/{Cleanup,Cookbook,Instances,QuickStart,Running}.md guide:… != crates/sdax-tokio/tests/guide/….rs`, each `first difference` on the `let rt = Arc::new(TokioRuntime::new(…))` line | `cargo fmt` wrapped the longer call over three lines in all five scenarios, then each fence body was re-copied from its file — the code block only, no prose, no heading, no surrounding sentence. `./scripts/check-guide-quotes.sh`: **7 fences, 7 scenarios, 0 unquoted, GUIDE QUOTE GATE PASSED** |
| 5 | no new test — the gates and the record | — | Contract § 10's "What the substrate adds" says the drainer fact is no longer only stated, and § 13 gains **OD-CT-DRAIN** dated 2026-09-06. The crate docs' drainer bullet names both constructors. `dev-docs/QueuedWork.md` § 1 is deleted: that file is a queue, not a record, and the reasoning now lives in the Decisions row |

## What is deliberately not here

- **No observer event from the drop guard.** The owner left it open as an
  optional diagnostic, explicitly *not* as the mechanism. It is not taken: it
  would fire after the fact, in the destructor the doc review objected to, and
  the only honest spelling would be a new `TraceKind` — a change to the
  zero-dependency core for something the constructor already prevents. Drop and
  drain semantics are untouched by this change, which was the instruction.
- **No refusal at `start`, and no `RunOptions` flag.** Both are recorded as
  rejected alternatives in `OD-CT-DRAIN`.
- **No fix for the `current_thread` behaviour itself.** There is none to make:
  a task cannot run while nothing is polling it. The exposure is named where the
  runtime is chosen, which is the only place a caller can act on it.
- **`docs/` prose was not edited** beyond the five mechanical fence
  re-copies — the owner has a doc writer for the words. Two pages now need that
  writer, and are listed in the handover: `docs/Reference.md` (its ungated
  ```rust snippet at line 211 builds a `current_thread` runtime and then calls
  `TokioRuntime::new`, which now panics; line 220's "`TokioRuntime::new(Handle)`
  does not own the runtime" is true but no longer the whole story) and
  `docs/Running.md` (lines 21–27 describe the drainer hazard correctly but as
  something the reader must remember, not as something the constructor says).
