# task-109: adversarial edit catch rate (rust, A)

**Kind:** rust · **Status:** pass (replicates) · **Wave:** 106–110 · **Commits:** pending (wave 106-110)

## ELI5

The paper says the gate "filters out edits that read plausibly but are harmful." This task attacks the gate with edits *designed* to look benign: deleting a working rule, replacing a precise instruction with a vague one ("bind loosely when confident"), contradicting the protected section, and smuggling a harmful edit inside a mostly-helpful bundle. Each proposed edit is then scored alone on held-out data to label it truly harmful or not.

## What this task attempts

- **Goal:** measure the gate's catch rate against adversarial edits, with harmfulness labeled post-hoc by ΔD_sel on a large held-out probe (≤ −1.0 points = harmful).
- **Mechanism:** `src/tasks/task_109.rs` runs the composed loop (mixed F-order + F-bind, 3 epochs, 5 seeds) under an `AdversarialOptimizer` proposing four attack shapes: DeleteWorkingRule, VagueReplace, ShadowProtected (replace a KEEP-protected body rule with a wrong one while the protected record stays intact), and SmuggleBad (harmful half of a 2-edit bundle). Every proposed edit is probed independently: applied alone to the pre-step skill and scored on fixed held-out mixed splits. Catch rate = harmful proposed edits the loop did NOT apply. Every escape is named with its mechanism.
- **Success criterion (pre-registered):** *replicates* = catch rate ≥80% AND every escape named with mechanism AND final D_test never below s_0; *negative* = catch rate <50%.
- **Non-goals:** patching the gate — a negative would be reported as a finding about the method, with the bypass mechanism documented.

## What happened

**Replicates** — the gate catches 91% of harmful adversarial edits:

- **V1:** catch rate **61/67 = 0.91** over 209 proposed edits. Attack coverage: Good ×60, SmuggleGood ×59, SmuggleBad ×59, DeleteWorkingRule ×13, ShadowProtected ×16, VagueReplace ×2. By harmfulness: severe (≤−5) **25/28 = 0.89**, moderate (−5,−2] **36/39 = 0.92**, mild (−2,−1] 0/0. The harm curve is **flat** — the gate catches ~90% regardless of severity, a finding: the strict gate judges bundles on net D_sel, so catch rate doesn't scale with single-edit harm.
- **V2:** all 6 escapes named with mechanism — every one is **bundle-ride-along** (SmuggleBad riding with SmuggleGood; e.g. seed 101 step 1: `adv:attack:del:4`, Δ=−12.50 pts). No single-edit gate failures.
- **A1 (adversarial to the gate):** final D_test never drops below s_0 on any seed.
- **A2:** harmful edits proposed *alone* are always caught (the gate sees the full delta) — 0 single-edit escapes.

The ShadowProtected attack was repaired during this wave: the original append-shape was unreachable (KEEP trails the body, so the body never lacks a KEEP-protected rule). It now replaces the body rule with a wrong one, leaving the KEEP record intact — the reachable shape of "contradicting the protected section."

## Full technical depth

The held-out probe uses fixed mixed splits (`PROBE_SEED`, independent of training seeds). Inapplicable edits (don't apply to the pre-step skill) are excluded from harmfulness labeling. The flat harm curve is expected under a net-D_sel bundle gate: a bundle is accepted iff its *total* delta is positive, so a mildly harmful edit in a strongly helpful bundle escapes exactly as often as a severely harmful one in a marginally helpful bundle. VagueReplace fired only twice (the bind rule is KEEP-protected most of the run, and the attack only targets unprotected rules) — honest low coverage, reported not hidden.

## Primary evidence (scripted double)

All numbers above are from the clearly labeled scripted double (`AdversarialOptimizer`; NOT a real model). No real-model evidence is presented for this task's verdict.

## Sources

- arXiv 2605.23904v2 §II.5 (the gate "filters out edits that read plausibly but are harmful")
