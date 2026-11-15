# Rung 3 corpus — results (PH-2 / PH-5 measurement run, 2026-11-15)

Origin lane: qompassai. Content label: sfw. Evidence: this file, the
manifest, the labels, and the two proposal files beside it; the outcome
rows themselves live in the corpus store named below.

## What ran

24 bounded phlow runs against the PH-10 torogao seat, driven by
`run_corpus.py` from the plan in `PLAN.md` (pre-registered before any
run: task list, split by index parity within class, reviewer rotation,
budgets). Two task classes, 12 runs each:

- **analysis** — read-only questions on the seat tree itself
  (`~/.local/share/pax/torogao-ph10-wt-20261114`), checked by the
  seat's three named checks (theodolite_zero, toolchain_pin,
  tree_freeze).
- **code_fix** — one planted defect per run in a throwaway clone of
  the seat tree at the same commit
  (`torogao-corpus-scratch-20261115`, cde6a69), restored and removed
  after each run; the seat tree was never written.

Two probes were included by design: F10 (brief asserts a defect that
does not exist; correct behaviour is to change nothing) and A11
(brief asks about a file that does not exist; correct behaviour is to
report that, not fabricate).

One pilot run (A0) was executed first with the installed
`~/.local/bin/phlow` and produced **no outcome row**: that binary is
dated Nov 10 and contains no PH-1 recording code (verified by string
scan). The corpus therefore ran with a binary built from the landed
tree (`target/debug/phlow` at 8947d0d + this rung's branch, whose
runtime code is untouched). The pilot is not counted in the 24.

## Outcome rows

24 rows in `~/.config/phlow/corpus-rung3/configs/feedback.db`
(the store sits beside the corpus configs by PH-1's rule). Per-run
reports and role digests are in `~/.config/phlow/corpus-rung3/reports/`;
the run-by-run manifest is `results/manifest-draft.jsonl` here.

| Class | Runs | Phlow ok | Truth success | Truth failure |
|---|---|---|---|---|
| analysis | 12 | 11 | 12 | 0 |
| code_fix | 12 | 1 | 11 | 1 |

Ground truth was assigned independently of phlow's own verdicts:

- code_fix truth is mechanical: after the run, the scratch tree must
  contain the pristine string, not the planted string, and no
  forbidden file (F1). F10's truth: tree byte-identical to pristine.
- analysis truth is a verifier read of each run's role digests against
  the tree facts recorded in `tasks.json` before the runs. All twelve
  answers were substantively correct — including A11, whose coder
  correctly reported the missing file. (Minor blemishes noted, not
  scored as failures: A6's coder echoed the planner's correct answer
  rather than deriving it; A8's digest misspells one path prefix; the
  set it names is exact.)

## Headline finding: the review loop, not the work

The coder's work product was correct on 23 of 24 runs. Yet only one
code_fix run (the F10 probe) ended `ok`/`tool_checked`:

- Every real fix run burned both cycles (11–36 model calls for
  one-line fixes) and ended `failed` or `unverified` with
  verification strength `self_reported`, even though the final
  recorded checks were green on 10 of 11 (F0–F7, F9, F11) and the
  mechanical truth was success.
- F1: the fix was correct and all checks passed; the reviewer
  returned `not_approved` with an **empty summary**, twice.
- F2: cycle 1 was approved by the reviewer with an accurate summary,
  the run still went to cycle 2, and the same reviewer then marked
  `not_approved` while its summary said the file "is correct as-is".
- A11: the truthful negative answer was rejected by the reviewer
  ("a concrete failure that needs to be repaired") — a false
  rejection of correct behaviour.
- F8, the one genuine failure: the coder role itself errored with no
  output and changed nothing; the reviewer correctly rejected, and
  `tree_freeze` correctly failed on the still-planted tree.

The precise internal sequencing (why cycle approval failed on runs
whose final checks and, in F2's case, cycle-1 review were green) is
**not** established by these artifacts and is flagged for PH-3 rather
than guessed at here. What is established: in this corpus the binding
constraint on run success is the review/verification loop, not the
coder and not the host checks.

A second, smaller finding for the fleet-checks record: `checks_json`
collapses every non-zero check exit to `failed`, so theodolite's
exit 2 ("unproven — producer precondition") is indistinguishable
from exit 1 (real diagnostics) in the outcome row. The per-run
reports keep the distinction; the row does not.

## PH-2 — budget fitting (`results/budget-proposal.md`)

Fit on fit-split successes (nearest-rank p95 × 5/4), replayed on the
held-out split. **Held-out successes cut: 0 of 12** (the binding
gate). Neither class needed the disclosed escalation.

| Class | Fit successes | max_iterations | max_cycles | max_tool_calls | Run timeout |
|---|---|---|---|---|---|
| analysis | 6 | 27 | 2 | 23 | 310 s |
| code_fix | 5 | 39 | 3 | 33 | 1616 s |

Wasted-turn reduction on held-out failures: **0 of 0 (0.0%)** — the
held-out split contained no failed runs (the corpus's single genuine
failure, F8, fell in the fit split under the pre-registered parity
rule), so the saving is unmeasured rather than zero-valued. A future
corpus should stratify failures across the split if this number is
wanted. The code_fix timeout is dominated by F0's 1292 s wall (the
first fix run, cold models and a cold diagnostics producer); treat
that one number as the least stable in the table.

## PH-5 — reviewer calibration (`results/reviewer-calibration.md`)

Eight scored verdicts per reviewer (rotation by design):

| Reviewer | Agreement | False appr. | False rej. | Verdict |
|---|---|---|---|---|
| hf-granite-4.2-8b-tools-64k | 0.88 | n/a¹ | 0.12 | calibrated |
| hf-devstral-24b-tools-64k | 0.88 | 0.00 | 0.14 | calibrated |
| hf-gpt-oss-20b-tools-64k | **0.75** | n/a¹ | 0.25 | **demote to advisory (proposed)** |

¹ The model scored no truth-failure runs, so the rate is undefined,
not zero. No reviewer produced a single false approval in 24 runs;
the failure mode is over-rejection, which burns cycles instead of
passing bad work. The demotion proposal's interim and durable forms
are spelled out in the calibration file; nothing is applied by any
tool.

## GPU courtesy

Every run was preceded by the harness's courtesy gate (rose `/api/ps`
plus two `nvidia-smi` samples). Recorded utilisation at run starts:
0–52%; two runs waited one 60 s courtesy cycle (A8, F5). No run
started over a hot GPU, and no second large model was loaded while
one was resident (the trio's roles share the already-resident set).

## Designed around / not proposed

- Installed phlow binary predates PH-1 (above) — corpus used a
  tree-built binary; the installed one should be refreshed when Pax
  lands this rung.
- `task_class` is hardcoded `flow_run` in the runtime until PH-6;
  class membership lives in the manifest, by design.
- Outcome rows record the reviewer only via the manifest (the row
  carries model ids as a set, not per role).
- No budget or demotion is applied anywhere. Both files are
  proposals for the operator's hand, per PH-9.
