# task-135: approval expiry

**Kind:** rust · **Status:** pass · **Wave:** 23 · **Commits:** <worktree commit on gate>

## ELI5

A permission slip with an expiration date stops working at midnight —
not "whenever someone gets around to checking". This task checks that
an approval authorizes nothing new after its expiry second, with the
boundary exact to the second. The subtle part: a probe that started
*before* expiry is allowed to finish (you don't yank the ladder out
from under it), but starting a *new* probe after expiry is refused.
And a slip dated in the future — "valid starting tomorrow", which
smells like backdating — is rejected when it's issued and never even
enters the system.

## What this task attempts

- **Goal:** verify the time dimension of a valid approval marker:
  expiry is fail-closed at the exact second; in-flight runs may
  finish; new launches after expiry are refused.
- **Mechanism:** driver-local `gate_launch` (same contract as task
  134's; the launch gate is duplicated per-driver since the scaffold
  owns no launch gate) and `issue_approval` with typed
  `IssuanceError`, in
  `crates/phlow-gauntlet/src/tasks/task_135.rs`, on the
  `bounty::clock::ManualClock`.
- **Success criterion:** launch at T=expiry−1 allowed; launch at
  T=expiry and T=expiry+1 → `GateError::Expired`; run launched before
  expiry finishes past expiry while a new launch is refused (ledger
  distinguishes); backdated approval → `IssuanceError::NotYetGranted`
  at issuance, store stays empty.
- **Non-goals:** marker existence/binding (task 134); submission-time
  expiry (task 146's gate also checks liveness).

## What happened

Pass on the second attempt. The first compile failed: `task_135.rs`
used the `Clock` trait without importing it. The four cases (all
passing after the import fix):

- `launch_before_expiry`: launch at T=3599 with expiry T=3600 →
  `Ok(7)`.
- `launch_after_expiry`: launch at T=3600 (exact boundary) and
  T=3601 → both `Err(GateError::Expired)`.
- `inflight_finishes_new_blocked`: run launched at T=3599 recorded
  `Running`; at T=3700 it transitioned to `Finished` normally (the
  gate is launch-only); a new launch at T=3700 → `Err(Expired)`;
  ledger holds exactly 1 run.
- `backdated_rejected_at_issuance`: approval with `granted_at` in the
  future → `Err(IssuanceError::NotYetGranted)`; zero-TTL approval →
  `Err(IssuanceError::NeverLive)`; approval store size 0 after both.

## The fix — what changed and why

One fix iteration: added the missing `use ...::Clock` import to
`src/tasks/task_135.rs` (the driver calls `Clock` methods on the
manual clock), then ran `cargo fmt` on the new files. No test logic
changed — the four cases passed on the next run.

## Full technical depth

The liveness check is `granted_at <= now && now < expires_at` — the
half-open interval `[granted, expires)`. The half-open form is what
makes the boundary exact: at `now == expires_at` the second
conjunct is false, so expiry is fail-closed at the very second it
names. This is the macaroon time-caveat rule (`time < T` is
satisfied only before T; Birgisson et al., NDSS 2014, use
`time < 2015-01-01T00:00` as the canonical example).

The gate is launch-only by design: nothing re-checks expiry when a
run *finishes*. The adversarial case pins this asymmetry — the
in-flight run completes past expiry (ledger: 1 `Finished`), while a
new launch at the same instant is refused (`GateError::Expired`,
ledger still 1 run). If the gate were completion-checked instead,
the ledger would show the old run stuck; if it were neither, the new
launch would slip through. The case asserts both halves.

Issuance is the second line of defense: `issue_approval` rejects
`granted_at > now` (`NotYetGranted`) and `expires_at <= granted_at`
(`NeverLive`) *before* the approval enters the store. A backdated
approval is therefore not merely invalid at check time — it is never
stored, so no code path can consult it. Note the scaffold's own
`Approval::valid_for` would also return false for such an approval
(`granted_at <= now` fails), so issuance rejection is defense in
depth, not the only barrier.

## Sources

- Primary: Birgisson et al., "Macaroons: Cookies with Contextual
  Caveats for Decentralized Authorization in the Cloud", NDSS 2014
  (https://www.ndss-symposium.org/ndss2014/ndss-2014-programme/macaroons-cookies-contextual-caveats-decentralized-authorization-cloud/):
  time caveats fail closed past their bound; our `[granted_at,
  expires_at)` half-open liveness is that rule.
- Scaffold: `src/bounty/types.rs` (`Approval::valid_for`),
  `src/bounty/approve.rs` (`GateError::Expired`), `src/bounty/store.rs`
  (`RunLedger`), `src/bounty/clock.rs` (`ManualClock`).
