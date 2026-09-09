# API authoring EXP5: corrected fixture namespace

The EXP4 test omitted an explicit explanation that supplied fixture types live
in `crate::support`. EXP5 added that explanation to initial and repair prompts
and reran all five models against the same task, source and scoring inputs.

| Model | EXP4 accepted /8 | EXP5 accepted /8 | Old API A /4 | Revised API B /4 |
|---|---:|---:|---:|---:|
| Gemma4:26b | 2 | 2 | 1 | 1 |
| Qwen3.8:27b | 1 | 4 | 2 | 2 |
| gpt-5.6-luna | 3 | 3 | 1 | 2 |
| gpt-5.6-terra | 6 | 7 | 3 | 4 |
| gpt-5.6-sol | 7 | 8 | 4 | 4 |

Acceptance requires compilation, behavioral/diagnostic assertions and every
applicable independent source-review criterion. Unassessed is not accepted.
Initial executable passes rose from **1/40 to 14/40**; fully accepted cells
rose from **19/40 to 24/40**. Executable tests alone passed 28/40; source review
excluded four for isolation or independent-output guarantees.

The namespace defect materially confounded EXP4. Qwen benefited from clearer
instructions, but scored equally across the APIs. Gemma used fewer attempts
without improving final acceptance. Both local models still failed F1/F2's
complex effect/recovery tasks. Two local initial cells still omitted fixture
imports; no cloud submission had that error.

Within EXP5, revised API/reference B scored 13/20 versus A's 11/20. One
trajectory per model/task/arm is insufficient to establish a reliable API
advantage. Cloud and local model settings/context differ, and cloud token/cost
usage is unavailable. Counts are 38 local calls and 47 scored cloud submissions,
not a combined API-call count. One unsolicited Terra alternate initial answer
was retained but never evaluated; transport recovery is documented in evidence.

Qualification passed 34 tests and 12 controls. Source A is
`711004f739e71fd437bbb8474109a9099099b13e`; source B is
`a2a601366ebc5083d4129b0a4c55a7886f430862`. The runner also enclosed its generated
conditional assertion module in braces; original criteria and source inputs
were preserved. Independent preparation and final-source reviews completed.

Use this qualified rerun in place of the confounded EXP4 model comparison,
while retaining EXP4 as historical evidence. It motivates improving executable
coverage of independent-output guarantees; it does not itself justify another
API change. No product implementation changed in this rerun or archive commit.

[Full report, protocol, raw records and provenance](https://github.com/owebeeone/sdax-core-evidence/tree/main/campaigns/api-authoring-comparison/runs/2026-09-09-exp5-namespace-correction)
require access to the **private evidence repository**. Public builds and tests
do not depend on it. Freeze SHA-256:
`c8127752856fa2f1442080b6c68782d7146f27a960b394fc4c5c259b70390128`.
