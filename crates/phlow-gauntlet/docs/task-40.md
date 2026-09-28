# task-40: approval TOCTOU

**Kind:** nvim-lua · **Status:** fail (seam absent — no state binding, no revocation, no execution-time re-validation; diver-owned finding, flagged not fixed) · **Wave:** 36–40 · **Commits:** pending (wave 36-40)

## ELI5

TOCTOU is "time-of-check to time-of-use": the guard checks your
ticket at the door, but by the time you sit down, someone swapped
your seat. For approvals: an operator approves "delete the cache
folder". Between the approval and the execution, an attacker swaps
the folder for a symlink pointing at the home directory. If the
executor never re-checks *what exactly was approved*, the delete
runs against the wrong target — with a valid approval.

The design asks for two defenses: the executor must re-validate the
target *at execution time* (compare what is about to run against
what was approved), and execution must check the approval's
*liveness* (is it still granted, or was it revoked?). diver's
`ai.harness.approval` has neither seam: the approval record binds no
approved-against state — no hash, digest, or fingerprint of the
target — so a symlink swap after grant is invisible to the record;
there is no revocation API at all (an approved record can never move
back — the probe's revoke attempt fails with "approval is already
approved"); and nothing in the harness consumes an approval at
execution — the supervisor only sweeps expiries. The design's
"executor re-validates the target" and "execution checks liveness"
have no seam to attach to.

## What this task attempts

- **Goal:** drive the real approval flow — operator requests an
  approval for a write; the approval is granted; then the target is
  changed (symlink replacement after grant); then the executor runs —
  the executor must re-validate the target against the approved
  state. Adversarial: the approval is revoked after grant (grant-
  then-revoke race); execution must check liveness and refuse.
- **Mechanism:** the `task_40.lua` driver in headless Neovim against
  the REAL diver Lua tree — real `approval.request` / `decide` /
  `get`, the real record shape inspected, the real revoke attempt
  made. No mocks of the approval store.
- **Success criterion:** executor re-validates the target at
  execution; execution checks approval liveness.
- **Non-goals:** fixing diver. Diver-owned findings stay flagged,
  never fixed on gauntlet authority.

## What happened

Fail at `"seam"` — on the first and only attempt, honestly. The
approval is a one-way latch with no state binding:

- `approval_grant_roundtrip` (V): the approval request and grant
  round-trip correctly — `decide` approves, `get` reads back
  approved. The approval API half of the seam exists.
- `record_inspected_for_state_binding` (V): the record fields are
  `{ path, ... }` plus approval state — no state hash, digest, or
  fingerprint of the approved-against target. A symlink swap after
  grant would be invisible to the record.
- `revoke_attempt_rejected` (A): attempting to move an approved
  record to denied fails with "approval is already approved" — there
  is no revocation API, so the grant-then-revoke race cannot be
  expressed, let alone checked.
- `no_execution_time_consumer` (A): no execution-time consumer or
  re-validation path exists — nothing in the harness consumes an
  approval at execution; the supervisor only performs expiry
  sweeping. The design's "executor re-validates the target" and
  "execution checks liveness" have no seam to attach to.

## The fix — what changed and why

No product fix was made — diver-owned, flagged not fixed. The
gauntlet-side work was an honest probe:

- **Changed:** `crates/phlow-gauntlet/lua/gauntlet/task_40.lua` (new) —
  drives the real approval API through grant, record-shape
  inspection, the revoke attempt, and the consumer scan; fail-closed
  (`where = "recon"` if the seam ever changes shape).
- **Changed:** `crates/phlow-gauntlet/src/tasks/task_40.rs` (new) —
  thin `nvim-lua` shim, mirroring `task_30.rs`.
- **Why:** a re-validation claim needs a re-validation seam. The
  probe proves the record binds no state, cannot be revoked, and is
  never re-checked at execution — so the honest verdict is
  seam-absent, not a faked pass on the grant round-trip.
- **Source:** `~/workspace/repos/diver/lua/ai/harness/approval.lua`
  (`request`, `decide`, `get`; no state binding, no revocation),
  `~/workspace/repos/diver/lua/ai/harness/supervisor.lua`
  (expiry sweeping only, no approval consumption).
- **Validation agents:** the 2 validation tests
  (`probe_reports_seam_absence`, `probe_exercises_the_real_approval_api`)
  pin the `fail`-at-`seam` verdict and prove the real approval API
  was exercised before concluding.
- **Adversarial agents:** the 2 adversarial tests
  (`verdict_is_a_finding_not_a_probe_crash`,
  `revocation_is_impossible_and_state_is_unbound`) rule out a
  crashing probe masquerading as the finding and pin the
  no-revocation / no-state-binding / no-liveness evidence.

## Full technical depth

`approval.request` creates a pending record; `approval.decide`
moves it to approved (or denied); `approval.get` reads it back. The
record carries the request fields (path, operation) and the
approval state — but no digest of the approved-against state: no
hash of the target's contents, no symlink-resolution snapshot, no
fingerprint. After grant, a symlink replacement at the approved
path is indistinguishable in the record from the original target.

`decide` is a one-way latch: moving an approved record back to
denied fails with "approval is already approved". There is no
`revoke` function and no state transition out of approved — the
grant-then-revoke race the design's adversarial scenario needs
cannot even be expressed, so "execution checks liveness" has
nothing to check against.

Finally, the consumer scan: nothing in the harness reads an
approval at execution time. The supervisor's approval-related code
is expiry sweeping — removing stale pending requests — not a
pre-execution gate. The design's "executor re-validates the target"
would need an executor that (a) exists, (b) reads the approval
record, and (c) compares the current target state against the
bound approved state — all three are absent.

What TOCTOU-safety would need (banked for Matt, not implemented
here): bind the approved-against state into the record (target
digest / resolved-path snapshot at grant time), a revocation path
with liveness, and an execution-time gate that re-resolves the
target and compares it against the bound state before acting. Until
then, an approval is a point-in-time opinion with no memory of what
it approved and no way to take it back.

## Sources

- Primary: `~/workspace/repos/diver/lua/ai/harness/approval.lua`
  (request/decide/get; no state binding; no revocation).
- Primary: `~/workspace/repos/diver/lua/ai/harness/supervisor.lua`
  (expiry sweeping only; no approval consumption at execution).
- Driver: `crates/phlow-gauntlet/lua/gauntlet/task_40.lua` (real
  approval API, headless Neovim).
- Shim: `crates/phlow-gauntlet/src/tasks/task_40.rs`.
- Tests: `crates/phlow-gauntlet/tests/task_40.rs` (2V/2A).
