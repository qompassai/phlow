# task-107: cross-family skill transfer (rust, V)

**Kind:** rust · **Status:** pass (null) · **Wave:** 106–110 · **Commits:** pending (wave 106-110)

## ELI5

The paper says a skill trained on one benchmark transfers to another: smaller gains than in-domain, but positive. This task freezes a skill evolved on one toy family and scores it on the other. The catch: the evolved skill's *text* can smuggle target-family lines inside its bundles — so the task strips those ride-along lines and re-scores, isolating genuine content portability from bundle contamination.

## What this task attempts

- **Goal:** freeze the F-order skill, score on F-bind (and reverse); measure raw transfer vs a no-skill baseline, then decontaminate and re-score.
- **Mechanism:** `src/tasks/task_107.rs` evolves a skill on the source family to convergence, freezes the exact body text, and scores it with the target family's independent verifier. Contamination analysis detects target-family lines riding along in source-family bundles (e.g. `ORDER[p]` lines inside a frozen F-bind skill), reports per-seed contamination counts, strips those lines, and recomputes the decontaminated transfer score. Verdicts classify on the *decontaminated* score.
- **Success criterion (pre-registered):** *paper-like* = target-family D_test gain >0 AND <50% of in-domain gain; *negative* = target-family score drops below the no-skill baseline.
- **Non-goals:** skill *discovery* — with scripted doubles this measures **content portability, not skill discovery**; the full transfer claim needs a real model.

## What happened

**Null (both directions)** — the apparent transfer is entirely bundle contamination:

- **V1:** transfer classifies on decontaminated scores. FOrder→FBind: in-domain gain **39.00** pts; raw transfer **0.00 ± 0.00**; contamination **[0,0,0,0,0]**; decontaminated **0.00** → null. FBind→FOrder: in-domain gain **100.00** pts; raw transfer **13.50 ± 11.02** pts (looks paper-like: >0 and <50% of in-domain); contamination **[2,2,3,3,1]** lines per seed; decontaminated **0.00** → null.
- **V2:** in-domain gains are positive in both directions (the source skills actually learned).
- **A1 (adversarial to the transfer claim):** no direction drops below the no-skill baseline — the paper's "no setting drops below baseline" holds.
- **A2:** provenance is preserved through freeze/transfer (the frozen artifact is byte-identical).

The mechanism: the scripted optimizer can propose order rules while processing bind failures (bind trajectories still carry profiles); accepted bind bundles therefore carry D_sel-neutral order edits. A frozen F-bind skill for seed 101 contained `ORDER[7]` and `ORDER[0]` lines alongside the exact bind rule. Strip those lines and the "transfer" vanishes. This is **content portability, not skill discovery** — and here, not even portability: it's contamination.

## Full technical depth

Contamination detection matches target-family line patterns (`ORDER[p]:` inside F-bind skills, the exact bind line inside F-order skills) against the frozen body. Decontamination strips exactly those lines and re-scores with the target verifier; the in-domain score is unaffected (the lines were D_sel-neutral in the source family). The 13.50-point raw transfer in FBind→FOrder is the mean over 5 seeds of a contaminated artifact; per-seed raw scores correlate with contamination count.

## Primary evidence (scripted double)

All numbers above are from the clearly labeled scripted double (`ScriptedOptimizer`; NOT a real model). No real-model evidence is presented for this task's verdict.

## Sources

- arXiv 2605.23904v2 §III.4 (cross-benchmark transfer: "uniformly positive" but "smaller than in-domain by an order of magnitude")
