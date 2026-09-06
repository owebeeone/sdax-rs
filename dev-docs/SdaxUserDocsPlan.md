# sdax-rs user documentation — phased plan

Status: **IN PROGRESS 2026-09-06, revision 4**. `docs/` pages are
written. Guide tests, the quote-check script, crate `README.md`,
`AGENTS.md`, and the crate-root rustdoc fix are **not** in this pass
— they live outside `docs/` / this plan. The `rust,guide:<name>`
fences are the draft of those test files; when the tests land, copy
the fence body out and turn the checker on. This is a temporary
inversion of "tests are the source of truth".

Python `sdax` (workspace member `sdax`, `git@github.com:owebeeone/sdax.git`)
is the inspiration for **topics and tone**, not for layout or vocabulary.
Its user guide is one fat `README.md`. The standard here is the opposite:
`dev-docs/` stays internal; `docs/` is how to use the crate; the README
points at `docs/`.

## 1. Split

| tree | reader | contents |
|---|---|---|
| `docs/` | someone writing a plan and running it | how to declare, start, observe, stop, recover |
| rustdoc | someone looking up a type | signatures, invariants of that item |
| `dev-docs/` | someone changing the crate | contract, stage reports, TDD logs, reviews, this plan |
| `README.md` | first hit on GitHub / crates.io | one screen: what it is, install, link to `docs/` |

`docs/` does not discuss stages, INV-/T-/C- ids, Proposal B, the host
engine, Monte Carlo, or how the crate was verified. Those stay in
`dev-docs/` and in rustdoc of `sdax::host` for people who need them.

rustdoc remains the API dictionary. `docs/` links to items; it does not
re-list every method.

## 2. What to take from Python sdax

Python's README is the topic list. Map, do not translate.

| Python topic | Rust page | notes |
|---|---|---|
| What it is / when to use | `docs/README.md`, Cookbook | in-process lifecycle, not Celery/Airflow |
| Installation | Quick Start | `cargo add sdax sdax-tokio` |
| Helper constructors | Authoring | kinds, not `task_func` / `task_sync_func` |
| Quick Start (graph + elevator) | Quick Start | one graph; **no elevator** — Rust has no level adapter |
| Cleanup guarantees | Cleanup | the page a leaky body would have needed |
| Execution model (DAG, waves, phases) | Concepts | need-readiness, no wave barriers; kinds instead of pre/exec/post-on-every-task |
| Error handling (`SdaxExecutionError`) | Errors | `Invalid` at build; `Report` / `Fault` at run |
| Per-task timeout / retry / keys | Authoring | `.within`, `Retry`, typed `Key<T>` |
| TaskGroup subtasks | Instances | `cx.spawn(&template, input)`, `Child` |
| Shared context + concurrent reuse | Concepts, Running | the `Plan` is the immutable value; each `start` is a run |
| Use-case recipes | Cookbook | API endpoint, service startup, pipeline |
| Phase-by-phase runner | *omit* | no `AsyncPhaseRunner`; `inspect` / `simulate` cover observation |
| Performance table | *omit until measured* | do not invent numbers |
| "Testing" = how to run pytest | *omit* | contributor, not user. `Plan::simulate` belongs on Inspect |
| Comparison-to-Celery table | Cookbook, one paragraph | when *not* to use |

A short "if you know Python sdax" box is allowed on Concepts. A dedicated
porting guide is out of scope — user docs do not discuss lineage.

Python's `SDAX_DAG_MODE_DESIGN.md` is a runtime design. The analogue here
is `dev-docs/SdaxContract-v1.md`, not a `docs/` page.

## 3. Target tree

```
docs/
  README.md        index and "I want to…"
  QuickStart.md    install, first plan, start, read the report
  Concepts.md      graph, kinds, needs, policy, reuse
  Cleanup.md       hold / Held / release graph / drop
  Authoring.md     every kind and the common attributes
  Running.md       start, Running, cancel, tokio caveats
  Errors.md        validate findings; Report / Fault / policy
  Inspect.md       inspect(), why, simulate
  Instances.md     templates, spawn, Child
  Cookbook.md      recipes and when not to use
```

The runnable programs behind those pages are **ordinary `#[test]`s**, not
`examples/` binaries and not a harness. They live next to the adapter a
user would depend on. Cargo does not treat files in a subdirectory of
`tests/` as targets — the same pattern `tests/conformance.rs` already
uses — so the target is the file that declares the modules:

```
crates/sdax-tokio/tests/guide.rs     the test target; `mod` each scenario
crates/sdax-tokio/tests/guide/
  simple_hold.rs                     finite resource + step (Quick Start)
  validate_reject.rs                 build() returns findings
  fail_and_release.rs                a fault still runs the release graph
  resident_service.rs                Mode::Resident, ready, shutdown
  spawn_child.rs                     template + cx.spawn + Child
  inspect_plan.rs                    inspect() + simulate() (Inspect)
  blocking_pipeline.rs               blocking step + pool (Cookbook)
scripts/check-guide-quotes.sh        fence body == named scenario file
```

`guide.rs` is the crate root of that target (`mod simple_hold;` looks
in `tests/guide/simple_hold.rs`). The command that runs them is
`cargo test -p sdax-tokio --test guide`. A filter of `-- guide` selects
test *names* and would match nothing.

Simple and complex both belong from the start. A three-node hold is
the floor; a resident service that spawns a child is the ceiling for
this tree. Nothing in between may grow a helper module, a clock fake,
or a scripted driver — if a scenario needs `sdax-testkit`, it does not
belong here.

Each file is one scenario: build a tokio runtime by hand (no
`#[tokio::test]`; this crate has no `macros` feature), write the plan,
`start`, assert on the `Report`.

`docs/` quotes those files. A Markdown fence is a second copy, so a
test can evolve and the page silently drift — the failure these tests
exist to prevent. The tests are the source of truth; the quote is
checked, not trusted:

- The info string is `rust,guide:<name>`. Markdown takes the first
  word as the language, so `rust` keeps highlighting on GitHub and
  every other renderer; the checker matches the `guide:<name>`
  attribute and ignores the rest. A fence that opens `guide:<name>`
  alone is unhighlighted grey text and is a defect.
- That fence quotes the **entire** file `tests/guide/<name>.rs` —
  `#[test]`, the function wrapper, the assertions. The front door
  shows a test, not an application. That is intended: the block is
  honest and it is what `cargo test -p sdax-tokio --test guide`
  runs. Do not trim the fence to "just the plan". Region markers
  were considered and rejected; trimming breaks the check.
- `scripts/check-guide-quotes.sh` walks `docs/**/*.md` and fails unless
  each such fence body equals that file (trailing-newline normalised).
  A `guide:<name>` with no matching file fails. An unquoted scenario
  file is fine — it still runs as a test.
- Do not keep a hand-copied fence that is not tagged `guide:`.

## 4. House rules (writers)

- Second person, present tense, working code.
- Show the author surface (`sdax::prelude`, `sdax_tokio::{TokioRuntime, PlanStart}`).
  Do not name `sdax::host::{Machine, Event, Effect, CxInner}` except to say
  an author does not implement them.
- Every snippet is a `rust,guide:<name>` fence quoting a
  `crates/sdax-tokio/tests/guide/` scenario, or a rustdoc-tested block.
  `cargo test -p sdax-tokio --test guide` runs the scenarios.
  `scripts/check-guide-quotes.sh` proves the fences still match.
  No `sdax-testkit`, no shared harness, no `examples/` binary.
- Do not default policy in prose that the API refuses to default.
- A guide test builds its own runtime, writes a plan, starts it, and
  asserts. If it needs a helper, a fake clock, or `sdax-testkit`, it
  is in the wrong tree. Use tokio paused time when the scenario needs
  a clock; do not sleep.
- Stale crate-root rustdoc ("Stage 0 has no execution") is a defect of
  the docs work, not a feature of the narrative.

## Progress (2026-09-06, doc-only pass)

Landed under `docs/`: README, QuickStart, Concepts, Cleanup, Authoring,
Running, Errors, Inspect, Instances, Cookbook. Fences:
`simple_hold`, `fail_and_release`, `validate_reject`,
`resident_service`, `spawn_child`, `inspect_plan`,
`blocking_pipeline`.

Landed after the doc-only pass: crate `README.md` is the front door
(points at `docs/`); `AGENTS.md` has the `docs/` vs `dev-docs/` rule
and the fence grammar. Workspace root README is a member index that
points at `sdax-rs/docs/`.

Still deferred: 1.2 tests + `guide.rs` + quote-check script; 1.3
crate-root rustdoc in `lib.rs`; Phase 5 gates.

## 5. Phases

Foundational first. After Phase 2, Authoring / Running / Errors can be
picked up independently. Each step is one goal, budgeted aspirationally
under 500 LOC.

### Phase 1 — The front door

Milestone: a new user can install, write a three-node plan, start it on
tokio, and know where the rest of the guide lives.

**1.1 Convention and index.** Create `docs/README.md`: one-paragraph
what-this-is, the "I want to…" table, and a sentence that `dev-docs/`
and `sdax::host` are not this tree. State the fence grammar
(`rust,guide:<name>` = entire `tests/guide/<name>.rs`, test
wrapper included). Add a short
`docs/` vs `dev-docs/` line to `AGENTS.md` so later writers do not
dump status into `docs/`.

**1.2 Quick Start, the target file, and the first two tests.** Create
`tests/guide.rs` (`mod simple_hold; mod fail_and_release;`) and those
two scenario files. `docs/QuickStart.md` quotes `simple_hold` with a
`rust,guide:simple_hold` fence: `TokioRuntime` from a `Handle`, a resource
that `hold`s, a step that needs it, `build(Policy, Shutdown, Mode)`,
`plan.start(rt)`, assert the `Report` is ok. No services
(`Mode::Finite`). `fail_and_release.rs` lands in the same step so the
tree is not only the happy path — still no harness, still one file,
one test. Land `scripts/check-guide-quotes.sh` here, against the first
fence, so later pages inherit a working gate rather than a promised
one.

**1.3 README becomes the front door.** Rewrite the crate `README.md` to
one screen: what it is, the three crates, install, link to `docs/`. Move
the Stage 0–3 narrative out — it already lives in `StageNReport.md`.
Fix the crate-root rustdoc in `crates/sdax/src/lib.rs` that still claims
there is no `Plan::start`.

### Phase 2 — Mental model

Milestone: a reader who finished Quick Start can explain `needs`, why
`hold` exists, and why policy is an argument.

**2.1 Concepts.** Acquisition graph; the kind table (resource, step,
try-step, blocking step, service, effect, join, component, template);
`needs` as data dependency; `Policy` / `Shutdown` / `Mode` as required
intent; a `Plan` is `Send + Sync` and reusable, a run is not. One short
Python box: no waves, no elevator, no pre/exec/post on every node.

**2.2 Cleanup.** The Python "post_execute runs if pre started" page,
in Rust: `cx.hold` / `hold_value`, `Held` cannot be forged, release
graph is the reverse of `needs`, dropping `Running` still drains,
compensate vs persistent, the body that does the effect and then
`hold_value` has reopened the window. This is the page that prevents
leaks; keep it separate from Concepts so Quick Start can link it early.

### Phase 3 — Authoring and running

Independent once Concepts exists.

**3.1 Authoring.** Every kind with a short example: `resource`, `step`,
`try_step`, `blocking_step` + `pool`, `service` + `Serving`, `effect` +
`on_ambiguous`, `join`, `component`, `template` (declaration only).
Common attributes: `needs`, `within`, `Retry` / `Backoff`,
`exclusive` / `shared`, `idempotent`. Point at rustdoc for the full
chain.

**3.2 Running.** `use sdax_tokio::PlanStart`; `Running` as a `Future`
for the `Report`; `ready`, `shutdown`, `cancel`, `handle`, `snapshot`.
Lazy start (cancel-before-poll is a no-op). Drop = cancel + one tracked
drainer. Tokio facts a supervisor has to know: a blocking body that
never returns blocks `Runtime::drop`; the drainer needs a driver thread
(`current_thread` does not progress between `block_on`s); panics are
caught only if the profile unwinds. Concurrent `start`s of one `Plan`.
`resident_service.rs` is the test this page quotes.

**3.3 Errors.** `build` → `Invalid` / `Finding` / `Rule`: a user table
of the validate rules (what it means, the usual fix), not the decision
procedures. After a run: `Report`, `Outcome`, `Fault`,
`Report::into_result`. `FailFast` vs `Isolate`. Cleanup failures are
records, not lost. `validate_reject.rs` is the test this page quotes;
`fail_and_release.rs` already exists from 1.2.

### Phase 4 — The rest of the author surface

**4.1 Inspect and simulate.** `inspect()`: nodes, edges, layers,
release order, `why`, `effects`, `diff`. `Plan::simulate` as the way
to test *your* plan with a script of body outcomes — user-facing, not
a tour of `sdax-testkit`.

**4.2 Instances.** `template`, `cx.spawn`, `Child::{ready, stop, id}`,
the refusals a body sees (`ForeignTemplate`, `UndeclaredTemplate`,
`ScopeStopping`). User language for the two facts that surprise people:
every key an instance imports is released only after that instance
ends; a start body that awaits `Child::ready()` makes the parent's
readiness include the instance. `spawn_child.rs` is the complex
scenario: a resident parent, one template, one spawn, assert the
child became ready and the imported key outlived it. Still one file,
one test, no harness.

**4.3 Cookbook.** Three recipes rewritten from Python's use-case
section: a request-scoped finite plan, a resident service graph, a
pipeline with a blocking step. One paragraph on when not to use
(distributed job queues, unbounded CPU farms, anything that wants
waves or levels).

### Phase 5 — Close the loop

**5.1 rustdoc crate roots** of `sdax` and `sdax-tokio` point at `docs/`.
Broken intra-doc links stay a real defect (`RUSTDOCFLAGS=-D warnings`).

**5.2 Guide tests run and quotes match.**
`cargo test -p sdax-tokio --test guide` executes the target (the
workspace `cargo test` already includes it).
`scripts/check-guide-quotes.sh` is the other half: a passing test
whose fence drifted is still a failure. Add the script to the
"All gates" list in `AGENTS.md`. No `examples/` target, no extra CI
job.

**5.3 Workspace README** lists `sdax` as the Python prior-art member
(done as part of adding the repo; revisit if the table is still stale).

## 6. Out of scope

- A host-API / "write a Runtime" guide.
- Rendering `docs/` to GitHub Pages or mdBook.
- A Python-to-Rust porting guide.
- Publishing performance numbers.
- Documenting `sdax-testkit` as a product (it is `publish = false`).

## 7. Verification

Each phase is done when:

- the new pages exist and link from `docs/README.md`;
- every snippet on those pages is a `rust,guide:<name>` fence that
  `scripts/check-guide-quotes.sh` accepts, or a rustdoc-tested block;
- `cargo test -p sdax-tokio --test guide` is green;
- `docs/` still names no stage, no host engine type, and no review id;
- `README.md` still points at `docs/` and does not re-grow a status dump.

No browser pass: these are Markdown files, not a web app.
