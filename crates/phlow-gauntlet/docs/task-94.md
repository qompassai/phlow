# task-94: improvement rollback

**Kind:** rust · **Status:** fail (open) · **Wave:** 91–95 · **Commits:** pending (wave 91-95)

## ELI5

Imagine a time machine with a big red "undo" button — except the button is a sticker. The machine carefully writes down *where* you'd want to go back to (`rollback_target`: "the exact revision to restore on rollback"), files the note in triplicate (proposal → promotion record), and even has a labeled arrival state (`RolledBack`). But there is no engine: nothing reads the note, nothing moves the machine, nothing checks that you arrived where the note said. The only "undo" code in the building is a sign that says "undo is disabled — do it yourself" (`SkillStore::revert_skill` always fails with `GitMutationDisabled`: "Automatic Git mutation is disabled; revert manually").

## What this task attempts

- **Goal:** verify a gated rollback path — byte-identical restore verified by hash, revert as a human-approved action, unsafe-revert refusal with conflicts named, revert-of-revert as a typed no-op — or document the absence with source evidence.
- **Mechanism:** `src/tasks/task_94.rs` runs four bounded source recons: `no_revert_mechanism` (zero hits for `git_checkout`/`restore_tree`/`checkout_tree`/`three_way`/`git2`; every bare `revert` hit is inside phlow-self-improve's disabled surface; the denial path still has its `GitMutationDisabled` shape); `rollback_target_is_data_only` (the field occurs in promotion.rs but no `Command::new`/`git2`/`worktree`/`checkout` machinery could act on it); `revert_approval_unverifiable` (zero hits for `revert_with_approval`/`approved_revert`/`rollback_with_approval` — the design's "revert requires its own approval" is vacuously true); `no_unsafe_revert_guard` (zero hits for `revert_conflict`/`unsafe_revert`/`revert_refused`/`three_way_merge`).
- **Success criterion:** rollback verified, or the absence documented with source evidence.
- **Non-goals:** building a rollback path on gauntlet authority (product decision — banked, never implemented here).

## What happened

Honest FAIL at `where = "seam"`, first attempt — the seam is ABSENT as designed:

- **V1:** no revert mechanism — the only `revert` code paths are explicit denials. `SkillStore::revert_skill` always fails with `GitMutationDisabled`; the message is "Automatic Git mutation is disabled; revert manually."
- **V2:** `rollback_target` is data only — a String field on `ProposalParams`/`ImprovementProposal`, exposed via an accessor, copied into `PromotionRecord`. Nothing reads it to perform a revert. (The one `std::fs::rename` in promotion.rs is atomic persistence of the consumed-approvals ledger, not a tree restore — verified by inspection.)
- **A1:** approval-gating unverifiable — with no revert path, "revert requires its own human approval" holds vacuously. Fail-closed by absence, not by enforcement.
- **A2:** no unsafe-revert guard — no conflict detection, no three-way merge, no refusal path. Subsequent changes landing after an improvement have no revert guard to trip, because there is no revert to guard.

## Full technical depth

The recon uses bounded exact-token scans over `crates/*/src/**/*.rs`, excluding the gauntlet crate per the harness-probe principle. The `no_revert_mechanism` case does two scans: the mechanism-vocabulary scan (must be zero) and the bare-token `revert` scan (must be non-zero but confined to phlow-self-improve's disabled surface) — the second scan guards against the first being vacuous. The `rollback_target_is_data_only` case counts field occurrences (non-zero, so the field exists) and then asserts no mechanism in the module could act on it.

The design's four properties — byte-identical restore verified by hash, revert as a human-approved action, unsafe-revert refusal with conflicts named, revert-of-revert as a typed no-op — have no implementation to probe. This is the honest shape of the finding: not "rollback is broken" but "rollback is a filed note with no engine."

Product decision banked for Matt: whether phlow-experiment should gain a real gated rollback path. Not implemented on gauntlet authority.

## Sources

- `crates/phlow-experiment/src/promotion.rs` — `rollback_target` (data-only field), `Lifecycle::RolledBack` (named state, no mechanism)
- `crates/phlow-self-improve/src/skill_store.rs`, `crates/phlow-self-improve/src/error.rs` — `revert_skill` → `GitMutationDisabled` (the explicit denial)
- `~/workspace/gauntlet-design-tasks-71-100.md` — task-94 design (Wave 91–95)
