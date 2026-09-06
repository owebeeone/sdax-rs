# Doc-writer handoff — 2026-09-06

Written by the manager after verifying the two landed changes (root plan input, and
the current-thread drain acknowledgement) against the tree. **Nothing in `docs/` was
edited to produce this list** — the pages are the doc writer's lane. Detail lives in
`dev-docs/UserDocs-GuideTests-Log.md` and `dev-docs/CurrentThreadDrain-Log.md`; this
file is the consolidated ask, plus one finding neither log carries.

State at the time of writing: **331 tests, all eight gates green**, verified
independently by the manager. Uncommitted.

---

## 1. One page is actually broken

`docs/Reference.md:210-218`. The fence builds a `current_thread` runtime and then
calls `TokioRuntime::new`, **which now panics**:

```
let tokio_rt = tokio::runtime::Builder::new_current_thread()
    ...
let rt = Arc::new(TokioRuntime::new(tokio_rt.handle().clone()));
```

`TokioRuntime::new` now reads `Handle::runtime_flavor()` and refuses a
`current_thread` handle, naming the constructor that takes one. The fix is the
substitution `TokioRuntime::new` → `TokioRuntime::current_thread_no_background_drain`.

This is the only code in `docs/` that a reader could copy and have fail.

## 2. Three prose spots, no longer wrong but no longer whole

| where | what changed under it |
|---|---|
| `docs/Reference.md:220` | "`TokioRuntime::new(Handle)` does not own the runtime." Still true. It is no longer the only thing worth saying about that constructor, which now also picks the flavour. |
| `docs/Reference.md:242-247` | The `Drop` / drainer paragraph is correct and now has a **constructor to name** rather than a fact the reader must carry. |
| `docs/Running.md:21-27` | Same: the drainer prose frames the hazard as something to remember. It is now something the type system says at the call site. |

Two softer notes from the guide work, unrelated to the constructor:

- `docs/Inspect.md` describes two capabilities its scenario does not demonstrate.
- `docs/Cleanup.md`'s prose is narrower than what its scenario now asserts.

## 3. The structural finding — `Reference.md` is the only unguarded page

Fence census across all eleven pages:

| page | guide-quoted fences | plain `rust` fences |
|---|---|---|
| `Reference.md` | 0 | **6** |
| the other ten | 7 total (one each on seven) | **0** |

Every line of Rust in `docs/` is compiled and byte-locked to a test **except**
the six fences in `Reference.md`, which nothing checks. That page is 274 lines,
more than a quarter of the corpus.

**The page is not rotten.** Five of the six are signature listings, and the
manager checked every entry against source today:

| listing | verified against | result |
|---|---|---|
| `Plan<Out = (), In = ()>` | `plan.rs:269` | exact |
| nine builder entries | `builder.rs:119,140,155,173,181,289,298,311,321` | exact, including `build`'s four arguments |
| seven node constructors | `shorthand.rs:23,45,56,72,86,100,124` | exact |
| `PlanStart`'s four methods | `running.rs:407-443` | accurate; the page writes `In` where the trait declares `Self::Input`, which the impl binds to `In`. Fair for a reference sheet |
| `Report<Out>` and `Ok \| Failed \| Cancelled` | `report.rs:280-295`, `report.rs:132-139` | exact, field for field, in order |

So the risk is not present rot. It is that **the one fence on that page which was
real executable code drifted the moment the API moved**, and no gate said a word.
The other five cannot drift silently in the same way only because they are not
programs.

**Recommendation, for the owner rather than the doc writer:** promote the snippet
at :210 to an eighth guide scenario so it compiles like the rest. The five
signature listings are better left as listings — they are a reference sheet, not
code — but retagging them so they do not read as copyable programs is worth a
thought.

## 4. What not to touch

The seven `rust,guide:<name>` fences are **byte-equal** to
`crates/sdax-tokio/tests/guide/<name>.rs` and `./scripts/check-guide-quotes.sh`
enforces it. Editing a fence breaks the gate. The rule is one way only:

> edit the test, run `cargo fmt`, then copy the file into the fence.

Prose around a fence is free to change; the fence body is not.
