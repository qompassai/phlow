# task-104: slow-meta ablation

**Kind:** rust · **Status:** pass (indeterminate) · **Wave:** 101–105 · **Commits:** pending (wave 101-105)

## ELI5

The recipe learner has two kinds of long-term memory. At the end of each week (epoch), the *slow update* writes lessons into a protected notebook section ("keep doing X, stop doing Y"), and the *meta skill* quietly tracks which *kinds* of changes have helped or hurt — momentum for the learner itself. The paper (arXiv 2605.23904v2 §II.6, §III.3) reports removing both costs 22.5 points on SpreadsheetBench, "the single largest ablation effect in the paper", because "the system effectively forgets across epochs". This task removes each memory, then both, on the long-horizon F-ledger family — and checks whether anything is actually forgotten.

## What this task attempts

- **Goal:** attribute the slow update's and meta skill's marginal contributions on F-ledger (chosen because multi-rule retention is where forgetting would show): four arms — full / no-slow / no-meta / neither — 3 epochs, 5 seeds.
- **Mechanism:** `src/tasks/task_104.rs` runs the four arms with the scripted mock fixed. Forgetting is measured *directly*: canonical rules accepted in epoch 1 are snapshotted (`epoch1_canonical`), and retention = fraction still present in the final document. The mock carries a genuine forgetting mechanism: a "replace canonical with narrow" distractor template that the slow update's KEEP lines are meant to block.
- **Success criterion (pre-registered):** *replicates* = neither costs ≥8.0 D_test points vs full AND full retains ≥80% of epoch-1 rules while neither retains <50%; *null* = neither within ±2.0 of full.
- **Non-goals:** attacking the slow update's integrity (that's task-114); changing memory semantics on gauntlet authority.

## What happened

**Indeterminate** — the score cost replicates, but the *forgetting* does not occur, and the mechanism behind the cost is different from the paper's story:

- **V1:** all four arms complete and classify. Neither costs **+8.50 points** vs full (26.50 vs 35.00) — the ≥8.0 floor is met. No-slow alone costs the full 8.50; no-meta costs 0.00 — the slow update carries the entire effect.
- **V2:** epoch-1 retention is **100% in every arm** — the retention-collapse leg does not materialize. Full does not "retain ≥80% while neither collapses"; nothing is forgotten anywhere.
- **A1 (adversarial to the design's retention story):** the retention collapse is *structurally inoperative* here — the strict D_sel gate already rejects the canonical→narrow replacement (it never improves D_sel), so the KEEP lines have nothing to prevent. The slow update's measured +8.5 comes from its GUIDE lines instead: the full arm's final documents carry strictly more GUIDE-unlocked twist-rule templates (2 vs 0 in the neither arm), and those twist rules are worth the points. The gain is guidance, not retention.
- **A2:** neither also loses on D_sel (41.00 vs 60.00) — the cost is real and selection-visible, not test-set noise.

The honest reading: the paper's *forgetting* mechanism does not replicate in this setup — not because the apparatus is broken, but because the strict gate (task-102's dimension, held fixed here) already does the retention work the KEEP lines were designed for. What the slow update actually contributes here is *guidance*: longitudinal notes that unlock template families the step-level optimizer cannot invent. That is a real, measured +8.5-point contribution with a different mechanism than the paper's "forgetting" story — hence indeterminate, not replicates.

## Full technical depth

`slow_update` diffs the epoch-start and epoch-end skills on D_sel, classifies lines into newly-fixed / newly-regressed / consistently-correct, and writes KEEP/GUIDE lines into the protected section; `canonical_lines` snapshots accepted canonical LEDGER rules at end of epoch 1. The mock's `ledger:repl:{i}` template proposes replacing canonical with narrow variants (good=false, weight 0.25) unless the direction is buffer-suppressed or the line is KEEP-protected. Across 5 neither-arm seeds the replacement was proposed twice (seeds 104/105, step 5, epoch 1) and rejected by the gate both times — the displacement cannot pass strict D_sel, so retention stays 100%.

Per-arm D_test (points): full 35.00±36.61; no-slow 26.50±37.47; no-meta 35.00±36.61; neither 26.50±37.47. Twist rules in final bodies: full 2, neither 0. Verdict: cost leg met (+8.5 ≥ 8.0), retention leg absent (100% everywhere, not <50%) → indeterminate.

## Primary evidence (real model)

Run 2026-09-28 on primo with `GAUNTLET_REQUIRE_REAL=1` against local `qwen3:8b` (toy-scale; direction/mechanism only):

**Null** — the real model accepts no epoch-1 rules in any arm (all D_test 0.00), so retention is undefined (NaN%) and the neither-vs-full cost is +0.00. The scripted finding (full beats neither by 8.5 via GUIDE-unlocked rules, both retain 100%) does not replicate because qwen3:8b is too conservative to commit to the ledger rules at all. The retention-collapse the preregistration demands remains unobserved with either backend — the genuine forgetting mechanism is still unimplemented (see the summary's critical unresolved deviation).

## Sources

- `crates/phlow-gauntlet/src/tasks/task_104.rs` — arms, `classify`, `retention`, `slow_gain_is_guidance_not_retention`
- `crates/phlow-gauntlet/src/skillopt/learner.rs` — `SeedState::epoch_end` (`slow_update`, meta), `epoch1_canonical`
- `crates/phlow-gauntlet/src/skillopt/optimizer.rs` — `ledger:repl:{i}` forgetting template, GUIDE-unlocked twist templates
- `~/workspace/gauntlet-design-tasks-101-115.md` — task-104 design (Wave 20); paper grounding arXiv 2605.23904v2 §II.6, §III.3
