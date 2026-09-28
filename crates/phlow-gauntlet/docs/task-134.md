# task-134: passive→active approval gating

**Kind:** rust · **Status:** pass · **Wave:** 23 · **Commits:** <worktree commit on gate>

## ELI5

Looking is free; touching costs permission. Passive recon (reading
public scope data) needs no approval, but the moment the agent wants
to actively probe a target it must hold a permission slip — an
approval marker from the operator. The slip says *which* scope version
it's good for, and the gate checks three things in order: did it
really come from the operator (not a forged copy), is it for *this*
program and *this* scope version, and is it still live. A slip for
last week's scope, or one the operator never signed, gets the probe
refused — with a specific reason, not a shrug.

## What this task attempts

- **Goal:** verify the launch-path gate: passive launches with no
  approval; active launches only on a live operator approval bound to
  the current scope version.
- **Mechanism:** driver-local `gate_launch` in
  `crates/phlow-gauntlet/src/tasks/task_134.rs` (check order mirrors
  `bounty::approve::SubmissionGate::submit`: provenance → binding →
  liveness), returning the bound approval nonce on success; typed
  refusals reuse `bounty::approve::GateError`.
- **Success criterion:** passive + no approval → launch; active +
  `Approval{scope_version: 2}` on scope v2 → launch, nonce bound;
  approval for v1 on scope v2 → `GateError::ScopeVersionMismatch`;
  forged marker (wrong issuer) → `GateError::NoApproval`.
- **Non-goals:** the time dimension of a valid marker (task 135);
  the submission gate's hash binding and nonce single-use
  (task 146).

## What happened

Pass on the second attempt. The first compile failed: `task_134.rs`
used the `Clock` trait without importing it. The four cases (all
passing after the import fix):

- `passive_needs_no_approval`: passive launch with no marker →
  `Ok(0)` (no nonce needed).
- `active_with_bound_approval`: active probe with
  `Approval{issuer: "operator", scope_version: 2}` on scope v2 →
  `Ok(42)`, the run bound to nonce 42.
- `stale_scope_approval_refused`: approval bound to v1, scope now v2
  → `Err(GateError::ScopeVersionMismatch)`.
- `forged_marker_refused`: correct-shaped marker with issuer
  `"bbscope-feed"` → `Err(GateError::NoApproval)` — provenance, not
  shape; operator approval for `prog-2` used against `prog-1` →
  `Err(GateError::ScopeVersionMismatch)`.

## The fix — what changed and why

One fix iteration: added the missing `use ...::Clock` import to
`src/tasks/task_134.rs` (the driver calls `Clock` methods on the
manual clock), then ran `cargo fmt` on the new files. No test logic
changed — the four cases passed on the next run.

## Full technical depth

The approval is modeled as a capability token with macaroon-style
caveats: `scope_version`, `program_id`, and the `[granted_at,
expires_at)` interval are caveats attenuating the token, and the
`issuer == "operator"` check is the provenance root (macaroons are
verifiable only by the party holding the root key — here, the
operator authority is the trust root by construction of the driver).
The gate fails closed on every unsatisfied caveat and returns the
*first* violated check in a fixed order, so error attribution is
deterministic: provenance (`NoApproval` for a missing or
non-operator marker) → binding (`ScopeVersionMismatch`) → liveness
(`Expired`).

The forged-marker case is the sharp one: the fixture is well-formed
in every field except `issuer`. A shape-only check would pass it;
the gate refuses it because provenance comes first. Note the nonce
replay question is deliberately *not* the launch gate's job — nonces
are single-use at the *submission* gate (task 146), which keeps a
spent-nonce set. The launch gate binds the run to the nonce so the
submission gate can check it later.

## Sources

- Primary: Birgisson et al., "Macaroons: Cookies with Contextual
  Caveats for Decentralized Authorization in the Cloud", NDSS 2014
  (https://www.ndss-symposium.org/ndss2014/ndss-2014-programme/macaroons-cookies-contextual-caveats-decentralized-authorization-cloud/):
  caveats attenuate and contextually confine a bearer credential
  (what/when/where); a time caveat like `time < 2015-01-01T00:00`
  fails closed past its bound. Our `scope_version` and expiry checks
  are first-party caveats in this sense; the issuer check is the
  root-key trust anchor.
- Scaffold: `src/bounty/types.rs` (`Approval`), `src/bounty/approve.rs`
  (`GateError`, and the check ordering of `SubmissionGate::submit`
  that `gate_launch` mirrors).
