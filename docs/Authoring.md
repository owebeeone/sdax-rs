# Authoring

Start the builder with one of three constructors. `Plan::builder(name)`
declares no input; start that plan `plan.start(rt, ())`.
`Plan::with_input::<In>(name)` declares a per-run input;
`p.input()` is the `Key<In>` a node `needs`, and each `start(rt, value)`
supplies that run's value. `Plan::template::<In>(name)` is the same
constructor under the name the child case is written with — the built
plan can still be started as a root. An input is not itself a runnable
node: a plan whose only nodes are its input and its imports is empty
(`V-EMPTY`).

A node joins the plan only when its **terminal** method runs. Omitting a
required piece is a compile error, not a finding: a resource without
`release` never yields a `Key`; an effect without `on_ambiguous` has no
`perform`; a blocking step without `on(pool)` has no `run`.

Attribute order is free. `.within(d).needs(k)` and `.needs(k).within(d)`
record the same declaration. A key must exist before it is named.

The chain for each kind, then the attributes every kind may take.
Signatures are in rustdoc.

| Kind | Chain |
|---|---|
| resource | `.resource(name).needs(…).acquire(\|cx, deps\| …).release(…)` or `.release(release::by_drop())` |
| step | `.step(name).needs(…).run(\|cx, deps\| …)` |
| try-step | `.try_step(name).needs(…).run(…)` — dependents receive `Arc<Result<T, Error>>`; something must need this key |
| blocking step | `.blocking_step(name).on(pool).run(\|cx, deps\| …)` — `run` is synchronous |
| service | `.service(name).needs(…).stop_within(d).start(\|cx, deps\| …)` — return `Serving::new(handle, serve)` |
| effect | `.effect(name).on_ambiguous(Ambiguity::…).perform(…).compensate(…)` or `.persistent()` |
| join | `.join(name, deps)` — ready when every need is ready |
| component | `.component(name, &child_plan)` — instantiated once per parent run |
| template | `Plan::template::<In>(name)` to write the child; `.template(name, &plan)` to register it |

## Common attributes

| Attribute | Meaning |
|---|---|
| `.needs(keys)` | data dependency and start eligibility |
| `.within(d)` | bound the prepare / run body |
| `.retry(Retry::attempts(n).backoff(…))` | re-run the prepare body; `attempts` is the total, not "retries after the first" |
| `.idempotent()` | you assert re-execution is safe; required for some retry / ambiguity choices |
| `.exclusive(res)` / `.shared(res)` | take a resource this node already needs |
| `.limit(pool)` | bound this node's concurrency (distinct from a blocking step's `.on(pool)`) |
| `.cooperative(grace)` | signal `cx.stop()`, keep polling for the grace, then abort — so an in-flight `hold` can still register |

Services also have `.restart(Restart::on_error(…))`, `.terminal()` (this
service finishing ends the scope), and `.spawns(&template)`.

`Ambiguity` is required on every effect: `Report`, `Compensate`, or
`Retry`. The last two require `.idempotent()`.

The [Quick Start](QuickStart.md) test is an input, a resource and a step. A
service is [Running](Running.md). A template is [Instances](Instances.md).
A blocking step is in the [Cookbook](Cookbook.md) pipeline. A plan that
`build` rejects is [Errors](Errors.md).
