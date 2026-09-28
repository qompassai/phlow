# task-86: MCP capability negotiation mismatch

**Kind:** nvim-lua (validation) · **Status:** fail (open) · **Wave:** 86–90 · **Commits:** pending (wave 86-90)

## ELI5

When a phone and a tower connect, they first agree on what languages they both speak — "I can do calls and texts", "me too". If the phone just says "I can do anything" and never checks what the tower actually supports, it will happily try to send a video call to a tower that only does text. Diver's MCP client does exactly that: its handshake says `capabilities = {}` (an empty Lua table, which encodes on the wire as `[]` — the same interop gap task-08 found), then throws away whatever the server says it supports. It keeps no record of the negotiation, so every tool call goes out on the wire unchecked. A server that advertises "I can do tools" but then fails every tools call is never caught lying — the failure comes back as a plain string, not a typed "you said you could". And a call for something the server never advertised (like `resources/list` from a tools-only server) is sent anyway.

## What this task attempts

- **Goal:** verify the client records negotiated capabilities, gates calls on them, surfaces advertisement-vs-behavior mismatches as typed `capability_mismatch` errors, and never sends unadvertised methods — or document the absence with wire evidence.
- **Mechanism:** `lua/gauntlet/task_86.lua` drives the REAL `ai.mcp.client` headless against a scripted Python MCP stdio server in four scenarios: `match` (handshake/list/call round-trip works, but the trace shows no introspection API and the wire carried the client's fixed empty capabilities); `record` (the trace shows `client_advertised_caps='[]'` and `negotiated_record=false` — the server's advertised capabilities exist nowhere in the client); `lie` (server advertises `tools` but fails every `tools/list` with -32602 — the error surfaces as a plain string, `typed_capability_mismatch=false`); `unadvertised` (server advertises only `tools`; the client's `resources/list` call still goes on the wire, proven by the mock's wire log). `src/tasks/task_86.rs` runs the driver scenarios and probes the machine-readable `cap-trace.json`.
- **Success criterion:** negotiated-capability gating verified, or the absence documented with wire evidence.
- **Non-goals:** adding capability machinery on gauntlet authority (it is Diver-owned — flagged, never fixed here).

## What happened

Honest FAIL at `where = "seam"`, first attempt — the seam is REAL but does not meet the criteria:

- **V1:** matching capabilities round-trip — handshake/list/call work against the real client. The mechanism works; the negotiation record around it does not exist.
- **V2:** negotiation never recorded — no capability/session introspection API; the wire shows the client's fixed empty capabilities; the server's advertisement exists nowhere in the client.
- **A1:** the advertising lie has no typed mismatch — an advertised-but-broken capability surfaces as a bare string; the lie is indistinguishable from an ordinary call failure.
- **A2:** unadvertised methods still go on the wire — `M.request` sends any method for any ready session; the mock's wire log proves `resources/list` was sent to a tools-only server.

## Full technical depth

The driver resolves `ai.mcp.client` through the rtp shim (never a hardcoded diver path; Matt's diver files are never touched — the shim is read-only), spins a scripted Python MCP server per scenario (the mock's wire log is the ground truth for what was actually sent), and writes machine-readable traces the Rust harness probes independently. The verdict logic lives in `src/tasks/task_86.rs`: the driver's per-scenario pass/fail is about the *mechanism* (did the handshake/call behave as documented?), while the harness probes assert the *absence* (negotiated_record=false, typed_capability_mismatch=false, unadvertised_call_sent=true) and the task-level verdict reports the honest seam failure.

Source-level mechanism (verified by reading `lua/ai/mcp/client.lua`, not inferred): `run_handshake` sends a fixed protocol version (`'2024-11-05'`) and `capabilities = {}`; the initialize callback names the result `_result` and discards it; server-advertised capabilities are neither read nor recorded; `M.request` sends any method for any ready session with no negotiated-capability gating. Two interop notes for Matt: the empty-Lua-table-`{}`-encodes-as-`[]` quirk (task-08's gap, observed on the wire here too), and the client speaking `2024-11-05` while phlow-mcp supports only `2025-*` (task-89's skew, silently accepted on both ends).

Distinct from task-07 (the A2A boundary) and task-89 (the server-side version negotiation in phlow-mcp): this is the *client-side* capability record — the client's copy of the negotiation — and it is empty.

Diver-owned finding: flagged, never fixed on gauntlet authority — whether diver's MCP client should record negotiated capabilities, gate calls on them, and type mismatch errors is Matt's call.

## Sources

- `~/workspace/repos/diver/lua/ai/mcp/client.lua` — the real client under test (run_handshake sends `protocolVersion='2024-11-05'`, `capabilities={}`; initialize callback discards `_result`; M.request gates on nothing)
- `~/workspace/gauntlet-design-tasks-71-100.md` — task-86 design (Wave 86–90)
