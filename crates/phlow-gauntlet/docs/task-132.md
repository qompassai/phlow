# task-132: scope diffing — added/removed targets

**Kind:** rust · **Status:** pass · **Wave:** 23 · **Commits:** <worktree commit on gate>

## ELI5

Each poll brings a fresh list of in-scope targets. To know what to do
next, the agent has to compare yesterday's list with today's: what's
new (queue it), what's gone (stop probing it), and what changed in
place. This task checks that comparison. The important subtlety: if a
target keeps its ID but its details change, that's *one* changed
entry — not a removal plus an addition, which would lose the thread.
And the comparison has to be fast: with ten thousand targets it must
finish in under a second.

## What this task attempts

- **Goal:** verify `diff_scope(old, new)` returns exact added /
  removed sets and `changed` as (old, new) pairs.
- **Mechanism:** `bounty::diff::diff_scope` (id-indexed `HashMap`s,
  O(n)), driven directly in
  `crates/phlow-gauntlet/src/tasks/task_132.rs` against synthetic
  snapshots.
- **Success criterion:** v1={a,b,c} → v2={b,c,d,c′} gives added={d},
  removed={a}, changed=[(c,c′)] (one pair); empty new snapshot →
  removed={a,b,c}, added=[]; 10k-target diff < 1s wall with exact
  counts.
- **Non-goals:** acquiring the snapshots (task 131); acting on the
  diff (task 133).

## What happened

Pass on the first attempt. The four cases:

- `added_removed`: exact set equality — added `["d"]`, removed
  `["a"]`.
- `changed_is_pair`: exactly one changed pair, `c: c.example.com →
  c2.example.com`; id `c` appears in neither added nor removed.
- `empty_new_snapshot`: added 0, removed `["a","b","c"]`, changed 0 —
  no panic on the empty snapshot.
- `ten_k_diff_bounded`: 10,000-target snapshots (50 removed, 50
  added, 100 changed, 9,850 untouched) diffed in well under the 1s
  budget with exact counts.

## The fix — what changed and why

No fix iterations: the task passed on the first attempt, so there is
no fix entry.

## Full technical depth

`diff_scope` builds two id-indexed maps (`HashMap<&str, &Target>`)
and two id sets, then walks set differences for added/removed and
the set intersection for changed (a `Target` inequality under a
stable id, since `Target` derives `PartialEq` over id + kind +
value). The walk is O(n) in the snapshot size — no nested loops —
which is what the 10k case pins: 50/50/100/9850 split asserted
exactly, wall time asserted under `DIFF_WALL_SECS_MAX = 1.0s`.

The changed≠add+remove invariant matters downstream: task 133's
revocation logic keys off `removed`, so a value change under a stable
id must not look like a revocation. The added/removed lists are
sorted by id before return, so the output is deterministic (no
hash-map iteration order leaks into reports).

## Sources

- Primary: the contract is ours — asserted in code
  (`src/bounty/diff.rs`, 32 lines). The design doc explicitly notes
  set-reconciliation literature is overkill here.
- Secondary: none.
