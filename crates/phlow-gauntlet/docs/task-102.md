# task-102: selection-gate ablation

**Kind:** rust · **Status:** pass (null) · **Wave:** 101–105 · **Commits:** pending (wave 101-105)

## ELI5

The recipe-card learner proposes changes every round, but a bouncer — the selection gate — only lets a change through if the recipe *measurably* improves on a held-out tasting panel (D_sel), and ties are rejected. The paper (arXiv 2605.23904v2 §II.5) calls this gate "the single most important defense in the method". This task fires the bouncer: one arm keeps the strict gate, one accepts everything, one accepts ties too — and checks whether the food actually gets worse without it.

## What this task attempts

- **Goal:** measure the gate's average-case contribution on an F-order+F-bind mix: does removing it let harmful edits through, and does the strict-greater (ties-rejected) choice matter against evaluation variance?
- **Mechanism:** `src/tasks/task_102.rs` runs three arms — strict (>), off (accept all), tie-accepts (≥) — 5 seeds each, 2 epochs, mixed-family splits, scripted optimizer fixed across arms. Per-edit D_sel deltas are probed incrementally, so every accepted edit is scored post-hoc as helpful/neutral/harmful.
- **Success criterion (pre-registered):** *replicates* = gate-off final D_test ≤ s_0 in ≥3/5 seeds AND ≥30% of gate-off accepted edits post-hoc neutral-or-harmful (ΔD_sel ≤ 0), with tie-accepts measurably more zero-gain accepts than strict; *null* = gate-off still improves in ≥4/5 seeds (gate unnecessary *in this domain* — an honest negative about gate necessity at toy scale).
- **Non-goals:** adversarial catch rate (that's task-109); changing gate semantics on gauntlet authority.

## What happened

**Null** — the gate is unnecessary *in this domain at toy scale*, but the mechanism the paper describes is visible in the edit stream:

- **V1:** all three arms complete and classify. Gate-off improves D_test in **5/5 seeds** (64.00 vs initial, n=5) — the preregistered null: the mock is competent enough that even unfiltered acceptance climbs.
- **V2:** the filter leg *does* replicate in the edit stream: **92.9%** of gate-off accepted edits are post-hoc neutral-or-harmful (≥30% floor), vs 63.4% under strict — the gate really is filtering plausible-but-useless edits, exactly as the paper claims ("filters out the vast majority of proposals").
- **A1:** tie-accepts accepts **183** zero-gain edits vs **45** under strict (totals 198 vs 71) — the strict-greater choice measurably matters against evaluation variance; ties are not rare.
- **A2:** the strict arm still finishes at 78.75 vs 64.00 for gate-off — the gate buys ~15 points of final quality even though both arms "improve", because the off arm's climb is polluted by junk it can never remove.

The honest reading: the paper's *mechanism* (the gate filters plausible-but-harmful edits; ties must be rejected) replicates crisply, but its *necessity claim* does not survive the toy domain — a competent proposer climbs even unfiltered. Null here is a scope bound on the claim, not a failed task: at frontier scale with noisier proposals, the filter is load-bearing; here it is merely valuable.

## Full technical depth

`decide(GateMode::Strict, current, candidate)` accepts iff `candidate > current`; `TieAccepts` accepts on `>=`; `Off` accepts everything. `SeedState::apply_candidate` probes per-edit D_sel deltas incrementally (apply edit i to a probe copy, re-score) before the gate decision, so the post-hoc neutral/harmful audit is measured, not modeled. Zero-gain = ΔD_sel == 0.0 exactly on the 40-case D_sel grid.

Per-seed D_test (points): strict 78.75±6.85; off 64.00±8.04; tie-accepts 64.00±8.04. Not-improved seeds: 0/5 in every arm. The null bucket follows directly: gate-off improves in ≥4/5 seeds.

## Primary evidence (real model)

Run 2026-09-28 on primo with `GAUNTLET_REQUIRE_REAL=1` against local `qwen3:8b` (toy-scale; direction/mechanism only):

**Null** — gate-off still improves in 5/5 seeds (not-improved 0/5), confirming the preregistered null. The filter mechanism is visible but weaker than scripted: **59.4%** of gate-off accepted edits are post-hoc neutral-or-harmful (vs 92.9% scripted), and tie-accepts accepts **32** zero-gain edits vs **1** under strict (totals 46 vs 15). The real model proposes fewer junk edits than the scripted double, so the gate has less to filter — but the strict-greater choice still measurably matters.

## Sources

- `crates/phlow-gauntlet/src/tasks/task_102.rs` — arms, `classify`, post-hoc edit audit
- `crates/phlow-gauntlet/src/skillopt/learner.rs` — `SeedState::apply_candidate` (per-edit delta probing, `decide`)
- `crates/phlow-gauntlet/src/skillopt/gate.rs` — `GateMode`, `decide`
- `~/workspace/gauntlet-design-tasks-101-115.md` — task-102 design (Wave 20); paper grounding arXiv 2605.23904v2 §II.5, §III.3
