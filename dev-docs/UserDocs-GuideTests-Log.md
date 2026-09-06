# Guide scenarios and the quote gate — TDD log

`docs/SdaxUserDocsPlan.md` § 3 and § 4 rest on one mechanism: **a documentation
snippet is a checked quotation of a real test**, never a hand-maintained copy.
The pages landed in the doc-only pass; the mechanism did not. This is the log of
building it — same convention as the stage logs (LBT-007): one row per step, the
test written first, the RED evidence quoted from what the compiler, the runner
or the gate actually said, and the change that made it GREEN.

Delivered: `crates/sdax-tokio/tests/guide.rs` and seven scenarios under
`crates/sdax-tokio/tests/guide/`; `scripts/check-guide-quotes.sh`; the gate
added to `AGENTS.md`'s "All gates" list.

## What was wrong

Eleven pages carried **seven** `rust,guide:<name>` fences naming scenarios that
did not exist. There was no `tests/guide.rs`, no `tests/guide/`, and no checker.
Every fence was therefore an unchecked prose copy — the exact failure the plan's
fence grammar exists to prevent — and five of them had already drifted past the
API: they called `plan.start(rt)`, superseded by `plan.start(rt, input)`
(`OD-ROOT-INPUT`, `dev-docs/RootInput-Log.md`).

The Quick Start had drifted further than the signature. It varied per-run data
with a captured `Arc<Mutex<Vec<u32>>>` inbox popped once per `start` — the
untyped shared-cell workaround the design rejected and that `start`'s new
parameter exists to close. Its own prose said "there is no context object", and
the code beneath it was a context object with a lock around it. That fence is
now a `Plan::with_input::<u32>` plan started twice with two different values,
and there is no `Mutex` anywhere in the guide tree.

## Log

| # | written first | RED | GREEN |
|---|---|---|---|
| 1 | `crates/sdax-tokio/tests/guide.rs`, declaring the seven modules, and the seven files under `tests/guide/` | `error[E0583]: file not found for module 'blocking_pipeline'` … `= help: to create the module 'blocking_pipeline', create file "crates/sdax-tokio/tests/blocking_pipeline.rs"` — 7×, "could not compile `sdax-tokio` (test "guide") due to 7 previous errors" | the `#[path]`s are load-bearing. `tests/guide.rs` is a **crate root**, so a bare `mod simple_hold;` resolves against `tests/`, not `tests/guide/` — the plan's claim that `mod` alone finds the subdirectory is wrong, and `tests/conformance.rs` already spells its modules out for the same reason. Each `mod` now carries `#[path = "guide/<name>.rs"]`, and `guide.rs`'s own rustdoc says why |
| 2 | the seven scenarios against today's API. The RED for this row is the pages' own bodies: all seven fences were extracted from `docs/**/*.md` verbatim, dropped into `tests/guide/`, and compiled | `error[E0061]: this method takes 2 arguments but 1 argument was supplied` at `guide/blocking_pipeline.rs:35:49`, `guide/fail_and_release.rs:39:49`, `guide/resident_service.rs:27:32`, `guide/simple_hold.rs:40:48`, `guide/simple_hold.rs:41:49`, `guide/spawn_child.rs:72:32` — each pointing at `crates/sdax-tokio/src/running.rs:432:8`. Six call sites in five files; `inspect_plan` and `validate_reject` start nothing and so compiled | the seven rewrites below. `start(rt)` → `start(rt, ())` where the plan declares no input; the Quick Start's mutex inbox → a typed root input; closure parameters that needed an annotation once the surrounding code changed; and, where a fence asserted less than the page claimed, the assertion the claim needs |
| 3 | `scripts/check-guide-quotes.sh`, written against the tree as it then stood | 7 × `FAIL`, one per page — `FAIL docs/QuickStart.md:27 guide:simple_hold != crates/sdax-tokio/tests/guide/simple_hold.rs / first difference at fence line 1 / page: PLACEHOLDER / file: use sdax::prelude::*;`, and six more naming the first line at which each page and its test disagreed. `GUIDE QUOTE GATE FAILED`, exit 1 | the fences hold the files. Note the concurrency: `docs/` was being revised by the doc writer at the same time, and by the time the synchronising pass ran they had already copied the scenario files into all seven fences, so it reported `synchronised 0 fence(s)`. Who typed the copy does not matter; what the gate asserts is that page and file are byte-equal, and it does |
| 4 | the gate's own teeth — four probes, below | see the table | a checker that cannot fail is worse than none; that defect has been found twice on this project |
| 5 | no new test — `AGENTS.md` | — | `./scripts/check-guide-quotes.sh` added to "All gates", with the rule stated in one line: edit the test, then copy it into the fence — never the other way round |
| 6 | no new test — the gates | — | all eight green, quoted in the report. 319 tests → **326**, the rise being exactly the seven scenarios |

## The seven

| scenario | page | what it proves |
|---|---|---|
| `simple_hold` | `QuickStart.md` | the floor: a resource that `hold_value`s, a step that needs it **and** the run's typed input, `Policy` / `Shutdown` / `Mode` all passed explicitly, an exported output. One `Plan<u32, u32>` built once, started with `123` and `456`, each run exporting its own number — no shared cell, so the only thing that can differ between the runs is the value handed to `start`. Both reports `is_clean()` |
| `fail_and_release` | `Cleanup.md` | a fault does not skip cleanup. `Boom` fails, the report is `Outcome::Failed`, the release body still ran (an `AtomicBool` it set), and the single fault names the node that raised it (`faults[0].node.leaf() == "Boom"`) |
| `validate_reject` | `Errors.md` | `build` refuses, twice over and with no runtime: an empty plan yields exactly one finding whose rule is `Rule::Empty`, and a plan with two nodes named `Ping` yields a `Rule::DupName` finding whose `nodes` are `["Ping", "Ping"]`, whose `detail` names the node and whose `fix` is non-empty. A finding is a value that names the rule, the node and a fix — asserted, not described |
| `resident_service` | `Running.md` | `Mode::Resident` with one service: `ready()` reaches steady, `shutdown()` ends it, awaiting the `Running` gives `Outcome::Ok` and a clean report |
| `spawn_child` | `Instances.md` | a `Plan::template::<u8>` registered on the parent, declared with `.spawns`, instantiated by the service body through `cx.spawn(&link, 1u8)`, awaited with `Child::ready()`. The child's `Sock` releases before the parent's `Endpoint` — the release body reads the child's flag as it runs and finds it already set, which is the imported key outliving the instance |
| `inspect_plan` | `Inspect.md` | `inspect()` before any effect: the two nodes in declaration order, **exactly** the one declared edge (`Ping` → `Conn`, `Reason::DeclaredNeed`), `release_order().before("Ping", "Conn")`, and `why("Ping")` waiting on `Conn` while `why("Conn")` waits on nothing. No-effect is not left to construction: every body increments an `AtomicUsize`, and after `inspect()` **and** `simulate(&Script::new())` the counter is asserted `0` |
| `blocking_pipeline` | `Cookbook.md` | a declared `pool("cpu", 2)`, a `blocking_step` on it whose `run` is not `async`, and a dependent async step that consumes what it produced and exports `42`. Real time, not `start_paused`: a pool thread cannot advance a paused clock, and the auto-advance would fire the shutdown budget while the body worked |

## The gate has teeth

Four probes. Each was applied to a green tree, run, and reverted; the tree is
green again after all four.

| probe | injected | what the gate said |
|---|---|---|
| 1 — silent drift | `Some(&123)` → `Some(&124)` in the `simple_hold` fence in `docs/QuickStart.md`; one digit, still valid Rust, still plausible prose | `FAIL docs/QuickStart.md:27 guide:simple_hold != crates/sdax-tokio/tests/guide/simple_hold.rs / first difference at fence line 37 (…/simple_hold.rs:37) / page: assert_eq!(first.output.as_deref(), Some(&124)); / file: assert_eq!(first.output.as_deref(), Some(&123));` — `GUIDE QUOTE GATE FAILED`, exit 1. Restored → `GUIDE QUOTE GATE PASSED`, exit 0 |
| 2 — a claim with nothing behind it | `tests/guide/inspect_plan.rs` moved away, the fence left in place | `FAIL docs/Inspect.md:23 guide:inspect_plan names no scenario: crates/sdax-tokio/tests/guide/inspect_plan.rs is missing`, `== 7 fence(s), 6 scenario(s), 0 unquoted`, gate failed. Restored → passed |
| 3 — the grammar | ` ```rust,guide:inspect_plan ` → ` ```guide:inspect_plan ` | `FAIL docs/Inspect.md:23 info string is 'guide:inspect_plan', not \`rust,guide:inspect_plan\` / a fence opening \`guide:inspect_plan\` alone renders as grey text`, gate failed. Restored → passed |
| 4 — the allowed case | an extra `tests/guide/orphan.rs` no page quotes | `note crates/sdax-tokio/tests/guide/orphan.rs is quoted by no page (allowed; it still runs)`, `== 7 fence(s), 8 scenario(s), 1 unquoted`, `GUIDE QUOTE GATE PASSED`, exit 0 — a scenario nobody quotes is not a defect. Removed |

Probe 1 is the one that matters. The failure this mechanism exists to prevent is
not a page that stops compiling — that is loud — but a page that still compiles
and no longer says what the test says.

## Gates

All eight, on the finished tree:

```
cargo fmt --check                                                  clean
cargo test --workspace --locked --offline                          326 passed, 2 ignored, 0 failed
cargo clippy --workspace --all-targets --locked --offline -D warnings   clean
cargo package -p sdax --locked --offline --allow-dirty              Packaged 55 files, 507.5KiB
./scripts/check-architecture.sh                                     ARCHITECTURE GATE PASSED
./scripts/compile-fail.sh                                           15 witnesses, all PASS
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --offline  clean
./scripts/check-guide-quotes.sh                                     7 fences, GUIDE QUOTE GATE PASSED
```

Test count 319 → 326: the seven scenarios and nothing else.

## What is deliberately not here

- **No `sdax-testkit`, no shared helper module, no fake clock, no `examples/`
  binary.** Every scenario that runs builds its own runtime and repeats the four
  lines that do it. That repetition is the point: the fence is the whole file,
  so a reader who copies a block gets a program that runs. A helper would be a
  line the page cannot show.
- **No `#[tokio::test]`.** The workspace pins tokio without `macros`. Five
  scenarios build a `current_thread` runtime by hand — four with
  `start_paused(true)`, `blocking_pipeline` on real time because a pool thread
  cannot advance a paused clock. `inspect_plan` and `validate_reject` need no
  runtime at all, which is itself what they are showing. Nothing sleeps.
- **No prose edits.** The one editing exception granted was a mechanical copy of
  the seven fence *bodies*; no heading, sentence or surrounding paragraph was
  touched, and no page outside those seven fences was changed. The pages'
  narrative belongs to the doc writer.
- **No region markers.** The fence is the entire file, `#[test]` and all, as
  § 3 decided. Trimming to "just the plan" is what breaks the check.
- **Nothing committed, tagged or pushed.**
