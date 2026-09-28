# task-163: hostile peer on the daemon socket

**Kind:** rust · **Status:** pass · **Wave:** 26 ·
**Commits:** single wave commit `gauntlet: wave 26 drivers, tests, docs`

## ELI5

The daemon listens on a local socket — and *any* program on the
machine can connect to it, including a malicious one. "It's local"
is not a security argument. This task proves the daemon treats every
peer as hostile until proven otherwise: a well-formed privileged
command with no credential gets a typed `AuthError` and the
connection is dropped — zero privileged effects run. And if the
attacker replays a *captured, valid* snapshot frame (right
credential, but old), the daemon rejects it as `StaleSnapshot` and
its state doesn't change by a single key. Authentication is checked
before anything else: a credential-less replay is `AuthError`, not
`StaleSnapshot`.

## What this task attempts

- **Goal:** unauthenticated privileged frames are refused with typed
  errors and zero effects; replayed valid-but-stale snapshots are
  stale-rejected; the daemon's state is unchanged by either; attempts
  are audit-logged.
- **Mechanism:** `DaemonFixture` accept path (credential check before
  op parsing/effects) in
  `crates/phlow-gauntlet/src/daemon_client.rs`, driven by
  `crates/phlow-gauntlet/src/tasks/task_163.rs` with raw TCP probes
  over real loopback sockets.
- **Success criterion:** `AuthError` for no-cred and wrong-cred
  privileged ops with `privileged_effects == 0`; `StaleSnapshot`
  naming `current_version=5` for the replayed `{version:4, as_of:90}`
  frame; daemon state before == after; ≥2 `StaleSnapshot` audit lines.
- **Non-goals:** parser-level attacks (tasks 155–157), the client's
  own snapshot handling (task 161).

## What happened

Both adversarial cases pass on the final gates: `cargo test -p
phlow-gauntlet --test task_163` → 2 passed, 0 failed; lib 64/64; fmt
and clippy clean. A1: no-cred `exec` → `AuthError`; wrong-cred `exec`
→ `AuthError`; privileged effects executed: `[]` (none); the positive
control — a credentialed `ping` — returns `pong`, proving framing is
fine and auth is the gate. A2: replayed valid snapshot
(`version=4, as_of=90`, good cred) → `StaleSnapshot` with
`current_version=5`; as_of-only-stale (`version=5, as_of=95`) →
`StaleSnapshot`; cred-less replay → `AuthError` (auth checked first);
daemon state before == after; audit holds ≥2 `StaleSnapshot` lines.

## Where it went wrong

- **Stage:** A2 helper extraction (refactor for the ≤70-line bound).
- **Symptom:** `cargo clippy -- -D warnings` failed:
  `error: variable does not need to be mutable` on
  `let mut evidence` in the A1 case.
- **Evidence:** clippy `-D unused-mut` after the case was converted
  to the shared `finish` helper, which takes ownership of the
  evidence vec.
- **Root cause:** leftover `mut` from the pre-refactor inline tail.
  Removed — no behavior change.

No authentication bypass was found: the credential check runs before
op parsing and before any effect, so malformed-but-credentialed and
well-formed-but-credential-less frames both fail closed.

## The fix — what changed and why

- **Changed:** `crates/phlow-gauntlet/src/tasks/task_163.rs` —
  `expect_err` helper (one probe + typed-error assertion), `finish`
  helper with `case: &'static str`, `let evidence` (no `mut`).
- **Commit:** single wave commit `gauntlet: wave 26 drivers, tests,
  docs`.
- **Why:** the three replay probes differ only in payload and wanted
  error; one helper removes the copy-paste that previously let the
  auth-before-staleness assertion drift between copies.
- **Source:** the credential/staleness contract is ours, defined in
  `DaemonFixture` (`daemon_client.rs`): credential checked before
  operation parsing/effects; staleness is `version < current` or
  (`version == current` and `as_of < current_as_of`).
- **Validation agents:** the subagent author ran the full gate set on
  primo after the fix: targeted 2/2, lib 64/64, fmt clean, clippy 0
  warnings.
- **Adversarial agents:** same author — tried a replay with a *fresh*
  version but old `as_of` (rejected: as_of-stale), a replay with the
  credential stripped (rejected as `AuthError`, not `StaleSnapshot`),
  and a privileged `exec` with the credential of a *different*
  fixture token (rejected). All failed closed.
- **Citations:** loopback-is-not-trust is the standing assumption
  (design notes); Rust std `std::net::TcpStream` docs for the raw
  probe path.

## Full technical depth

The fixture protocol: 4-byte big-endian frame length + JSON body,
`MAX_FRAME_BYTES = 64 KiB` cap. On accept, the fixture reads one
frame with `FRAME_READ_TIMEOUT = 250 ms`, parses JSON, and checks
`cred` against its token *before* dispatching on `op` — so `exec`,
`snapshot`, and `subscribe` are all unreachable without a valid
credential, and a parse failure in the op can never run an effect.
Privileged effects append to a shared effects log the driver inspects;
the A1 run shows it empty.

Staleness is a version *and* cursor check: the daemon tracks
`(version, as_of)`. A snapshot frame is stale if `version <
current_version`, or `version == current_version && as_of <
current_as_of`. The rejection is typed (`StaleSnapshot` naming
`current_version`) so the client can distinguish "you're behind, fetch
newer" from "you're not authenticated". The audit log records every
rejection with its reason — the driver's ≥2-line assertion pins the
logging, not just the refusal. State is snapshotted before the probes
and compared after: byte-equal.

## Sources

- Primary: `crates/phlow-gauntlet/src/daemon_client.rs`
  (`DaemonFixture`: credential-before-op contract, `StaleSnapshot`
  rule, `FRAME_READ_TIMEOUT`, `MAX_FRAME_BYTES`).
- Primary: Ghostex `packages/gx-client/src/socket.rs` @
  c91146607205ac49303d1bcfe2fd6f9a86741500 (socket accept-path
  doctrine; the credential/staleness contract itself is ours).
- Secondary: wave 26 design notes
  (`~/workspace/gauntlet-design-tasks-151-200.md`, task-163 section).
