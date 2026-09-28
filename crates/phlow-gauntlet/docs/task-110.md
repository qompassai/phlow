# task-110: poisoned rollout evidence (rust, A)

**Kind:** rust · **Status:** pass (negative) · **Wave:** 106–110 · **Commits:** pending (wave 106-110)

## ELI5

The optimizer learns from rollout evidence — but what if the evidence lies? This task poisons the rollout batch at 0%, 10%, 20%: failure traces that blame the *wrong* cause (a failure caused by missing evidence-binding whose trace blames tool order), and lucky successes (wrong behavior, reward 1 through a verifier gap). The scripted optimizer is *supposed* to fall for the poison at the reflection stage; the question is whether the gate and the rejected-edit buffer contain the damage.

## What this task attempts

- **Goal:** at each poisoning level, count accepted poisoned edits; check the rejected buffer records the poisoned directions and whether that record suppresses later re-proposals.
- **Mechanism:** `src/tasks/task_110.rs` runs the F-bind loop (3 epochs, 5 seeds) at 0%/10%/20% trajectory poisoning with the normal `ScriptedOptimizer` (not told the poison labels — it must fall for poisoned evidence at reflection; the test is whether downstream controls catch it). Attribution is seed-bound: `ProposalNote` carries `(seed, step, direction, poison_induced, suppressed)` via `ReflectCtx.seed`, so tainted acceptances are counted per (seed, step) pair, not per step index (step indices repeat across seeds and would overcount).
- **Success criterion (pre-registered):** *contained* = at 20%, final D_test ≥ s_0 AND accepted poisoned edits = 0; *negative* = ≥1 poisoned edit accepted at 10% (mechanism named: which defense failed).
- **Non-goals:** real-model evidence; patching the gate on a negative.

## What happened

**Negative** — the gate accepted a poison-contaminated proposal at 20%:

- **V1:** poison 0%: mean D_test **100.00**, 0 considerations. Poison 10%: **100.00**, 0 considerations — **vacuous** (fast convergence; no poisoned failure reached reflection). Poison 20%: **100.00**, 2 poison considerations, **1 tainted (seed, step)**, **1 tainted acceptance**, direction **`order:add:0`**; buffer rejections 1, suppressions 1. Design bars: *contained* requires 0 tainted acceptances at 20% — failed. The design's *negative* bar names 10% (0 there); classified Negative as the closest verdict with the level named explicitly.
- **V2:** the buffer records and helps — at 20% it recorded 1 poisoned-direction rejection and suppressed 1 later re-proposal.
- **A1 (adversarial to containment):** no D_test regression at 20% (0.00 points) — the damage was epistemic, not behavioral.
- **A2:** the accepted poison-induced direction was `order:add:0` — the *correct* ORDER[0] rule, proposed for the wrong reason (a poisoned F-bind failure blamed profile 0). Epistemic contamination without measured F-bind damage: the gate failure is real (contaminated evidence entered the skill), but the content is benign, which is why the regression check stays green.

Which defense failed: the **gate** (it accepted a poison-contaminated candidate on net D_sel — the order edit is score-neutral on F-bind fixtures, so the bundle passed). Reflection fell for the poison as designed (that's the mock's job); the buffer did its job (recorded + suppressed).

## Full technical depth

Seed-bound attribution was a wave fix: the original arm-global attribution aggregated by step index, overcounting (2 tainted steps, 5 acceptances); per-(seed, step) counting gives the honest 1 and 1. The 10% arm is vacuous because F-bind converges in ~2 epochs on the mock — poisoned trajectories only reach reflection when failures persist, which at 10% they don't. A slower-converging family would make 10% non-vacuous; flagged, not patched.

## Primary evidence (scripted double)

All numbers above are from the clearly labeled scripted double (`ScriptedOptimizer`, not told poisoning labels; NOT a real model). No real-model evidence is presented for this task's verdict.

## Sources

- arXiv 2605.23904v2 §II.3 (the backward pass "converts evidence from the forward pass into concrete edit proposals")
