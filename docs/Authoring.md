# Authoring

Start a reusable definition with `Plan::with_input::<In>(name)`.
`Plan::builder(name)` is the ordinary unit-input form, exactly equivalent to
`Plan::with_input::<()>(name)`. Every builder exposes `p.input()` as a
`Key<In>`. Supply input with `plan.start(rt, value)`, bind it when mounting
with `parent.component(name, &plan, input_key)`, or supply it when spawning
with `cx.spawn(&template, value)`. A unit-input mount accepts explicit `()`.
Inputs do not run bodies or carry cleanup obligations.

Declare additional typed imports with `child.port::<T>(name)`. Bind each
one for a parent using `let bound = child_plan.bind(port, parent_key)?`, then
mount `&bound`. A binding preserves the parent's lifetime until the child
finishes cleanup. The same definition may be mounted multiple times or
bound under different parents. Concrete `import(parent_key)` remains useful
when deliberately constructing a child for one known parent.

Beginning a node chain reserves its declaration. Its **terminal** completes
that declaration and returns its key. A resource without `release` cannot
produce a `Key`; an effect without `on_ambiguous` has no `perform`; a blocking
step without `on(pool)` has no `run`. Discarding an incomplete chain makes
`build` return a finding naming the node and its missing terminal.

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
| service | `.service(name).needs(…).stop_within(d).initialize(\|cx, deps\| …).serve(\|cx, handle\| …)` — initialize one stable handle, then serve it |
| effect | `.effect(name).on_ambiguous(Ambiguity::…).perform(…).compensate(…)` or `.persistent()` |
| join | `.join(name, deps)` — ready when every need is ready |
| component | `.component(name, &child_plan, input_key)` (or `()` for unit) — instantiated once per parent run |
| template | `Plan::with_input::<In>(name)` to write the child; `.template(name, &plan)` to register it |

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

`Ambiguity` is required on every effect: `Report`, `Recover`, or `Retry`.
`Recover` requires `.idempotent()`, `.identified_by(key)`, and
`.recover_unknown(handler)`; `Retry` requires `.idempotent()`.

The [Quick Start](QuickStart.md) test is an input, a resource and a step. A
service is [Running](Running.md). A template is [Instances](Instances.md).
A blocking step is in the [Cookbook](Cookbook.md) pipeline. A plan that
`build` rejects is [Errors](Errors.md).
