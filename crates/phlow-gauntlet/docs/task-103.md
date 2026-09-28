# task-103: rejected-buffer ablation

**Kind:** rust · **Status:** pass (indeterminate) · **Wave:** 101–105 · **Commits:** pending (wave 101-105)

## ELI5

When the recipe learner rejects a bad change, it writes the idea on a "don't try this again" sticky note and shows the notes to itself next round. The paper (arXiv 2605.23904v2 §II.5) calls this "the textual equivalent of negative-example memory" and says removing it costs 2.4–4.6 points. This task splits the sticky-note system in two — *writing* notes vs *reading* them — with three arms: full (write+read), write-only (writes notes, never reads them), off (no notes). If reading is what matters, write-only should score like off.

## What this task attempts

- **Goal:** isolate the rejected-edit buffer's two sub-mechanisms (recording vs consulting) on F-bind: does the buffer's value come from the memory or from the act of recording?
- **Mechanism:** `src/tasks/task_103.rs` runs three arms — full, write-only, off — 5 seeds each, 2 epochs, scripted F-bind. The scripted optimizer re-samples from a fixed template distribution, so without the buffer it re-proposes dead directions; in full mode it sees `ctx.rejected` and suppresses them. Write-only records rejections into the buffer but never shows them to the optimizer — recording alone must not change behavior. A mode-independent (direction, step) rejected-log feeds the re-proposal metric in every arm.
- **Success criterion (pre-registered):** *replicates* = off costs ≥1.5 D_test points vs full AND the re-proposal rate (a rejected direction re-proposed within 3 steps) is ≥2× higher with the buffer off, with write-only between the two; *null* = re-proposal rates within ±20% across arms. Buffer hit rate reported: fraction of reflections where a buffered rejection actually suppressed a candidate.
- **Non-goals:** poisoned evidence (that's task-110); changing buffer semantics on gauntlet authority.

## What happened

**Indeterminate** — the *mechanism* replicates perfectly, but the *score* cannot move because F-bind saturates:

- **V1:** all three arms complete and classify. Every arm reaches **100.00±0.00** D_test — F-bind is fully solved by the mock with or without the buffer, so the ≥1.5-point cost leg cannot replicate here. The domain is too easy, not the mechanism too weak.
- **V2:** the mechanism leg replicates crisply: re-proposal rate is **0.000** with the buffer full vs **0.480** with it off (ratio ∞ — full suppresses perfectly). The buffer's hit rate is **84.0%** of reflections — when the mock sees the notes, it uses them.
- **A1:** write-only sits exactly at off (re-proposal 0.480, D_test 100.00) — recording alone changes nothing, proving the *consulting*, not the *recording*, carries the effect. Inclusive equality with off counts as "between" per the preregistration.
- **A2:** the mode-independent rejected-log confirms the re-proposal metric is measured identically in all arms — the 0.000 vs 0.480 gap is the buffer's doing, not a measurement artifact.

The honest reading: this is a domain-ceiling indeterminate, not a mechanism failure. The negative-example memory works exactly as the paper describes (84% hit rate, zero re-proposals when consulted), but F-bind gives it nothing to save — every arm converges to perfect. A harder family would be needed to price the score cost; the mechanism evidence is banked as-is.

## Full technical depth

`RejectedBuffer::readable(mode)` returns directions only in `Full` mode; `record` writes in Full and WriteOnly, no-ops in Off. The mock's `templates()` filters any template whose direction appears in `ctx.rejected` (counted via `last_suppressed`). The learner's `rejected_log: Vec<(String, usize)>` records (direction, step) for every rejected edit in every mode; `reproposal_within3` counts proposed directions present in that log within the last 3 steps. Hit rate = steps with `buffer_suppressed > 0` / total reflection steps.

Per-arm D_test (points): full 100.00±0.00; write-only 100.00±0.00; off 100.00±0.00. Re-proposal: 0.000 / 0.480 / 0.480. Verdict: not replicates (cost 0.0 < 1.5), not null (rates differ by more than ±20%) → indeterminate.

## Primary evidence (real model)

Run 2026-09-28 on primo with `GAUNTLET_REQUIRE_REAL=1` against local `qwen3:8b` (toy-scale; direction/mechanism only):

**Indeterminate** — the buffer mechanism is much weaker with the real model. Re-proposal rates: full **0.496**, write-only **0.700**, off **0.714** (ratio 1.44×, not ∞). The buffer hit rate is **0.0%** (vs 84.0% scripted) — qwen3:8b rarely re-proposes the exact same rejected edit, so the full buffer has little to suppress. Write-only is still between full and off (0.700 between 0.496 and 0.714), preserving the ordering, but the absolute suppression the scripted double showed does not replicate. The mechanism needs a model that actually repeats its mistakes.

## Sources

- `crates/phlow-gauntlet/src/tasks/task_103.rs` — arms, `classify`, `arm_data` (hit-rate denominator = reflections)
- `crates/phlow-gauntlet/src/skillopt/learner.rs` — `RejectedBuffer`, mode-independent `rejected_log`
- `crates/phlow-gauntlet/src/skillopt/optimizer.rs` — `templates()` suppression via `ctx.rejected`, `last_suppressed`
- `~/workspace/gauntlet-design-tasks-101-115.md` — task-103 design (Wave 20); paper grounding arXiv 2605.23904v2 §II.5, §III.3
