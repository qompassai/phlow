# Calibration crate built — 2026-10-13 (Pax)

Matt ordered the build ("please ensure phlow calibration crate is
built"), superseding the 2026-10-11/12 activation STOP. The crate
exists now: `crates/phlow-calibration`, per the design of record
(`calibration-harness-design-2026-10-04.md`, Matt approved
harness-design-first 2026-10-04). Modules: tuple, metrics,
collector, dataset, report. Deps: phlow-gauntlet (trace contract +
held-out split), phlow-system1 (protocol types, real wire boundary),
phlow-self-improve (FeedbackStore), serde_json. No chrono (the
workspace has none) and no serde derive — manual JSON, estate style.

## Corpus provenance (real runs, nothing fabricated)

- Two corpora on primo, both produced by the gauntlet binary with
  the DecisionTrace capture from `9d764ca`, system1 wave
  task-251..318 at the 870ed53 lineage:
  `~/workspace/gauntlet-corpus-20261011/` (2026-10-11) and
  `~/workspace/gauntlet-corpus-20261013/` (fresh, this session).
- Each: 142 traces (94 Noul / 19 Choice / 29 Score), 68 task
  outcomes (41 pass / 27 fail), 41 held-out traces. The fresh run
  re-proved capture behavior-neutrality at this HEAD: capture-on
  vs capture-off outcome diff = zero across all 68 tasks.
- The gauntlet binary needs `--nvim-bin /usr/bin/nvim` and
  `--diver-lua $HOME/.config/nvim/lua`; `GAUNTLET_NVIM_BIN` alone
  does not satisfy it.

## What remains honestly empty, and why

- **Labels.** `ground_truth` is null on every trace in both
  corpora — by the capture's deliberate design (label-pending):
  the scenarios' expected values equal the scripted predictions,
  so harvesting them would manufacture perfect calibration.
  Metrics therefore run over labeled tuples only, scores are
  `None` (never 0.0) with none, and the report's Phase A verdict
  on the real corpora is INSUFFICIENT DATA. Calibration numbers
  arrive when real labels do (adjudicated scenario verdicts or
  Matt's own decisions); nothing is estimated in the meantime.
- **Approval source.** Approval *decision* history is persisted
  nowhere in phlow (phlow-approval events are in-memory;
  `~/.local/share/phlow/consumed-approvals.jsonl` is
  phlow-experiment's promotion replay store of `{id, expires_ms}`
  lines and does not exist on primo). The collector reads that
  store fail-closed when present and reports Absent otherwise;
  `TupleSource::Approval` (weight 2.0) is fully implemented for
  when a producer exists.
- **Feedback source.** No `feedback.db` exists on primo. Even
  populated, its rows are session-level (rating + outcome, no
  decision point), so the collector surfaces them as signals and
  yields no tuples from them. Collecting never creates the file.

## Interpretation choices (design silent; recorded in crate docs)

- `decided_at` stored as `u64` epoch milliseconds (capture
  precedent, lossless, no new dependency).
- `predicted_dist: Option<Vec<f64>>` — scalar backends genuinely
  record null for Choice/Score; inventing a distribution would
  fabricate the signal being measured. Noul's Bernoulli
  completion `[1-p, p]` is the protocol's own construction.
- Both Brier forms reported, never conflated: the design's
  confidence form, and the full-distribution multiclass form
  (idea statement) over distribution-carrying tuples only.
- Score correctness = exact value equality (no tolerance band
  invented). Score tuples never enter full-distribution Brier.
- Dataset export: labeled tuples only, held-out gauntlet tuples
  excluded (validation reserve), weights 1.0 / 2.0 / 0.5.

## Gates at build time

- Crate tests: 53 passed / 0 failed / 1 ignored (the ignored e2e
  runs green against both corpora via `PHLOW_CALIBRATION_CORPUS`).
- Strict clippy (`-D warnings`, all targets, all features): 0.
  `cargo fmt`: clean. Headless-nvim diagnostics: 0 on all 12 new
  files.
- Full workspace suite: 2578 passed / 0 failed / 29 ignored
  (baseline 2525 / 0 / 28 + this crate's 53 / 1).
