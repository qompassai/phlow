# task-106: optimizer-loop composed dynamics (rust, V)

**Kind:** rust · **Status:** pass (null) · **Wave:** 106–110 · **Commits:** pending (wave 106-110)

## ELI5

The paper reports three odd-but-specific facts about its optimizer loop: it accepts very few edits (median 2.5), it throws away the vast majority of proposals, and the final skill is small (300–2000 tokens) — yet one single accepted edit usually explains most of the gain. This task puts all the pieces together (the composed F-order + F-bind loop) and asks: do the *dynamics* look like the paper's, or do the components interact in a way the paper never described?

## What this task attempts

- **Goal:** measure the composed loop's dynamics against the paper's three numbers (median accepted edits, proposal acceptance rate, artifact size) plus the single-edit dominance claim, on mixed F-order + F-bind, 3 epochs, 5 seeds.
- **Mechanism:** `src/tasks/task_106.rs` runs the composed arm and measures: median accepted edits per seed (from `SeedLog.all_accepted`, the run-wide accepted-edit ledger); proposal acceptance rate (applied ÷ proposed); exact artifact bytes plus a clearly labeled whitespace-token proxy; per-seed D_test gains; the top-one accepted edit's share of D_test gain via leave-one-out re-scoring; the top-one D_sel share from incremental gate deltas.
- **Success criterion (pre-registered):** *replicates* = median ≤4 accepted edits AND acceptance <25% AND artifact in the 300–2000 token window AND every seed gains AND no seed regresses vs s_0; *negative* = any D_test regression or acceptance >50%; otherwise *null*.
- **Non-goals:** real-model evidence (scripted double only); exact tokenization (reported honestly as bytes + labeled proxy).

## What happened

**Null** — the composed loop is much more edit-happy than the paper's, but not pathological:

- **V1:** the dynamics classify. Median accepted edits **14.0** (paper 2.5); acceptance rate **43.0%** (paper: vast majority filtered, i.e. <25%); artifact **1231 bytes / ~161 whitespace-tokens** (paper 300–2000 — below the window on the proxy); D_test gains **[83.8, 73.8, 65.0, 70.0, 81.2]** points, all >0, none below s_0. Top-one edit share of D_test gain: **67%** mean (paper: "most of it" — this leg *replicates*); top-one D_sel share **70%**.
- **V2:** the accepted-edit ledger is legible (every accepted edit recorded with seed, step, direction, line).
- **A1 (adversarial to the loop):** no seed regresses vs s_0 across the full composed run — the gate holds even at 43% acceptance.
- **A2:** an untrusted document (wrong provenance) cannot enter the loop — the learner refuses non-experiment documents.

The honest reading: the single-edit dominance finding *replicates* (one edit explains ~2/3 of the gain), but the loop's *permissiveness* does not — the mock accepts 14 edits at 43% where the paper accepts 2.5 with most filtered. The components don't interact pathologically (no regression, gains positive), but the mock's gate is quantitatively looser than the paper's. This is a double limitation, not a paper refutation.

## Full technical depth

`SeedLog.all_accepted` is a run-wide ledger (unlike `epoch_accepted`, never cleared) recording every accepted edit line. Top-one D_test share is measured by leave-one-out: re-score the final skill with each accepted edit's line removed, take the max drop. Top-one D_sel share uses the incremental gate deltas recorded per step. The whitespace-token proxy is `bytes ÷ mean bytes-per-whitespace-token` — labeled as a proxy in every evidence line; the exact byte count (1231) is the primary measure. The 43% acceptance rate reflects the Strict gate accepting any D_sel-positive bundle; the paper's real loop with a frontier model proposes fewer, larger edits.

## Primary evidence (scripted double)

All numbers above are from the clearly labeled scripted double (`ScriptedOptimizer`; NOT a real model). No real-model evidence is presented for this task's verdict.

## Sources

- arXiv 2605.23904v2 §II (optimizer loop), §III (dynamics: median 2.5 accepted edits, majority filtered, 300–2000 tokens, single-edit dominance)
