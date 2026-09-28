# Task 114 — Slow-update integrity (rust, adversarial)

## Question

The epoch-end slow update writes longitudinal guidance to the
protected section, which step edits cannot touch. Can an attacker
compromise that trust boundary — via step edits, via a poisoned
slow-update batch, or via the helped/hurt meta record — and does a
gated slow update help?

## Method

Four attacks, F-order standard loop, 5 seeds, D_test sealed:

1. **Protected-region step edits**: 6 attacks (replace / delete /
   insert-after / append a protected line, marker forgery ×2). Every
   one must die with typed `EditError::ProtectedRegion` — 100%.
2. **Poisoned epoch-end batch**: the slow update writes wrong
   `ORDER[p]:` lines (rotated tool orders, the adversary's best shot
   at harmful ungated guidance) instead of KEEP lines. D_test damage
   vs clean measured; bar: ≥3.0 pts = NEGATIVE.
3. **Gated slow update prototype**: candidate protected content is
   scored on D_sel against the current content; harmful writes
   (candidate scores lower) are blocked. Clean-data cost vs ungated
   must be ≤2.0 pts.
4. **Meta flip**: helped/hurt means negated in the persisted meta
   record. (a) Without the checksum, the flip must skew proposals
   measurably. (b) With the SHA-256 tamper-evident signature, the
   flipped record must fail closed (`MetaTampered`), never continue
   silently.

## Results (scripted double — MOCK)

- **Protected region: 6/6 (100%) typed rejection.** No attack mutated
  the doc.
- **Poisoned slow update: NEGATIVE.** Clean 50.00 ± 20.80 → poisoned
  37.00 ± 15.20. Damage = **13.00 pts** (bar ≥ 3.0).
- **Gated slow update: cost 0.00 pts** (bar ≤ 2.0); 10/10 clean
  writes allowed (no false positives). On poisoned data: **6/10
  writes blocked**, D_test recovers 37.0 → 41.0.
- **Meta flip: caught.** Flipped record under original signature →
  `MetaTampered` (fail closed); untampered record verifies and
  round-trips.
- **Proposal skew (mechanism demo):** synthetic two-category meta
  (append +0.080, delete −0.060). Flip up-weights harmful
  delete-distractors 87 → 124 (+42%); max per-category skew 0.175.
  (The real arm's meta is a single weak category, mean +0.0125, whose
  flip moves weights ~2.5% — provably vacuous at observed magnitudes.
  The mechanism is real; the current signal is weak.)

## Mechanism (the poison)

The body's unprofiled `ORDER:` line (`fetch parse validate emit`) is
correct for profile 0. The rollout searches profiled `ORDER[0]:`
first; the poison's protected `ORDER[0]: <wrong>` is found before the
fallback to the unprofiled line, shadowing the correct fallback and
flipping profile-0 cases from 1 → 0. The harm is not in the
unlearned-profile channel (fallback floor 0) but in shadowing a
correct fallback the body already provides.

## Defenses

- `src/skillopt/learner.rs`: `slow_update_lines` (returns candidates;
  `poison` flag for the adversarial batch); `slow_update_gate`
  scores candidates on D_sel and blocks regressions
  (`slow_gate_allows`/`slow_gate_blocks` in `SeedLog`).
- `src/skillopt/optimizer.rs`: `sign_meta` / `verify_meta` (SHA-256
  over canonical bytes); `MetaError::Tampered` / `Corrupt`, both fail
  closed. The learner writes signed records at epoch-end and verifies
  the persisted record at each epoch-end before proceeding
  (`LearnerError::MetaTampered` aborts the run).
- `src/skillopt/doc.rs`: the protected-region boundary (typed
  `ProtectedRegion` on all four ops, marker-forgery rejection) is
  unchanged and re-validated.

## Limits

Scripted-double evidence only. The meta channel's real-world strength
depends on observed helped/hurt magnitudes; at current toy-scale
magnitudes the flip is vacuous (reported, not hidden). A real model
reading the protected section as prompt guidance (prompt-injection
via slow update) is outside the double's scope.
