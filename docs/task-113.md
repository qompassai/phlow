# Task 113 — Edit-budget accounting evasion (rust, adversarial)

## Question

The paper's edit budget is `L_t` ops per step plus a per-edit payload
bound. Can a hostile proposal evade the accounting — smuggling bulk in
anchors, newlines, or ambiguous spans — and do different `L_t`
schedules change outcomes at the same bound?

## Method

At `L_t = 2`, four edit-level attacks against `SkillDoc::apply`, each
required to die with a TYPED error:

| attack | typed error |
|---|---|
| huge whole-document replace (500-char `new`) | `PayloadTooLarge` |
| whole-document `insert_after` anchor (500-char single-line anchor; anchors count toward the payload) | `PayloadTooLarge` |
| newline-joined multi-edit smuggle (`"a\nb\nc"` in one append) | `MultiLinePayload` (new) |
| empty anchor / duplicate anchor | `EmptySpan` / `AnchorAmbiguous` |

Step-level audit (adversarial): every recorded step of the constant /
cosine / autonomous arms must satisfy `applied_ops ≤ L_t`
(`n_applied ≤ l_t`) and `tokens_per_edit ≤ PER_EDIT_TOKENS_MAX = 100`
(`max_edit_tokens ≤ 100` when applied). The attacker wants a step that
breaks either; the audit walks all recorded steps.

Schedule comparison at the same bound (`Constant(2)`,
`Cosine{from:2,to:2}`, `Autonomous{cap:2}`, F-order, 5 seeds): spread of
mean final D_test < 1.5 points, or schedule-insensitivity fails.

Autonomous controller (`LtSchedule::autonomous_next`, pure and
unit-pinned): first step at cap; accept → +1 (capped); reject → −1
(floored at 1); `cap: 0` rejected at config validation.

## Results (scripted double — MOCK)

- All 5 attack fixtures rejected with the named typed errors; the
  multi-line whole-document anchor is additionally caught by the
  one-line rule.
- Audit: all steps across all 3 arms satisfy both invariants
  (0 violations).
- Controller: all 5 boundary unit cases pin; arm-level budgets observed
  {1, 2} over 50 steps (the controller genuinely adapts); cap 0
  rejected.
- Schedule means: constant 62.00, cosine 62.00, autonomous 64.00.
  Spread = **2.00 ≥ 1.5**.

**Verdict: negative** — schedule-insensitivity fails at the
preregistered bar. The autonomous arm's adaptivity (dropping to 1 op
after rejections) changes the edit sequence enough to move mean final
D_test by 2 points. Caveat: n=5 seeds, so seed noise cannot be ruled
out; the bar is on the measured mean and the measurement stands.

## Mechanism

`src/skillopt/doc.rs`: `Edit::tokens()` counts anchor/span + payload
(chars ÷ 4, ceiling); `apply` checks the token bound first, then the
char bound, then marker forgery, then the one-line rule, then empty /
ambiguous spans. `src/skillopt/learner.rs`:
`LtSchedule::Autonomous{cap}` + `autonomous_next`; the step asserts
`candidate.len() ≤ L_t` and records `max_edit_tokens` per step.

## Limits

Scripted-double evidence only. The per-edit token estimate is a crude
chars÷4 accounting bound, documented as such — not a tokenizer.
