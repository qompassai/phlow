# task-87: MCP session resumption

**Kind:** nvim-lua (validation) · **Status:** fail (open) · **Wave:** 86–90 · **Commits:** pending (wave 86-90)

## ELI5

When your phone drops a call and redials, it should pick up where you left off — not start a brand-new conversation with no memory of the first. Diver's MCP client has no "redial with memory": when the server process dies, the client's `teardown` deletes the whole session record. The next call just fails with "server is not running" until you manually start a new session — and that new session has no idea it is a *resumption*. It re-handshakes from scratch, it keeps no note of what tools the old server had (so if the restarted server now offers different tools, the client serves them with no warning), and if a tool call was in flight when the server died, the caller gets a plain string — never a typed "I don't know if that ran" — with no idempotency tracking to tell a safe retry from a dangerous double-execution.

## What this task attempts

- **Goal:** verify the client auto-resumes after a between-calls death (session record, resumption-boundary marker, tool-list identity invalidated on change, in-flight calls answered with typed `unknown_call_outcome`, idempotency metadata distinguishing safe retries) — or document the absence with process-death evidence.
- **Mechanism:** `lua/gauntlet/task_87.lua` drives the REAL `ai.mcp.client` headless against a killable scripted Python MCP stdio server in four scenarios: `restart-between` (death between calls → `M.request` reports 'server is not running'; no auto-resume; manual `M.start` re-handshakes and `tools/list` works; trace: `auto_resume=false`, `rehandshake_ok=true`, `resumption_boundary_marked=false`); `identity-change` (restarted server returns a different tool list — served as-is; trace: `old_tools`/`new_tools` differ, `session_invalidated=false`); `inflight` (server dies mid-`tools/call` — exactly 1 wire call, never retried, but the error is a plain string: `typed_unknown_call_outcome=false`, `idempotency_tracked=false`); `no-record` (the client module exposes no session-record API at all: trace `session_record_api=false`, `resumption_boundary=false`). `src/tasks/task_87.rs` runs the driver scenarios and probes the machine-readable `resume-trace.json`.
- **Success criterion:** resumption machinery verified, or the absence documented with death-and-restart evidence.
- **Non-goals:** adding resumption machinery on gauntlet authority (it is Diver-owned — flagged, never fixed here).

## What happened

Honest FAIL at `where = "seam"`, first attempt — the seam is REAL but does not meet the criteria:

- **V1:** no auto-resume — the session is deleted on process death; recovery is a manual `M.start` from scratch; no resumption boundary is marked.
- **V2:** changed tool list served without invalidation — the restarted server's different tools are served as-is; there is no tool-list identity to compare against.
- **A1:** in-flight death is untyped and single-attempt — exactly 1 wire call (no blind retry — the teardown gives at-most-once *by accident*, not by discipline), but the fate surfaces as a bare string with no idempotency tracking, so a caller cannot tell a safe retry from an unsafe one.
- **A2:** no session-record API — `teardown` deletes the session outright; there is no record in which a re-handshake could be observed or a resumption boundary marked.

## Full technical depth

The driver resolves `ai.mcp.client` through the rtp shim (read-only; Matt's diver files are never touched), registers a per-scenario scripted Python server (steered by `GAUNTLET_MCP_TOOLSET`: toolset `a` vs `b` for the identity change; the `gauntlet-die` tool triggers process exit; `gauntlet_suicide` exits(1) mid-call with no reply), and writes machine-readable traces the Rust harness probes independently. The verdict logic lives in `src/tasks/task_87.rs`: the driver's per-scenario pass/fail is about the *mechanism* (did death/restart behave as documented?), while the harness probes assert the *absence* (auto_resume=false, session_invalidated=false, typed_unknown_call_outcome=false, session_record_api=false) and the task-level verdict reports the honest seam failure.

Source-level mechanism (verified by reading `lua/ai/mcp/client.lua`, not inferred): process exit invokes `teardown`; `teardown` increments the generation, resolves pending calls with a plain string, closes the backend, then deletes `sessions[name]` — no automatic re-handshake, no session record, no tool-list identity, no idempotency metadata, no resumption marker exists anywhere in the module.

Distinct from task-86 (capability *negotiation* at handshake): this is the session's *lifetime* — what survives a server death — and the answer is "nothing, by design of teardown".

Diver-owned finding: flagged, never fixed on gauntlet authority — whether diver's MCP client should gain resumption machinery (session records, at-most-once/idempotency tracking, tool-list identity, invalidation on identity change) is Matt's call.

## Sources

- `~/workspace/repos/diver/lua/ai/mcp/client.lua` — the real client under test (teardown deletes sessions[name]; pending calls resolved with plain strings; no resumption machinery)
- `~/workspace/gauntlet-design-tasks-71-100.md` — task-87 design (Wave 86–90)
