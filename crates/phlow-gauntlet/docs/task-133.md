# task-133: scope revocation mid-cycle

**Kind:** rust · **Status:** pass · **Wave:** 23 · **Commits:** <worktree commit on gate>

## ELI5

Probing a target that just left the program's scope is a program
violation — the equivalent of testing a system you were never allowed
to touch. This task checks the *policy* (not the mechanism): when a
new scope snapshot drops a target, the agent must drop it from the
probe queue before launch, and cancel its run if it's already probing,
recording why. If the target comes back into scope later, it becomes
fair game again — there's no permanent blacklist. A revocation naming
a target nobody knows is a harmless no-op, not a crash.

## What this task attempts

- **Goal:** verify revocation ⇒ no probe: queued revoked targets are
  dropped and never launched; running revoked targets' runs are
  cancelled with reason `ScopeRevoked`.
- **Mechanism:** driver-local `apply_scope_transition` /
  `revoke_target` / `try_enqueue` in
  `crates/phlow-gauntlet/src/tasks/task_133.rs`, driving
  `bounty::diff::diff_scope`, `bounty::store::TargetQueue`, and
  `bounty::store::RunLedger` against scripted v1/v2/v3 snapshots
  (v2 drops b; v3 re-adds b).
- **Success criterion:** zero ledger runs for revoked b after v2;
  c's run Running → Cancelled with reason `ScopeRevoked`; unknown
  revocation → typed `UnknownTarget` no-op; b queueable again under
  v3, refused (typed `NotInScope`) under v2.
- **Non-goals:** computing the diff (task 132); the mechanics of
  killing a probe process (task 137).

## What happened

Pass on the second attempt. The first compile failed on a borrow error
in `task_133.rs`: `match c_run` moved the value that was used again
later. The four cases (all passing after the borrow fix):

- `revoked_queued_target_dropped`: v1→v2 revocation outcome for b was
  `Applied{dropped_from_queue: true, runs_cancelled: 0}`; b gone from
  the queue; ledger shows 0 runs for b (never launched); a's run
  undisturbed.
- `revoked_running_run_cancelled`: revoking c (whose run is Running)
  gave `Applied{dropped_from_queue: false, runs_cancelled: 1}`; the
  run is now `Cancelled` with `cancel_reason == "ScopeRevoked"`.
- `unknown_revocation_noop`: revoking `ghost` returned
  `RevocationOutcome::UnknownTarget`; queue length and ledger run
  count unchanged.
- `readded_target_queueable`: `try_enqueue(b)` under v2 →
  `Err(QueueRefusal::NotInScope)`; under v3 → `Ok(())`, and the queue
  holds b. Latest snapshot wins; no permanent ban.

## The fix — what changed and why

One fix iteration: changed `match c_run` to `match c_run.as_ref()`
in `src/tasks/task_133.rs` so the run record is borrowed rather than
moved, then ran `cargo fmt` on the new files. No test logic changed —
the four cases passed on the next run.

## Full technical depth

`apply_scope_transition` diffs old→new and calls `revoke_target` per
removed id. `revoke_target` does two things: `TargetQueue::cancel`
(removes queued entries by id) and, for every non-terminal ledger run
(`Queued | Running`) on that target, `RunLedger::set_state(...,
Cancelled, Some("ScopeRevoked"))`. The two halves are independent —
a queued target has no runs yet, a running target is no longer in the
queue — and the `RevocationOutcome` reports both counts, so the
driver can assert the exact shape per case.

The queue mouth is the structural enforcement: `try_enqueue`
refuses any target not present in the *latest* snapshot with the
typed `QueueRefusal::NotInScope`. This is what makes the refusal
structural rather than advisory — there is no code path that pushes
a target without passing the latest-snapshot check. Re-adding under
v3 works because the check is against the latest snapshot, not a
tombstone list; revocations leave no permanent ban.

Terminal runs are never touched: the non-terminal filter means a
`Finished`/`Failed`/`Cancelled` run for a revoked target keeps its
history (resume logic in task 138 depends on this).

## Sources

- Primary: the policy is ours — asserted in code. The program-
  violation framing (out-of-scope testing is a violation) is the
  standard stated in the task design; Bugcrowd/HackerOne program
  pages define per-program scope, and testing outside it is the
  violation this policy structurally prevents.
- Scaffold: `src/bounty/diff.rs`, `src/bounty/store.rs`
  (`TargetQueue::cancel`, `RunLedger::set_state`).
