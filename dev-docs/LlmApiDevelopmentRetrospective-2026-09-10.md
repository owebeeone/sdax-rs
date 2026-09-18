# Building APIs with LLMs: the SDAX retrospective

10 September 2026. Recollection, checked against selected project records;
recommendations for subsequent APIs, not a new review or release assessment.

## Overall judgment

This was a successful experiment in using LLMs to develop substantial software.
The useful result is more than generated Rust: a coherent API, explicit lifecycle
contracts, an implementation tested at several boundaries, and evidence that
other agents can learn to author against it. The process also exposed mistakes
in our evaluation and workspace management. Those are part of the result.

My judgment is that SDAX is good enough to put to use. We should carry forward
the methods that made correctness and usability assessable, while reducing
repeated setup, duplicated paperwork and review without a concrete decision.
There is no measured estimate here of hours or money saved by each method.

I do not retain a complete personal memory of every earlier agent turn. The
early chronology below is reconstructed from the preserved design records;
the recent authoring experiments and workspace corrections are also present in
the conversation. Statements about what to do next time are recommendations,
not claims that this project already followed a perfectly repeatable method.

## The starting point: a real problem, Python prior art and an evaluator

The initial problem was repeated startup, failure, cancellation and shutdown
logic in Glade. We had real hand-written lifecycle code and the existing Python
SDAX implementation. That gave the designers concrete things to replace and
preserve, rather than an unconstrained request to invent an elegant framework.

The Python version made the intended value visible: task dependencies,
initialization, execution, teardown, retry and concurrent runs could be described
and centrally orchestrated. Its implementation and tests were available as a
behavioral reference. The Rust brief explicitly treated Python SDAX 0.7.1 as
inspiration and prior art, not a required architecture or literal port. Rust
ownership and typed references could change the representation substantially.

An important qualification: having a Python implementation is not the same as
having an independent differential oracle. The initial charter explicitly says
no differential harness existed. I cannot substantiate a claim that Rust was
systematically run against Python with matching inputs and compared outputs.
The later documented differential checks compared the Rust scripted driver
with the Tokio driver. We should preserve the distinction between a conceptual
model, an executable example, and an independently checked oracle.

The **Declarative API Evaluator v3** was the design instrument. Its original
filename said V2, but its contents were v3; the design workspace preserved it as
`DeclarativeApiEvaluatorV3.md`. It guided generation and comparison; it was not
itself a compiler that mechanically generated the API.

The most valuable parts of that instrument were these questions:

- What does the engine derive that application authors otherwise have to write?
- Can a declaration's meaning be stated independently of its implementation?
- Can a plan be inspected before effects occur? Is meaningful ordering explicit?
- Where are invalid programs rejected, and which wrong programs remain valid?
- What happens at the boundary between declarations and imperative body code?
- Are composition, explanations and escape boundaries predictable?
- Does the API express legitimate unseen needs, or only the examples used to
  design it? Does apparent safety come from excluding useful work?
- How much specification must an author read, and how costly is repairing an
  error using the actual diagnostics?

These are stronger design constraints than “make it concise” or “make it
declarative.” The evaluator deliberately separates declarative power from
correctness and distinguishes projected judgments from measurements. That
helped prevent a polished scorecard from becoming a claim of proven quality.

## How the initial design was selected

The `sdax-v1` exercise supplied two independent designers with the same frozen
brief and corpus: 42 intents, with 28 available for design and 14 held out.
Peer separation was instruction-based on a shared filesystem, not enforced
confinement. The manager assessed the designs before reading their self-scores.

Both designers converged on much of the architecture: immutable plans, typed
builders and dependencies, distinct resources/steps/services/effects, explicit
readiness, engine ownership of acquired values, a pure state machine and a
runtime adapter. That convergence was useful evidence of a stable direction;
the process did not need to manufacture two radically different answers.

The recommendation favored B, `stagehand`, as the semantic base. A, `lifecycle`,
had declarations reported about 40% shorter on the held-out set, but B had
executed witnesses for important runtime claims, including registration in the
completing poll and bounded cleanup. Compact syntax did not outweigh a missing
mechanism for a central safety claim. Three separable ideas from A were retained:
positional shorthand, the raw-spawn lint gate, and plan simulation/effect views.
The recommendation explicitly avoided blending incompatible semantics.

The design review also found gaps shared by both candidates. Agreement between
agents was therefore helpful, but not sufficient. Their scores were projections,
not a calibrated ranking against alternatives. The optional further design
revision round was not used: the findings were clear enough to move to an owner
decision. That is an early example of a sensible stopping point.

## What went well and should remain

### Contracts and architectural boundaries preceded implementation

Readiness versus completion, resource release versus effect compensation,
ambiguous success versus known failure, and in-process cleanup versus crash
recovery were made explicit. These distinctions constrain implementation and
prevent broad words such as “safe” or “rollback” from hiding incompatible ideas.

The pure machine/runtime-adapter/testkit split gave the project a fast way to
reason about scheduling and lifecycle semantics separately from Tokio behavior.
It also created a real boundary to test. The Stage 2 report records 58 shared
conformance cases on the adapter and fourteen plan/script comparisons between
drivers. Equal normalized traces are stronger evidence than two suites merely
reporting success, although shared machinery still limits independence.

For another API, retain one authoritative semantic contract, a narrow substrate
boundary, and tests at that boundary. Do not require this exact three-crate
architecture when a simpler domain does not need it.

### Small executable witnesses settled difficult design claims

Compiler witnesses and native runtime probes made some early architectural
claims assessable before a full engine existed. That was a particularly good
use of LLM labor: propose a mechanism, make the critical claim executable, then
choose a design with the result in hand.

Likewise, test-first implementation made regressions concrete. A useful failing
test expresses a requirement independently of the intended fix; an assertion
that merely mirrors generated code supplies little additional confidence.

### Adversarial findings became structural improvements

The external Stage 1 review observed that roughly fifteen machine defects were
largely three rules omitted on different terminal paths. Consolidating fault
flushes, component terminal observations and scope settlement reduced the places
that could forget those rules. The recorded refactor preserved the existing
suite and replayed pinned failures, rather than weakening expectations to fit
the new shape.

That is more valuable than counting reviews or bugs. The desired chain is:
counterexample, explicit invariant, regression witness, then a representation
or helper that makes recurrence harder. When possible, make invalid states
unrepresentable or reject them at a single validation boundary.

### Usability was tested by authors other than the implementer

An author who designed an API has too much unstated knowledge to be its only
documentation test. Fresh model contexts exposed missing information, awkward
builder transitions and excessive boundary boilerplate.

The experiments also separated possible remedies. Focused documentation alone
had limited results. A compiling scaffold helped Qwen substantially, but that
scaffold supplied graph structure, declaration order and policy as well as type
shapes. It was evidence for the complete starting package, not proof that adding
type annotations alone solved the problem.

The subsequent changes were proportionate: configuration forwarding on
`IdentifiedEffect`, and `Report::into_required_output` with full report retention.
They removed repeated authoring friction without changing the execution model.
A further mounting API was deferred because the evidence did not justify it.
That restraint is worth repeating.

### The evaluator was allowed to be wrong

EXP4 omitted an explicit fixture namespace explanation. EXP5 corrected the
environment contract and reran all five models with the original criteria.
Fully accepted cells rose from 19/40 to 24/40; initial executable passes rose
from 1/40 to 14/40. Gemma stayed at 2/8 accepted, Qwen rose from 1/8 to 4/8,
Luna stayed at 3/8, Terra rose from 6/8 to 7/8, and Sol rose from 7/8 to 8/8.

The lesson is methodological: test failures can implicate the prompt, fixture,
budget, model, documentation or API. They do not identify the cause by themselves.
The corrected run still had four executable passes rejected by source review
for isolation or independent-output guarantees. Neither the harness nor the
reviewer alone was a sufficient judge.

### Durable evidence made correction possible

Frozen inputs, exact responses, failed attempts, diagnostic repairs and source
hashes let us revisit the result without relying on a conversational summary.
The private evidence repository now separates that record from the public
product. Build outputs remain external; concise conclusions remain discoverable
in product documentation. This should be the arrangement from the first run of
the next project, rather than a later migration.

## What to cut or reduce

These are proposed changes to future work. They do not retrospectively change
the evaluator's rules or this project's acceptance criteria.

| Practice | Keep | Reduce or replace |
|---|---|---|
| Design exploration | Two independent proposals when the architecture is genuinely uncertain; one concrete adversarial comparison | Repeated competing designs after the semantic choice is settled; cosmetic alternatives |
| Declarative evaluator | Meaning, derivation, error boundaries, composition, coverage and evidence status | Full scorecards for routine local changes. Use a short adaptation explicitly labeled as such; do not claim formal evaluator compliance |
| Python/reference implementation | Concrete behavior, realistic examples and a small independent semantic model where useful | Maintaining a second complete production engine solely to call it an oracle; literal cross-language ports |
| Reviews | A bounded review at a meaningful semantic boundary; targeted follow-up on unresolved defects | Reopening the entire design after documentation edits or clean closure of an unchanged requirement |
| Regression checks | Deterministic cases, negative controls, pinned failing seeds and native substrate checks | Re-running unchanged broad stress suites without a new failure hypothesis; tests that restate implementation |
| Authoring experiments | A qualified smoke test, then a bounded representative comparison with clear repair accounting | A full model matrix for every wording edit; optimizing until the weakest model is perfect |
| Documentation | One normative contract, a short reference, executable starters and dated decisions | Multiple living summaries repeating the same mutable state; a new handoff that silently contradicts old ones |
| Evidence | Exact inputs/results, a manifest, one outcome summary and provenance | Repeated source bundles where a retained revision plus patch suffices; build caches in evidence; copying every historical campaign into every new freeze |
| Parallel work | Independent tasks with explicit write boundaries, valid clone lanes and an integration owner | Agents contending over the same files; treating plain Git clones as GWZ workspaces; fixing ambiguous paths after work begins |
| Model selection | Match capability to the consequence of mistakes; measure useful completed work | Assuming the lowest-priced model is cheapest overall, or that a stronger model removes the need for checks |

The workspace mistakes were avoidable process failures, including mine. Work was
confused between stale Documents paths and the real limbo workspace, and clone
layout was mistaken for GWZ workspace structure. The useful response is a
verified root, supported tool operations and structural rejection of invalid
layouts. More prose asking agents to “be careful” is a weaker defense.

Reducing paperwork must not reduce honesty. Keep the failed attempt and its
reason; reduce the number of places that retell it. Generate inventories and
status from authoritative records where practical. Historical documents should
be marked as historical rather than maintained as competing current truths.

## A leaner method for the next API

This is a reusable sequence, not an instruction to begin another project now.

1. **State the job and boundaries.** Name the repeated user code being removed,
   the useful behavior that must remain expressible, the non-goals, and where
   effects become externally observable. Collect real examples before designs.
2. **Apply the evaluator's core questions.** Specify meaning, engine-derived
   work, inspectability, ordering, validation and imperative escape boundaries.
   Keep author convenience separate from correctness and implementation cost.
3. **Resolve only consequential design uncertainty.** Use independent proposals
   if needed; require small executable witnesses for risky substrate claims.
   Choose one coherent semantics and record the rejected tradeoff once.
4. **Build the smallest meaningful vertical slice.** Include one normal path,
   one failure/recovery path and one composition case. If a Python model exists,
   state exactly which semantics overlap. Compare normalized outcomes only
   where the contract permits equivalence; add Rust-specific ownership tests.
5. **Make the slice self-checking.** Use meaningful negative controls and shared
   conformance at replaceable boundaries. Pin discovered failing traces. Expand
   the implementation through the same checks instead of inventing new gates
   independently in every lane.
6. **Qualify the authoring environment before measuring authors.** Run a known
   correct answer through the exact evaluator and prompt packaging. State module
   layout, imports, supplied types, output shape and repair rules explicitly.
   Exercise one real author response before a larger campaign.
7. **Test representative authoring, then fix the dominant friction.** Use fresh
   contexts, a capable reference author and a lower-cost target author. Inspect
   whether failures concern syntax, discovery, semantics or the evaluator.
   Prefer documentation/helpers over new abstraction when they solve the issue.
8. **Close with a bounded adversarial pass and use the API.** Resolve defects
   that violate the contract. Record residual limitations. Run another broad
   cycle only for a new mechanism, counterexample or material uncertainty.

For a small follow-on API, steps may share one short document and one compact
test suite. If we choose a smaller corpus or omit formal calibration, call the
result an engineering evaluation, not a measured frontier ranking under v3.

## How to know when to stop

The target should be correct useful programs, clear rejection of common mistakes
and affordable repair—not perfect first-attempt generation by every model.
We should stop this sort of polishing when the required scenarios work, important
invariants have credible checks, representative authors can recover from errors,
and remaining failures do not reveal an unresolved contract defect.

That does not make a semantic violation acceptable merely because it is rare.
It separates a release-blocking engine defect from an author failing to express
a valid program under a small budget. Gemma's remaining difficulties are a
useful usability signal; they are not automatically an API release veto.

The next evidence should come from actual integrations and maintenance edits:
can users compose features, diagnose failures, change policies and preserve
behavior? Add those real surprises to the corpus. Reopen an API decision when
the new evidence challenges it, rather than because another review is possible.

We have evidence for the quality of SDAX's own contracts and for useful authoring
improvements. We do not yet have a like-for-like benchmark establishing that it
beats all alternatives. The process lesson is strong without that claim: LLMs
became effective collaborators when intent, executable feedback, independent
challenge and retained evidence constrained their plausible answers.

## Records used

These links identify the basis for the recollection. Sibling-member links assume
the SDAX workspace layout; they are not product build dependencies.

- [Original owner brief](../../sdax-v1/brief/00-OwnerBrief.md),
  [evaluation charter](../../sdax-v1/brief/01-EvaluationCharter.md) and
  [Declarative API Evaluator v3](../../sdax-v1/brief/DeclarativeApiEvaluatorV3.md).
- [Design comparison](../../sdax-v1/reviews/Comparison.md) and
  [handoff, including evidence/projection limits](../../sdax-v1/reviews/Handoff.md).
- [Python SDAX](../../sdax/README.md): prior art; its documented promises are not
  automatically claims about the Rust contract.
- [Stage 1 external review and consolidation](Review-2026-09-06-External.md),
  [Stage 1 TDD record](Stage1-TDD-Log.md) and
  [Stage 2 conformance/differential report](Stage2Report.md).
- [Documentation experiment](AuthoringDocumentationResults-2026-09-09.md),
  [scaffold experiment](AuthoringScaffoldResults-2026-09-09.md),
  [implemented authoring helpers](ApiAuthoringImplementation-2026-09-09.md) and
  [corrected EXP5 comparison](ApiAuthoringExp5Results-2026-09-10.md).
- [Evidence policy](../../EVIDENCE.md) and
  [private archive guidance](../../sdax-core-evidence/README.md).
  Raw experiment records require access to the private evidence member.
