# Root input — TDD log

`start(rt, input)`: a **root** plan may declare a per-run input and be started
with one. Same convention as the stage logs (LBT-007): one row per step, the
test written first, the RED evidence quoted from what the compiler or the runner
actually said, and the change that made it GREEN.

## What was wrong

`Plan<Out, In>` carried the input type parameter, `PlanBuilder::input()` declared
the node, and the machine already delivered a per-run input value — but only to
a template instance. `PlanStart` was implemented for `Plan<Out>` alone
(`In = ()`), `Table::build` refused **any** plan carrying a `Kind::Input` node at
the root, and `PlanBuilder::input()`'s rustdoc said root plans have no input.

That contradicts the design corpus. `I-04` — build the per-request orchestration
once, run it for thousands of concurrent requests, *each with its own typed
state* — is the headline intent, and both proposals answered it with a plan
typed on its input and a `start` that takes one. What an author could reach
instead was a captured `Arc<Mutex<..>>` popped per run: shared across every run,
not per-run state, and precisely the untyped shared-context shape the design
rejected.

**`I-04`'s conformance row passed the whole time.**
`c04_two_runs_of_one_plan_value_share_nothing` asserts *isolation* — two runs of
one `Plan` value share no slots, locks or pools — which the code does provide,
and its two runs never needed different inputs to prove it. A half-met intent
that a green suite could not see. What now prevents it is
`i04_one_plan_two_runs_each_with_its_own_typed_input`
(`crates/sdax-tokio/tests/root_input.rs`): one `Plan<Response, Request>` built
once, started twice with **different** inputs, each answer checked against *its
own* input, and no `Mutex`, `Cell` or other shared cell anywhere in the test — so
the only way a run can differ from its sibling is the value handed to `start`.

## Log

| # | test written first | RED | GREEN |
|---|---|---|---|
| 1 | `crates/sdax-testkit/tests/conformance/root_input.rs` (new, included by **both** binaries): `C-70`, a root plan taking a per-run `u8` with `Cfg` needing it and `Work` needing `Cfg`, run through `Drv::run_with_input`; `C-71`, the same plan offered to a driver that supplies no value and refused. `crates/sdax-tokio/tests/root_input.rs` (new): the `I-04` row above, sixteen concurrent runs each keeping their own input, `try_start(rt, ())` on a plan with no input, and the instance half of the slot rule. `crates/sdax/src/tests/instances.rs`: `an_input_bearing_plan_runs_only_when_its_value_is_supplied` | `error[E0599]: no associated function or constant named 'with_input' found for struct 'sdax::Plan<Out, In>'` (3×); `no associated function or constant named 'run_with_input' found for struct 'ScriptedDriver'` / `'TokioDriver'`; `error[E0599]: no method named 'start' found for struct 'sdax::Plan<Out, In>'` (3×); `error[E0061]: this method takes 1 argument but 2 arguments were supplied` (2×); `no associated function … named 'with_input' … for struct 'Machine'` | `Plan::with_input::<In>(name)` declares the input node and `Plan::template::<In>` is that same constructor under the template case's name — one `Plan<Out, In>` value is startable both ways. `Table::build` takes a `RootInput` (`Supplied` / `Absent`); the `Kind::Input` refusal fires only for `Absent`. `Machine::new<Out, In>` is `Absent` (generalised over `In` so a real template can be *offered* and refused), `Machine::with_input<Out, In>` is `Supplied`. `bodies_of_with_input::<Out, In>(plan, input)` seeds the root scope's slot with `Arc<In>` **at construction**, before the source reaches a driver, which is the same moment `open_instance` fills an instance's. `PlanStart` gains `type Input` and every entry (`start`, `try_start`, `start_with`, `try_start_with`) takes it; the impl is `Plan<Out, In>` with `In: Send + Sync + 'static` (`OD-SPAWN-INPUT`'s bound, kept consistent). `Simulator::with_input`, `Plan::simulate_with_input`, `ScriptedBodies::with_input`, `ScriptedDriver::run_with_input` and `TokioDriver::run_with_input` are the matching entry points; the value is dropped in the scripted ones, which read no slot, and that is said in their rustdoc. `PlanStart::Input` carries the `Send + Sync + 'static` bound in the trait, not only in the impl, so the rustdoc's claim is enforced rather than promised. 22 existing call sites became `start(rt, ())` / `start_with(rt, (), opts)`. `an_input_bearing_plan_runs_only_when_its_value_is_supplied` pins the split at both ends: `Machine::new` vs `Machine::with_input`, and `Plan::simulate` (`Err`) vs `Plan::simulate_with_input` (a trace with `Cfg` in it and no `input`) |
| 2 | `C-70` again, now compiling | **`INV-1: Cfg started at #0 while its need input is not Ready`** — the independent checker, from the *view*: `PlanView` resolved a need on the root's own input to a node it had already skipped, so the edge pointed at a node that does not exist | `view.rs`: a need on **this** plan's own input is not an edge when the plan is the root — the value is in the run's slots before the first step, so there is nothing to wait for and nothing to show. Inside a template or a component the same key stands for the node that instantiates the plan, which the resolver records, so that case is untouched. This is what `Table::flatten` already did for the run; the view now agrees with it, and `why("Cfg")` answers "waits for nothing" |
| 3 | `a_spawned_instance_body_receives_the_input_cx_spawn_was_given`: a template whose inner step `needs` the input stores what it read; the run asserts it read `42` | **`assertion left == right failed: the instance's body read the input cx.spawn supplied / left: 0 / right: 42`** — verified by reverting the fix alone and re-running | a latent bug the root work would otherwise have replicated. A slot holds `Arc<T>` for every node (`Cx::register`, `Slots::get::<T>`), and `OD-SPAWN-INPUT` says so in as many words — but `Cx::spawn` boxed the bare `I`, so `Deps::fetch` downcast to `Arc<I>`, got `None`, and the body was **never built**. No test reached it: the scripted source documents that it never looks at the input, and no tokio test spawned an instance whose body read one. `Cx::spawn` now boxes `Arc::new(input)`; `Cx::spawn_instance`, `Scope::spawn_instance` and `BodySource::open_instance` say in their rustdoc that the box holds `Arc<I>` |
| 4 | the Monte Carlo walk: some generated root plans declare a per-run input with a `Cfg` step that needs it, and two coverage floors — `a root plan takes a per-run input` and `the root input's reader ran` | written as a floor, not a failure: the corner did not exist, so the walk could not reach it | `mc/gen.rs`: `Plan::with_input::<()>` on a 0.30 coin, a `Cfg` step needing it, and `Generated::root_input` so the runner can count it. `In = ()` keeps `Generated::plan` one type, so the walk still covers both shapes in one stream; the typed value is what the `I-04` row proves and what a random `u8` would only repeat. Both walks now start every case the way an author starts a root run — `run_with_input(&plan, (), &script)` — and `TokioDriver::run` carries an `allow(dead_code)`, because the two binaries that include that file use different halves of it |
| 5 | `monte_carlo_big`, 50 000 cases on two seeds | — | `SDAX_MC_SEED=1`: clean, `a root plan takes a per-run input` **14 347** (floor 300), `the root input's reader ran` **13 749** (floor 200), every other floor cleared. `SDAX_MC_SEED=20260906`: clean, **14 032** and **13 455**. The default 3 000-case fast loop reaches 895 and 858, and the adapter walk is clean too |
| 6 | no new test — the gates | — | `cargo fmt`; clippy `-D warnings` (one `cmp_owned` in the new machine test, rewritten to compare `NodePath`s); `crates/sdax/tests/surface.rs` pins `bodies_of_with_input` at its intended path. Contract § 1 gains the input's vocabulary, § 5's seam row says the slot holds `Arc<In>`, § 7 gains the `L-INPUT` load gate, § 10 gains driver obligation 6 (the renumbering of "a run never ends in silence" to 7 is carried through its one cross-reference), and the Decisions table gains **OD-ROOT-INPUT**, dated 2026-09-06 |

## What is deliberately not here

- **`RunOptions::bodies`.** A body source supplied by a harness owns its own slot
  tables, so it owns what the input means to it; `try_start_with` seeds the
  plan's own source and nothing else. Said in `PlanStart`'s rustdoc and in
  contract § 10, obligation 6. `TokioDriver::run_with_input` is the case: it
  passes a real value to `start_with` and then hands the driver `ScriptedBodies`,
  which reads no slot.
- **A template used as a *component*.** This was a pre-existing residue of the
  root-input change and is now resolved by F2 (2026-09-06). Machine admission
  recursively checks every component, including components inside nested template
  declarations, and refuses its declared input before root effects. Unit and
  unused inputs receive no implicit value. `SdaxFixer-Input-Log.md` records the
  regression matrix and verification. The component API still supplies no input.
