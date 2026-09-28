# task-104: slow-meta ablation

**Kind:** rust · **Status:** pass (indeterminate) · **Wave:** 101–105 · **Commits:** pending (wave 101-105 + repair)

## ELI5

The recipe learner has two kinds of long-term memory. At the end of each week (epoch), the *slow update* writes lessons into a protected notebook section ("keep doing X, stop doing Y"), and the *meta skill* quietly tracks which *kinds* of changes have helped or hurt — momentum for the learner itself. The paper (arXiv 2605.23904v2 §II.6, §III.3) reports removing both costs 22.5 points on SpreadsheetBench, "the single largest ablation effect in the paper", because "the system effectively forgets across epochs". This task removes each memory, then both, on the long-horizon F-ledger family — and checks whether anything is actually forgotten.

## What this task attempts

- **Goal:** attribute the slow update's and meta skill's marginal contributions on F-ledger (chosen because multi-rule retention is where forgetting would show): four arms — full / no-slow / no-meta / neither — 3 epochs, 5 seeds.
- **Mechanism:** `src/tasks/task_104.rs` runs the four arms with the scripted mock fixed. Forgetting is measured *directly*: canonical rules accepted in epoch 1 are snapshotted (`epoch1_canonical`), and retention = fraction still present in the final document. The mock carries a genuine forgetting mechanism: in later epochs, without slow-update guidance, it rediscovers one twist rule from persistent twist failures and *bundles* it with canonical→narrow replaces for unprotected slots. The twist fix is a strict D_sel improvement, so the strict gate genuinely accepts the bundle on net and the epoch-1 canonical rules are displaced as a side effect. The slow update's KEEP lines block the replaces at proposal time (full arm), and the meta buffer suppresses re-proposals of burned directions.
- **Success criterion (pre-registered):** *replicates* = neither costs ≥8.0 D_test points vs full AND full retains ≥80% of epoch-1 rules while neither retains <50%; *null* = neither within ±2.0 of full.
- **Non-goals:** attacking the slow update's integrity (that's task-114); changing memory semantics on gauntlet authority.

## What happened

**Indeterminate** — the *forgetting* replicates, but the *score cost* only partly does:

- **V1:** all four arms complete and classify. Neither costs **5.00 points** vs full (30.00 vs 35.00) — the ≥8.0 floor is not met, and ±2.0 is not met either. Verdict: indeterminate. No-slow alone costs the full 5.00; no-meta costs 0.00 — the slow update carries the entire effect; the meta skill contributes nothing measurable here.
- **V2:** the full arm retains **100%** of epoch-1 canonical rules — the ≥80% leg holds. The slow update's KEEP lines block every replacement proposal.
- **A1 (adversarial to the design's retention story):** the neither arm's retention collapses to **33%** (seeds 104/105 lose their epoch-1 canonical rules to the accepted twist-rediscovery bundle; seed 102's already-saturated D_sel=1.0 cannot be displaced). The collapse is measured, not assumed: the displacements pass through the unchanged strict D_sel gate on genuine strict improvement. The preregistered <50% leg holds.
- **A2:** neither also loses on D_sel (54.00 vs 60.00) — the cost is real and selection-visible, not test-set noise.

The honest reading: the paper's *forgetting* mechanism replicates at the toy scale — without slow/meta protection, epoch-1 rules are genuinely displaced through an accepted gate (100% → 33% retention), while KEEP lines preserve them in the full arm. But the *score* effect (5.0 points) lands in the indeterminate band: the rediscovered twist rule that enables the displacement also fixes twist cases, so the neither arm recovers some of the cost the forgetting inflicts. Retention replicates; magnitude does not — hence indeterminate, not replicates.

## Full technical depth

`slow_update` diffs the epoch-start and epoch-end skills on D_sel, classifies lines into newly-fixed / newly-regressed / consistently-correct, and writes KEEP/GUIDE lines into the protected section; `canonical_lines` snapshots accepted canonical LEDGER rules at end of epoch 1. The mock's twist-rediscovery (optimizer.rs, `templates()`) fires in later epochs when (a) no GUIDE mentions twist, (b) no twist rule is present (one-shot), (c) all ledger slots are filled, and (d) the step's L_t fits the bundle whole (L_t ≥ 3). It emits one `Append` of a rediscovered twist rule plus `Replace` canonical→narrow for unprotected, non-recent slots, as a single ranked bundle the gate judges together. Seeds 104/105 (neither arm, epoch 1, step 5): the bundle [twist, repl] is accepted on net D_sel improvement, displacing the epoch-1 canonicals. In the full arm the same failures are handled under GUIDE with KEEP-protected canonicals (2 twist rules, 0 displacement).

Per-arm D_test (points): full 35.00±36.61; no-slow 30.00±36.78; no-meta 35.00±36.61; neither 30.00±36.78. Retention (eligible seeds 102/104/105): full 100%; no-slow 33%; no-meta 100%; neither 33%. Verdict: cost leg 5.00 (indeterminate band), retention legs met (100% ≥ 80%, 33% < 50%) → indeterminate.

## Primary evidence (real model)

Run 2026-09-28 on primo with `GAUNTLET_REQUIRE_REAL=1` against local `qwen3:8b` (toy-scale; direction/mechanism only):

**Null** — the real model accepts no epoch-1 rules in any arm (all D_test 0.00), so retention is undefined (NaN%) and the neither-vs-full cost is +0.00. The scripted finding (full beats neither by 5.0, retention 100% vs 33%) does not replicate because qwen3:8b is too conservative to commit to the ledger rules at all. The scripted run establishes the mechanism; it does not replace primary-model evidence, which remains null at this toy scale.

## Sources

- `crates/phlow-gauntlet/src/tasks/task_104.rs` — arms, `classify`, `retention`, `neither_fails_retention`
- `crates/phlow-gauntlet/src/skillopt/learner.rs` — `SeedState::epoch_end` (`slow_update`, meta), `epoch1_canonical`
- `crates/phlow-gauntlet/src/skillopt/optimizer.rs` — twist-rediscovery + bundled displacement in `templates()`, `ReflectCtx.epoch`
- `~/workspace/gauntlet-design-tasks-101-115.md` — task-104 design (Wave 20); paper grounding arXiv 2605.23904v2 §II.6, §III.3
