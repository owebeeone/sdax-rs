# Authoring remediation lane report

Date: 9 September 2026. Baseline: `ce339a59eefcd136562b16f4f20035c29ef0c988`.

## Scope and evidence label

This lane changed only the composition guide test and compact AI authoring
reference. The removed historical evaluation fixtures were neither present nor
accessed. The new cases reconstruct the exposed requirements from the remediation
plan, so they are regression guards, not exact reproductions or held-out evidence.
No engine behavior or report formatter changed.

## TDD record

1. The expanded guide initially failed to compile because it named a nonexistent
   `Port<T>` alias and treated `Fault` as if it had an `error` field. The public
   types are `Key<T>` for formal ports and `Fault::kind` for the retained error.
2. The first executable run passed normal and cleanup-failure behavior, while the
   permanent-failure assertion expected one report fault and observed three. The
   requirement is exactly two acquisition attempts plus upstream release, not one
   flattened fault record; the assertion now checks the rates fault, attempt count,
   and base release directly.
3. With those consumer corrections, all three composition tests passed against the
   unchanged API. This is authoring/test coverage, not an engine RED or behavioral
   fix.
4. Before the documentation update, `scripts/check-guide-quotes.sh` failed because
   `docs/AI-Authoring.md` still quoted the old component program. Replacing the
   compact reference with the complete executable file made the quote gate pass.

## Covered behavior

- ordinary component input is separate from a formally bound parent resource;
- output is the parent base plus each mount's rate;
- `Retry::attempts(2)` gives exactly two attempts on permanent failure;
- normal release order is derived, rates, then base;
- a derived cleanup error remains reported while rates and base cleanup continue;
- two mounts retain different inputs and independent retry state;
- output extraction calls `Report::into_result` first and renders the report only
  at an explicitly string-returning boundary.

The compact reference is under 16,000 UTF-8 bytes and retains the typed
`Plan<Out, In>`, `Key<T>`, `Arc<T>`, `Held<T>`, tuple dependency, existing-`Arc`,
unsized acquisition, effect ordering, retry, service, and diagnostic boundaries.
Generic unresolved operation identities are not claimed to be printable.
