# phlow-trainlab: executable-reward training harness (new crate)

Date: 2026-10-08 (UTC) · Agent: pax (trainlab worktree session)
Branch: `pax/trainlab-20261007` off `8e2a82d` · LOCAL COMMIT ONLY (no push).

## What

New workspace member `crates/phlow-trainlab` (+ root `Cargo.toml`
member entry, `Cargo.lock` entries). It ports the measurement half of
the GLM-5.3-Flash from-scratch course (freeCodeCamp / Vuk Rosić;
repo `vukrosic/glm-5.3-flash-from-scratch`) into phlow, per Matt's
2026-10-07 commission after reviewing the video:

- `task`: the course's 8 synthetic Python families and frozen split
  generation (dev 1701 / rl 3907 / final 2909 / confirm 8123 seeds,
  unseen entry-point names per split — the seed *scheme* mirrors the
  Python; the PRNG differs, so names are phlow-reproducible, not
  byte-identical to the Python run).
- `executor`: bounded subprocess reward execution (temp dir,
  deadline, output caps) — requires explicit operator acknowledgment
  (`acknowledge_code_execution` / CLI `--execute-rewards`); phlow is
  not a sandbox and the crate says so.
- `reward`: binary 1.0 / 0.0 / −0.1-invalid and case-fraction modes.
- `group`: RLOO leave-one-out advantages; unbiased pass@k (Chen et al.).
- `sampler`: `Sampler` trait; `ScriptedSampler`; loopback-only-by-
  default `OllamaSampler` with a conservative completion extractor
  (instruct models answer in prose/fences; the extractor only ever
  *selects* lines — same normalization-ahead-of-strict-consumer
  pattern as the reviewer's `strip_verdict_code_block`).
- `gate`: persistent selection/confirmation ledger — selection is
  recorded from dev, confirmation opens exactly once against it,
  tampered ledgers (open count > 1) are rejected.
- `receipt`: `phlow.trainlab.receipt/v1` run receipts with config
  SHA-256; writes refuse to overwrite existing receipts.
- `runner` + `phlow-trainlab` bin: `tasks | eval | run |
  record-selection | open-confirm`.

## Explicit non-goal (named gap)

No weight updates. Specialists are GGUF models served by Ollama —
no gradient path exists in the workspace. Group records carry
`updated: bool` meaning "this group had non-zero reward spread, i.e.
a trainer backend would have stepped". Closing the loop needs a
trainer backend emitting weights + receipts; phlow-experiment's
signed promotion gate is the natural approval point for such a
candidate. The crate does not modify phlow-experiment (its
no-execution invariant is preserved); integration is future work.

## Validation (on primo, pinned nightly-2026-09-25)

- `cargo test -p phlow-trainlab` — 43 passed, 0 failed (validation +
  adversarial: split disjointness, gate open-once/lock/tamper,
  unacknowledged execution refusal, deadline kill of an infinite
  loop, receipt overwrite refusal, pass@k known values, extraction
  of chatty/fenced responses).
- `cargo clippy -p phlow-trainlab --all-targets -- -D warnings` — clean.
- `cargo fmt -p phlow-trainlab` applied.
- CLI smokes in /tmp/trainlab-smoke: stub eval (pass@1 1.0), stub run
  → receipt written; rerun to same path refused (exit 2); confirm
  eval while sealed refused (exit 2); record-selection +
  open-confirm → confirm eval runs.
- Live Ollama eval: `qwen2.5-coder:7b`, dev split, increment+double,
  per-family 1, 4 samples @ 0.35 → 4/4 passing on both tasks
  (mean pass@1 1.0, ~25 s). Before the extractor existed, the same
  eval scored 0 because the model wraps answers in prose/fences —
  that failure is why `extract_completion` exists.

## Notes

- The main checkout's dirty files (phlow-agent/system1/inference)
  and the CS1-rename state were not touched; all work happened in
  worktree `~/workspace/repos/phlow-trainlab-wt`.
- Review of the video: `~/workspace/phlow-edit/training-tooling-review.md`
  (sandbox-side). Transcript fetch hit YouTube HTTP 429 and was
  treated as a hard stop; the review rests on video metadata/chapters
  plus the companion repo as primary source.
