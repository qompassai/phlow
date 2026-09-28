# task-89: protocol version skew

**Kind:** rust (driver) · **Status:** fail (open) · **Wave:** 86–90 · **Commits:** pending (wave 86-90)

## ELI5

When two sides speak slightly different versions of a protocol, they should negotiate the *best version they both support* — or fail loudly. What phlow-mcp's server actually does: known versions negotiate correctly (you offer 2025-06-18, it answers 2025-06-18), but *unknown* versions — garbage like "banana" and a future version like "2999-99-99" — are silently treated as "give me the newest". The server cannot tell a future version from garbage, and a client that only implements an older version is told "the server speaks the newest", which could unlock behavior the client doesn't understand. And if a client tries to re-initialize mid-session with a new version, the server says "already initialized" (-32600) but keeps the old session running unchanged — no invalidation, no fresh negotiation.

Meanwhile the other side of the skew is diver's client (diver-owned, flagged): it sends `protocolVersion = '2024-11-05'` — older than every version phlow-mcp supports — and then *throws away* the initialize result, so the negotiated version never reaches any feature gate. Skew is silently accepted on both ends.

## What this task attempts

- **Goal:** verify the server negotiates the greatest mutually supported version or fails closed; future versions never unlock unimplemented behavior; mid-session version changes invalidate the session; garbage versions are a typed `version_negotiation_failed` — or document the deviations with reply evidence.
- **Mechanism:** `src/tasks/task_89.rs` drives the REAL `phlow_mcp::McpServer<FakeRuntime>` through its public `reply()` in four cases: `matching_versions_negotiate` (offer 2025-11-25 → 2025-11-25); `skew_negotiates_older` (offer 2025-06-18 → 2025-06-18); `garbage_and_future_silently_upgrade` (offer "banana" and "2999-99-99" → both negotiate 2025-11-25, no error); `mid_session_change_not_invalidated` (initialize 2025-06-18 + notifications/initialized → ready; second initialize 2025-11-25 → -32600 "Already initialized"; session still ready; ping still succeeds).
- **Success criterion:** the skew discipline verified, or the deviations documented with reply evidence.
- **Non-goals:** changing `phlow-mcp`'s negotiation on gauntlet authority (it is phlow product code — the decisions are banked for Matt, never implemented here).

## What happened

Honest FAIL at `where = "seam"`, first attempt — the seam is REAL and half-capable:

- **V1:** matching versions negotiate — offered 2025-11-25, negotiated 2025-11-25. Known-version negotiation works.
- **V2:** skew negotiates the greatest mutually supported version — offered 2025-06-18, negotiated 2025-06-18. The supported-version path is correct.
- **A1:** garbage and future versions silently upgrade — "banana" and "2999-99-99" both negotiate 2025-11-25 with no typed error. The design's `version_negotiation_failed` does not exist; a future version unlocks the newest behavior for a client that may not implement it.
- **A2:** mid-session version change does not invalidate — the second initialize is rejected (-32600) but the session survives on the old version (ping still succeeds). The design's invalidation does not happen.

## Full technical depth

The driver builds frames through `serde_json::json!` and reads `result.protocolVersion` / `error.code` from the real server's replies — no mocks, no reimplementation of the negotiation. The helper `negotiated_or_code` distinguishes the two reply shapes. The mid-session case uses the public `is_ready()` to observe session liveness before and after the rejected re-initialize, and a `ping` to prove the session continues on the old version.

Source-level mechanism (verified by reading `crates/phlow-mcp/src/server.rs` and `protocol.rs`, not inferred): `SUPPORTED_VERSIONS = ["2025-11-25", "2025-06-18", "2025-03-26"]`; any unknown nonempty version — future or garbage — is mapped to the newest (`PROTOCOL_VERSION = "2025-11-25"`) with no typed error; empty is rejected; a second `initialize` returns -32600 "Already initialized" and leaves the session untouched. The `phlow-mcp` crate is a path dependency of the gauntlet (added for task-77), so this is the real product negotiation, exercised through its public surface.

Distinct from task-86 (the *client's* capability record): this is the *server's* version negotiation — and from the diver client's side, which speaks an older version and discards the negotiated one (diver-owned, flagged).

Product decisions banked for Matt (phlow-owned): whether unknown versions should fail closed with a typed `version_negotiation_failed` instead of silently upgrading to newest, and whether a mid-session re-initialize should invalidate the session. Not implemented on gauntlet authority — `phlow-mcp` is untouched.

## Sources

- `~/workspace/repos/phlow/crates/phlow-mcp/src/server.rs` — the real negotiation under test (McpServer::initialize; unknown versions → newest; second initialize → -32600, session untouched)
- `~/workspace/repos/phlow/crates/phlow-mcp/src/protocol.rs` — PROTOCOL_VERSION = "2025-11-25"; SUPPORTED_VERSIONS (2025-11-25, 2025-06-18, 2025-03-26)
- `~/workspace/gauntlet-design-tasks-71-100.md` — task-89 design (Wave 86–90)
