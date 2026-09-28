# task-154: versioned envelope routing

**Kind:** rust (validation) · **Status:** pass · **Wave:** 25 · **Commit:** pending (wave 25 commit)

## ELI5

Every message carries a version number, because the protocol will change over time. The receiver has to look at that number and send the message to the right parser — v1 messages to the v1 parser, v2 to the v2 parser — and if the version is nonsense (too old, missing entirely), it must say so with a clear, typed error instead of crashing or, worse, quietly misreading the message with the wrong parser. This task proves the routing: v1 and v2 messages interleaved on one connection each reach their version, every reply carries the matching version back, and version 0 / a missing version produce `TooOld` / `Missing` errors while the connection stays up.

## What this task attempts

- **Goal:** `envelope.version` routes to the matching parser; unroutable versions fail with typed `VersionError`s; the connection survives refusals.
- **Mechanism:** `crates/phlow-gauntlet/src/wire.rs` — `route_version`, `EditorSocket::read`; driver `crates/phlow-gauntlet/src/tasks/task_154.rs`.
- **Success criterion:** 6 interleaved v1/v2 envelopes → routed `[1,2,1,2,1,2]`, 6 responses with matching versions, connection up; version 0 → `TooOld{got:0,min:1}`, missing version → `Missing`, 2 rejections counted, connection up and still reading.
- **Non-goals:** downgrade attacks and absurd versions (task 157); unknown kinds (task 151).

## What happened

Passed. `cargo test -p phlow-gauntlet --test task_154` → 3 passed, 0 failed. The interleaved stream routed exactly as sent, every response carried its envelope's version, and the connection stayed up throughout. Version 0 produced `VersionError::TooOld`, the missing version produced `VersionError::Missing`, the socket counted 2 rejections, and a subsequent good envelope still read cleanly.

## The fix — what changed and why

No driver fixes; one lint fix during gating:

- **Changed:** `src/tasks/task_154.rs` — `let mut socket` → `let socket` in `case_interleaved_versions_routed`.
- **Why:** the case only calls `socket.is_alive()` (immutable); clippy `unused_mut` under `-D warnings` is a gate failure.
- **Source:** clippy `unused_mut`.
- **Validation:** full gate sequence re-run green.

## Full technical depth

`route_version` is an if-chain over ranges — `None → Missing`, `v < 1 → TooOld`, `v > 3 → Unsupported`, else `Ok(v)` — never `parsers[version]`, so there is no dispatch table to index out of or overflow (task 157 leans on this for the absurd-version case). The supported range 1–3 is a named bound pair (`MIN_SUPPORTED_VERSION`, `MAX_SUPPORTED_VERSION`), not literals. `EditorSocket::read` parses, then routes, then counts: a refusal increments `rejected` and returns `SocketError::Frame`/`SocketError::Version` — the `alive` flag has no transition to `false` anywhere on the read path, so refusal is never a disconnect by construction. The interleaved case matters because version routing is per-envelope, not per-connection: a single stream legitimately carries mixed versions during upgrades, and routing must not latch onto the first version seen. The typed errors carry their context (`got`, `min`) so a peer can diagnose a skew without guessing.

## Sources

- Ghostex versioned-envelope rule: `rpc.rs` `protocolVersion` gate, `event.rs` `EventHeader` @ c91146607205ac49303d1bcfe2fd6f9a86741500 (adaptation map). Unlike Ghostex's exact-match gate, this build routes a *range* of versions — the adaptation is documented in the driver's module docs.
- `crates/phlow-gauntlet/src/wire.rs` — `route_version`, `VersionError`, `EditorSocket`.
- `crates/phlow-gauntlet/src/tasks/task_154.rs`, `crates/phlow-gauntlet/tests/task_154.rs`.
