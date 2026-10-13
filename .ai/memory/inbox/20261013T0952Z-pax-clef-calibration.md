# Clef-backed calibration: first honest numbers (2026-10-13)

Commission: replay the mock-backend gauntlet corpus through a real
distribution-bearing backend, label from scenario semantics, and
produce phlow's first honest calibration numbers (Matt's word,
2026-10-13, on Pax's recommendation (b)).

## What was built

- `crates/phlow-calibration/src/bin/clef-replay.rs` (new): reads a
  mock corpus's decision points (state + question only — the mock's
  answers are never carried), re-asks each of the estate's only
  `decide_rich` backend, and writes a fresh corpus in recorder
  format. Labels come from an operator-authored labels file,
  checked against the points before any backend call (kind match,
  index range, no unknown or duplicate points). Backend failures
  are recorded per point in `replay-failures.json`, never dropped.
  After writing, it collects the new corpus through the crate's
  own collector and renders the design's report at 10 bins and at
  5 bins into the corpus directory. 10 unit tests (5 validation /
  5 adversarial), all passing.
- Corpus: primo `~/workspace/gauntlet-corpus-clef-20261013/`
  (README.md there carries full provenance + label doctrine).
  Source mock corpus `gauntlet-corpus-20261013` untouched
  (sha256 482f227f…).

## Backend reality (verified firsthand)

- No Clef HTTP service exists in this estate: `RichAnswerBatch::
  from_clef_wire` has no production caller, nothing listens as a
  systemone endpoint, and the CS1 engine (`phlow-agent::SystemOne`)
  has no production embedder or trained head (test doubles and a
  hash projection only). The predictions therefore come from
  system1's rich-answer path — the `RichAnswer` types that landed
  with the Clef backend (`clef.rs`) — served by `OllamaBackend`
  (first-token logprobs), model `qwen2.5-coder:7b` Q4_K_M, stock
  Ollama 0.40.3 on loopback against the rose store, inference on
  the RTX A6000 (daemon dropped the iGPU itself; both MATE GPU
  tenants undisturbed). The daemon was down when found, was
  started for the run, and was stopped afterward. Note: daemon
  startup pruned one unreferenced blob from the rose store
  (OLLAMA_NOPRUNE unset); all 14 manifests remain served.

## Replay accounting (142 points)

- Answered 95: Noul 94/94, Choice 1/19 (task-257, correct), plus
  task-303 (label-pending by doctrine) among the Noul.
- Failures 47, each with the backend's reason: Score 29/29
  (`DistributionUnavailable` — logprobs expose no per-level score
  distribution); Choice 17 (an option absent from top_logprobs —
  the derivation matches whole option strings against first-token
  surfaces, so multi-word options fail closed) + 1 (task-278:
  duplicate options collide after normalization).
- Labels: 100 points / 112 traces authored from (state, question)
  semantics only (doctrine in the corpus README; task-272 is the
  one recorded judgment call). Score is never labeled (no
  predictions exist, and an exact-equality true value would be
  invented precision); task-303 states no decidable proposition.

## The numbers (qwen2.5-coder:7b, first-token logprob confidence)

- Held-out (gate scope): 24 labeled tuples, all Noul. Brier
  (design form) 0.2333; Brier (full distribution) 0.4666; ECE
  0.1067 at 5 bins (0.2195 at 10 bins — over-resolved at n=24;
  5 bins is the supportable resolution). Phase A verdict:
  **NOT CALIBRATED** (thresholds Brier < 0.1, ECE < 0.05). The
  high-confidence bin drives it: held-out [0.8, 1.0) mean
  confidence 0.966 against accuracy 0.750.
- Train scope: 71 tuples (70 Noul + 1 Choice), Brier 0.2116,
  full-distribution 0.4232, ECE 0.0921 at 5 bins.
- Context: a binary random predictor scores Brier 0.25 under the
  design's formula; held-out 0.2333 is barely better than chance
  on these risk-judgment questions. That is a measurement of this
  model + derivation, not of CS1 — CS1 remains unmeasured until it
  has a production embedder and a trained head.

## Neutrality and gates

- Behavior-neutrality for this path: the replay runs no gauntlet
  tasks and no task/funnel code changed; empirically, tasks
  251–318 re-run capture-off on the final tree reproduce all 68
  outcomes of the mock corpus run exactly (41 pass / 27 fail).
- Gates on the exact pushed tree (primo): `cargo fmt --all
  --check` clean; strict clippy (`--workspace --all-targets
  --all-features -D warnings`) 0; workspace suite
  `cargo test --workspace --all-features` **2588 passed / 0
  failed / 29 ignored** (baseline 2578/0/29; the +10 delta is
  this bin's tests); headless-nvim diagnostics (rustana_ls
  attached) 0 on the one new file. Suite ran with the Ollama
  daemon DOWN (the state it was found in) and
  `GAUNTLET_NVIM_BIN=/usr/bin/nvim`; the historical daemon-up
  artifact pair did not appear. (A first suite attempt without
  GAUNTLET_NVIM_BIN failed task_01 on the documented
  compiled-in sandbox nvim path — environment, not code; the
  recorded rerun above is the gate of record.)
- One gate interaction worth recording: gauntlet task_68's
  vocabulary scan (metrics label-registration surface) substring-
  matched this bin's label-file loader names (`load_labels(` /
  `check_labels(` contain the token `labels(`). Classification:
  innocent homonym — the scan guards Prometheus-style metrics
  label APIs; this bin's labels are operator-authored ground
  truth in a JSON file. Fixed by renaming the two private
  functions (`load_label_map` / `check_label_map`); the guard
  itself is untouched and task_68 passes 4/4.
- Environment fix on the way: `pixi install` run in
  `crates/phlow-trainer-mojo` (its build script requires the env;
  sibling worktrees had it, this checkout did not). Gitignored
  directory; no repo content changed by it.

## Remaining gaps (named, not papered)

- Score is unmeasured end to end: no backend in the estate
  produces a per-level score distribution. Options when wanted:
  a Jev/Clef-wire service speaking `from_clef_wire`'s envelope
  (the decoder and tests exist), or a score derivation with the
  same honesty labeling as the logprob derivation.
- Choice coverage is derivation-bound: whole-option first-token
  matching excludes multi-word options by construction. Widening
  it changes the derivation's meaning and needs its own design
  note, not a quiet patch.
- 24 held-out tuples is thin. More labeled points = more wave
  scenarios or repeated waves; the replay is one command once a
  labels file exists.
- The calibration e2e test's final assertion still encodes the
  label-pending era ("INSUFFICIENT DATA"); it is run against the
  mock corpora and was left untouched deliberately.
- Approval (2.0-weight) and feedback (0.5) sources unchanged:
  no producer persists approval decisions; no feedback.db exists.
