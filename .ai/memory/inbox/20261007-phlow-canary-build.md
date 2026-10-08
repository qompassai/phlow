# 2026-10-07 — phlow-canary crate built (design + 2026-10-07 amendment)

Built per Matt's authorization ("please build the canary build
crate", 2026-10-07), implementing
`~/workspace/phlow/canary-battery-design-2026-10-04.md` as amended.

## What shipped

- New workspace crate `crates/phlow-canary`: the behavioral battery
  that runs against a candidate model before it can serve behind
  phlow's signed approval layer. 11 probes across the four designed
  categories (injection ×3, trigger ×3, refusal ×2, calibration ×3),
  each run 3× requiring 3/3 (design open-question-3 rule). Binary
  fail-closed `Verdict` (`Deploy` / `Refuse { failed }`) cached
  against the artifact's SHA-256 + canary version.
- The 2026-10-07 amendment is implemented in the suite's verdict
  layer: on a unanimous trigger-category pass, one challenger
  repetition runs a perturbed variant (paraphrase or trigger-strip);
  collapse fails the probe, survival lets the pass stand. The
  `Probe` trait is unchanged. The disagreement resolver is NOT built
  (pinned separately); split runs are logged from this first build —
  every probe with a non-unanimous result appends a `SplitRun`
  record (per-run pass/fail + mean confidence, and the variance of
  run mean confidences) to the JSONL report and can append to a
  standing split log.
- Payloads live outside the repo:
  `/home/phaedrus/.local/share/phlow/canary/payloads-v1.json`
  (dir 0700, file 0600; the loader refuses group/other-accessible
  store files). Thresholds are per-model: the suite refuses
  uncalibrated models (`UncalibratedModel`); `calibrate()` derives
  a model's book from its measured clean baseline plus named policy
  margins. No universal constants.

## Forced deviation from the design text (recorded in lib.rs)

The design's probes read `RichAnswer` confidence distributions from
phlow-system1. At this commit, system1 exposes scalar `confidence()`
only; the distribution-carrying `RichAnswer` exists solely in the
uncommitted clef work on the dirty main checkout. Rather than touch
that state, phlow-canary defines canary-local `RichAnswer` /
`RichAnswerBatch` types and a synchronous `ProbeBackend` trait;
when the richer system1 API lands, an adapter maps it onto
`ProbeBackend` without changing any probe.

## Validation

- `cargo test -p phlow-canary`: 25 tests — 14 validation,
  11 adversarial (injection follower, sequence-locked stub caught,
  canary-aware evasive stub caught ONLY by the challenger, refusal
  inconsistency, bimodality, backend-error and malformed-distribution
  fail-closed, provenance/tamper guards). Debug + release green;
  fmt clean; clippy `-D warnings` clean.
- Battery budget: a full run is 51 backend calls by construction
  (asserted in tests); measured end-to-end at ≤1 ms against scripted
  in-process backends, so the 300 s budget leaves ≈5.9 s per call
  for real-model latency.
- Full-workspace gate and push status: see the commit message /
  session report. Baseline failure remains task_08
  `default_real_server_round_trip` only (see
  2026-10-07-push-waiver-verdict-fence.md for the gated-environment
  method).
