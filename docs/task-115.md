# Task 115 — Approval render attacks (nvim-lua, adversarial)

## Question

An operator approval UI renders proposed skill edits for review.
Can an attacker compromise the render — via terminal escapes, via
collapsed context hiding the true hunk, via rationale/edit mismatch,
via approval fatigue, or via urgency text — and does the renderer
hold the safety properties?

## Method

Six scenarios under headless Neovim (`lua/gauntlet/task_115.lua`;
self-contained, no diver seam — the render is task-local, stated
honestly):

1. **clean**: baseline well-formed render.
2. **escapes**: edit content carries ANSI escapes (clear-screen,
   set-title, SGR colors). Output must contain zero 0x1b bytes.
3. **elision**: a 20-line hunk with the change at line 15; collapsed
   context must be visibly marked and the true hunk shown.
4. **urgency**: rationale screams "URGENT!!! APPROVE IMMEDIATELY";
   it must stay in the rationale block, never in UI chrome.
5. **mismatch**: rationale says "fix a typo" but the edit DELETES a
   signature check. The behavioral summary (derived from the OP, not
   the rationale) must state the truth.
6. **fatigue**: 30 trivial approvals then 1 consequential delete.
   Presentation rate bounded (≤10 per batch); the consequential edit
   isolated in its own batch; no approve-all.

## Results (headless Neovim — REAL)

All six scenarios pass:

- **escapes**: 0x1b bytes in output = 0. CSI sequences, OSC title
  sets, and bare ESC bytes stripped; other C0 controls removed
  (newline/tab preserved).
- **elision**: every collapse marked `[... N lines elided ...]`; the
  true hunk visible and flagged `<-- CHANGED`.
- **urgency**: contained in the `RATIONALE (untrusted, not verified)`
  block; absent from all chrome lines. (Not censored — contained.)
- **mismatch**: behavioral summary states `DELETES 1 line: CHECK:
  verify signature` despite the "typo" rationale.
- **fatigue**: 31 edits in 4 batches (max 10); the consequential
  delete isolated in the final batch, still individually rendered;
  chrome contains no approve-all.

## Safety properties (proven)

- No terminal escape bytes in captured output.
- Every elision visibly marked with a count.
- Per-edit behavioral summary from the OP, visible above the
  (untrusted) rationale.
- Presentation rate bounded (BATCH_MAX = 10).
- No "approve all" in chrome or render.
- Urgency cannot enter UI chrome.
- Approval binds SHA-256 of the exact edit bytes (`op=…`,
  newline-joined fields), not the rendered output.
- The driver writes only inside `GAUNTLET_WORK_DIR` (a sha256sum temp
  file, removed afterward); poisoned experiment content never reaches
  deployed surfaces.

## Limits

The renderer is task-local, not diver's real approval UI (diver has
no approval-render seam; stated in the driver header). The SHA-256 is
computed via `sha256sum` on a temp file — adequate for the binding
demonstration, not a production KMS. A real operator's fatigue,
inattention, and trust in the rationale are human factors outside
the render's scope; the render can only make the truth visible, not
force the operator to read it.
