# task-101: edit-budget bound ablation

**Kind:** rust · **Status:** pass (negative) · **Wave:** 101–105 · **Commits:** pending (wave 101-105)

## ELI5

Imagine learning to cook by rewriting a recipe card. Each round you're allowed to change at most 4 lines — or 1 line, or 16 lines, or the whole card. The paper this replicates (arXiv 2605.23904v2) claims the *existence* of a limit matters more than its exact size: no limit at all costs you 2–4 points, but 1 vs 4 vs 16 are all about the same. This task checks that claim by running the same learning loop four ways — only the per-step edit budget changes — and measuring the final recipe quality on a held-out test set.

## What this task attempts

- **Goal:** isolate the marginal contribution of the textual learning rate L_t (max edits accepted per optimizer step) on F-order, with the selection gate held fixed at strict-greater.
- **Mechanism:** `src/tasks/task_101.rs` runs four arms — L_t=1, L_t=4 (cosine 4→2, the paper default), L_t=16, unbounded (no truncation) — 5 seeds each, 2 epochs, scripted target F-order, fixed D_tr/D_sel/D_test. The optimizer is fixed across arms and arm-independent: the model is always asked for the same fixed maximum proposals and the learner truncates to L_t, so the bound under test never leaks into the proposal budget.
- **Success criterion (pre-registered):** *replicates* = unbounded costs ≥2.0 D_test points vs L_t=4 AND the bounded spread (max−min over {1,4,16}) is <2.0 points; *null* = unbounded within ±1.0 of L_t=4; *negative* = unbounded beats L_t=4 by ≥2.0.
- **Non-goals:** tuning the schedule (that's the paper's job); product changes on gauntlet authority (never).

## What happened

**Negative** — removing the bound *helps* at toy scale with the scripted double, the opposite of the paper's claim:

- **V1:** all four arms complete and classify. Unbounded **beats** L_t=4 by **6.50 points** (53.50 vs 47.00, n=5) — the preregistered negative bucket (≥2.0). The paper's "removing the bound costs 2–4 points" does not replicate here.
- **V2:** the bounded spread is **25.50 points**, not <2.0 — L_t=1 reaches 72.50 while L_t=4 sits at 47.00. The "all competitive" leg does *not* replicate with the scripted double either.
- **A1:** per-step churn confirms the "rewrite everything" pathology exists in miniature: unbounded applies 1.56 edits/step at 50 chars/step vs 0.60 edits at 21 chars/step for L_t=1 — but the strict gate (held fixed for isolation) keeps the damage bounded, and larger bundles apply more independent fixes per step.
- **A2:** L_t=16 (53.50) is identical to unbounded (53.50) — the mock never proposes more than 6 edits, so a bound of 16 never binds. The unbounded arm is only unbounded relative to a 6-proposal ceiling.

The honest reading: with the strict gate held fixed, the *gate* — not the bound — is doing the safety work. The mock's edits are independent appends (one ORDER[p] rule per failing profile) with no destructive interference, so bundling more of them per step covers more ground and the gate filters the net. The paper's cost-of-unbounded was measured with a frontier model whose rewrites can destructively interfere; the mock cannot produce that pathology, so the bound looks unnecessary here. The mechanism that survives: bounds shape *how many* independent fixes land per step (0.60 vs 1.56 applied/step), and tight bounds are surgical when rank-1 dominates — L_t=1 keeps only the best edit while larger bundles dilute it.

## Full technical depth

`LtSchedule::Unbounded` varies *only* the truncation (`take = proposals.len()`); acceptance still goes through the configured strict gate — single-variable isolation (the gate is task-102's dimension). `ModelOptimizer::reflection_prompt` requests a fixed `PROPOSALS_MAX` (6) edits on every arm; the learner truncates to L_t, so proposal budgets are arm-independent. The scripted mock ranks its rank-1 template good with probability `SCRIPTED_COMPETENCE` ≈ 0.6 and fills ranks 2–6 by weighted sampling over remaining templates (good and distractor) — hence the steep rank falloff that makes L_t=1 dominant.

Per-seed D_test (points): L_t=1: 72.50±17.25; L_t=4: 47.00±16.69; L_t=16: 53.50±3.00; unbounded: 53.50±3.00. The verdict buckets: not replicates (spread 25.5 ≥ 2.0), not null (|−6.5| > 1.0), negative (−6.5 ≤ −2.0) → negative, honestly. (The mock's rank-1 competence is exactly `SCRIPTED_COMPETENCE` = 0.6 — the fallback draw is restricted to distractors so good templates cannot leak in through the fallback branch.)

## Primary evidence (real model)

Run 2026-09-28 on primo with `GAUNTLET_REQUIRE_REAL=1` against local `qwen3:8b` (toy-scale; direction/mechanism only):

**Null** — the real model erases the scripted differences. All four arms score **8.50±2.00 pts** (n=5): unbounded cost vs L_t=4 is **+0.00**, bounded spread **0.00**. The model applies only **0.14 edits/step** at **9 chars/step** — far more conservative than the scripted double (0.60–1.56 edits, 21–50 chars). With so few edits proposed, the bound never binds and the L_t settings are indistinguishable. The scripted negative (unbounded beats L_t=4 by 6.5) does not survive contact with the real optimizer: qwen3:8b's proposals are too sparse for the bound to matter.

## Sources

- `crates/phlow-gauntlet/src/tasks/task_101.rs` — arms, `classify`, evidence lines
- `crates/phlow-gauntlet/src/skillopt/learner.rs` — `SeedState::apply_candidate` (bound = truncation only; gate fixed)
- `crates/phlow-gauntlet/src/skillopt/optimizer.rs` — arm-independent prompt (`PROPOSALS_MAX`), `ScriptedOptimizer` rank falloff
- `~/workspace/gauntlet-design-tasks-101-115.md` — task-101 design (Wave 20); paper grounding arXiv 2605.23904v2 §II.4, §III.3
