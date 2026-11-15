# PH-2 / PH-5 measurement corpus — plan (fixed before any run)

Written 2026-11-15, before the corpus runs. The split rule, the
reviewer rotation, and the truth rules below were fixed here, in
advance; the fitting code never sees a choice made after the data
arrived.

## Design

- **24 runs**, two task classes, 12 each:
  - `analysis` — read-only questions about the PH-10 seat tree
    (torogao worktree at cde6a69), run against the seat workspace
    with the seat's three named checks (theodolite_zero,
    toolchain_pin, tree_freeze) in a corpus copy of the seat config.
  - `code_fix` — one planted single-line defect per run in a scratch
    clone of the seat tree at the same commit, with the same three
    named checks repointed at the scratch tree. The correct fix
    restores the pristine line, so a correct run also satisfies the
    freeze check. Scratch worktrees are removed afterwards; the seat
    tree is never edited.
- **Roles**: planner `hf-gpt-oss-20b-tools-64k`, coder
  `hf-devstral-24b-tools-64k` for every run. The **reviewer rotates**
  through the trio — granite-4.2-8b, gpt-oss-20b, devstral-24b — by
  task index within each class (`rotation[i % 3]`), so each reviewer
  model serves 8 runs (4 per class).
- **Split**: within each class, even task indices are `fit`, odd are
  `held_out` (6 + 6 per class). The rotation interleaves so every
  reviewer appears in both splits.
- **Budgets during the corpus**: max_iterations 24, max_cycles 2,
  max_tool_calls 128 — above what these small tasks naturally use,
  so successful runs record untruncated usage.
- **Probes**: each class carries two designed probes (a false-premise
  and a nonexistent-target task for analysis; an already-correct and
  a symptom-only task for code_fix). Probes test whether the
  pipeline fabricates success; their ground truth is assigned by the
  verifier's reading, like every analysis run.

## Ground truth (independent of the pipeline's own verdict)

- `code_fix`: mechanical and exact — after the run, the target file
  must contain the pristine line and must not contain the planted
  line (plus, for the module-rename defect, no substitute file may
  exist). The probe with no defect succeeds only if the tree is
  byte-identical to pristine.
- `analysis`: the verifier reads each run's role summaries from the
  saved report and scores the stated answer against the tree fact
  recorded in `tasks.json`. A run that fabricates (invents a file,
  confirms a false premise) is a failure even when its checks pass —
  that is the escape case PH-5 exists to measure.
- The pre-existing rung-2 seat-proof row in the store predates this
  plan and is **not** part of the corpus; it stays unlabelled and is
  counted as such by the proposal tool.

## Pre-registered measurement notes

- The outcome row's `checks_json` collapses the fleet checks' exit
  codes to `ok`/`failed`: theodolite's exit 2 (unproven — producer
  precondition) is indistinguishable from exit 1 (real diagnostics)
  in the row. A bacon producer therefore watches the scratch tree
  during code_fix runs (the seat's normal operating condition), so
  the in-run check reads fresh diagnostics. The calibration report
  scores verdicts against ground truth and the host-gate label, not
  against the raw per-check strings, for this reason.
- `max_iterations` is fitted from recorded **model calls**; the row
  carries no separate iteration counter.
- Rows do not record which model reviewed; the manifest carries the
  reviewer per run from the config that served it.

## GPU courtesy

Before every run the harness reads rose's `/api/ps` and samples
`nvidia-smi` twice, 5 s apart. If GPU utilisation is above 70% on
either sample (the factory lane mid-render), it waits 60 s and
re-samples, up to 10 waits, then proceeds and records the decision
in the manifest note. Runs are sequential; no second large model is
loaded while one outside the trio is resident.
