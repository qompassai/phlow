# Task 111 — Selection-split overfitting (rust, adversarial)

## Question

The SkillOpt loop tunes on D_sel and reports on D_test. How much of the
D_sel gain is real improvement, and how much is selection luck? And does
the three-split confirmation (accept on D_selA only when D_selB also
strictly improves) contain it?

## Method

Three F-order arms, 5 seeds each, D_test sealed (zero training-time
reads; the seal is a live counter, verified by an unsealed positive
control that counts exactly one seed-setup read):

| arm | D_sel | acceptance |
|---|---|---|
| small | 16 cases | strict improvement on D_sel |
| large | 64 cases | strict improvement on D_sel |
| three-split | 32 → A=16 / B=16 | strict improvement on **both** A and B |

Per-arm statistics: `sel_gain`, `test_gain` (points), and the
overfitting gap `gap = sel_gain − test_gain`.

Adversarial probes:

- **Seal (A1).** The counter is live: sealed arms read 0, the unsealed
  control reads 1 per seed. A hardcoded zero would fail the control.
- **Acceptance/D_test correlation (A2).** Post-hoc (seal-safe) per-edit
  ablation: for every accepted line, remove it from the final body and
  re-score on D_test. Accepted edits that are D_test-neutral or
  D_test-harmful are the overfitting mechanism caught red-handed.
  Seed-level Pearson r between acceptance count and D_test gain is also
  reported.

## Preregistered bars

- Overfitting detected if the small-split mean gap > **3.0** points.
- Contained if the gap ≤ **1.5** points, or the three-split arm shrinks
  the gap by ≥ **50%**.

## Results (scripted double — MOCK, not real-model evidence)

| arm | sel_gain | test_gain | gap | d_test_reads |
|---|---|---|---|---|
| small (16) | 52.50 | 37.50 | **15.00 ± 9.71** | 0 |
| large (64) | 45.31 | 41.00 | 4.31 ± 9.99 | 0 |
| three-split | 48.75 | 32.00 | 16.75 ± 8.12 | 0 |

Three-split shrinkage: **−12%** (the gap got *larger*).

Per-edit ablation: mean marginal D_test per accepted edit is positive
(+2.7 to +3.0 pts), but **65–67% of accepted edits are D_test-neutral
or D_test-harmful** — a few big winners carry the mean while the gate
accepts mostly noise. Seed-level r(accepted, D_test gain): 0.46 (small),
0.89 (large), 0.73 (three-split).

**Verdict: negative.** Overfitting is detected (small gap 15.00 > 3.0)
and the three-split confirmation does not contain it — shrinkage −12%,
well short of the 50% bar. Confirming on a second small split selects
edits good on A∧B, but A∧B jointly still overfit relative to D_test;
the arm ends with *fewer* accepted edits (9.6 vs 12.2) and a *lower*
D_test gain (32.0 vs 37.5).

## Mechanism

`src/skillopt/learner.rs`: `LearnerConfig.sealed_d_test` defers the
initial D_test scoring until `seed_log` (post-hoc baselines only);
`confirm_split` splits D_sel in half and requires strict improvement
on both halves (`confirm_before`/`confirm_after` on every `StepRecord`).

## Limits

Scripted-double evidence only; the design's real-model verdict backend
(qwen3:8b) was skipped per the wave-106–110 precedent. n=5 seeds, so
gap estimates are noisy (±8–10 pts). D_sel content differs across arms
(n_sel=16 vs 32), so arm contrasts carry a split-content confound;
D_test is identical across arms per seed.
