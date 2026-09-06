# F2 — supplied input inspection and component admission

Date: 2026-09-06. Status: **implemented and locally verified**.
This follows F1's verified 359-test checkpoint and preserves its changes.

## Changes

`view.rs` now resolves an import to either a real/diagnostic path or a distinct
supplied-input marker. The marker is seeded only for the inspected root's input
and propagated through imports. It removes only already-supplied input waits.
Template-owned input still resolves to its template boundary; ordinary resource
imports, release ordering and unresolved foreign-input diagnostics remain visible.

`host/engine/table.rs` checks component inputs throughout the declaration tree
before flattening or any root effects. Component input is refused even when it
is unused or has type `()`, including under nested template declarations that
are never instantiated. Each template's own supplied input remains legal. The
existing `EngineError::TemplateAsScope` includes the full offending path, without
adding the root plan name. Existing root-input/import error precedence remains.
The public machine documentation and contract load section record the behavior;
the earlier acknowledged residue in `RootInput-Log.md` is marked resolved.

No new dependency, manifest change, runtime protocol change or component-input
API was introduced. `PlanBuilder::build` still constructs these declarations;
refusal occurs at machine load, start and simulation.

## Local-model experiment

Used the user's existing inference connection and model inventory, following the
provided client guide. No local model installation or server configuration change
was needed. Requests were serialized to respect the single shared GPU. No Spark
request or credit reset was made.

- **Qwen (`qwen3.8:27b`)** reviewed the original source and proposed algorithms.
  It supported explicit supplied-input provenance and recursive load validation,
  and identified nested-template/boundary cases worth testing. Those suggestions
  were checked against the Rust API and implemented as regressions. Its references
  to generic "build" refusal were interpreted as machine load, not authoring-time
  validation. No model response was accepted as evidence that tests passed.
- **Gemma (`gemma4:26b`)** received the patch and tests for an independent coverage
  review. It reached the 8192-token response limit with empty final content
  (`finish_reason: length`). This is an inconclusive review attempt, not a passing
  review or a general conclusion about the model's capability. No code changes
  were taken from it.

Raw prompts/responses were retained locally, outside the repository:

- `/var/folders/02/bn9c9g2x5qj8bb42zb857p7c0000gn/T/sdax-qwen-f2-review-or3ymywa/`
- `/var/folders/02/bn9c9g2x5qj8bb42zb857p7c0000gn/T/sdax-gemma-f2-review-3kc7adne/`

These are advisory reviews on a bounded task, not a controlled model benchmark.
Qwen was useful for this review; Gemma supplied no final answer in this setup.

## RED and GREEN evidence

The interrupted Spark view-test patch was restored and reviewed. Its template
fixture initially used an invalid finite root; that was corrected to a resident
root with an explicit simulated shutdown before evaluating the intended failure.
All three inspection regressions then failed on phantom `input` targets:

```text
cargo test -p sdax --test fixer_input_view --locked --offline
3 failed: Child/NeedsRequest, Component/Nested/NeedsRequest,
and Template/NeedsRequest pointed at nonexistent input
```

The new admission regression initially failed because `Machine::new` returned
an accepted planned machine for a component's unsupplied input. The valid unit
root/component control passed. Initial test-authoring errors involving a
non-Debug Running handle and the ScriptError import were corrected before this
behavioral RED.

```text
cargo test -p sdax-tokio --test fixer_input_admission --locked --offline
1 failed, 1 passed: called unwrap_err on Ok(Machine { run: Planned, ... })
```

After implementation, the inspection target has five passing tests:

1. Direct root-input import, with an ordinary resource edge retained.
2. Nested component imports with no phantom waits.
3. A template importing root input while retaining its own input boundary.
4. Unrelated foreign input retains its unresolved edge and refuses at load.
5. A template inside a component, whose child imports both root and template
   input: only the template-input boundary remains.

The adapter admission target has five passing tests:

1. Sixteen invalid declaration cases: both input constructors, used/unused input,
   direct/nested component placement and one/two template declaration levels.
   Every case checks both machine constructors, try_start, refused start reports,
   both simulation entry points, full paths, zero root acquisitions and zero
   tracked tasks.
2. A supplied unit root and an ordinary no-input component execute normally.
3. A supplied unit template input is actually consumed after cx.spawn.
4. Inspection and the independent scripted invariant checker agree with a real
   imported-input/resource computation producing 49 from 42 + 7.
5. Supplying the root's own declared input does not supply a component's input.

## Final gates

All eight baseline gates and the consumer check passed. Final total:
**369 test/doctest executions passed, 0 failed, 2 ignored**.
The ten new F2 tests include the sixteen-case admission matrix above.

| Command | Result |
|---|---|
| `cargo fmt --check` | PASS |
| `cargo test --workspace --locked --offline` | PASS |
| `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` | PASS |
| `cargo package -p sdax --locked --offline --allow-dirty` | PASS |
| `./scripts/check-architecture.sh` | PASS |
| `./scripts/compile-fail.sh` | PASS: 15 error-code witnesses |
| `./scripts/check-guide-quotes.sh` | PASS |
| `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --locked --offline` | PASS |
| `python3 scripts/check-consumer-guide.py` | PASS: positive and both negative controls |
| `git diff --check` | PASS |

Raw logs: `/var/folders/02/bn9c9g2x5qj8bb42zb857p7c0000gn/T/sdax-f2-final-0t6q_x02/`.
The commands and results above are the durable record if temporary logs disappear.
Rust 1.75 is not installed locally and its MSRV was not reverified; no hosted CI
run or actual adapter registry package verification is claimed.

## Disposition

F2a and F2b are implemented. F3 and the remaining F5 package/CI-release work are
next; earlier deferred release/package patches remain unreviewed drafts.
`SdaxFixer.md` is unchanged, SHA-256
`3b2192c453a55492d254dcb3bd10a89101ce1dfc3e30f149953bb1be65b307be`.
No commit, tag, push, publication or visibility change was performed.
