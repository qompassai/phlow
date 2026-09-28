# task-66: trace propagation

**Kind:** nvim-lua · **Status:** fail (open) · **Wave:** 66–70 · **Commits:** pending (wave 66-70)

## ELI5

When a supervisor hands work to a subagent (or to another session, or an external agent), it should be able to attach a "trace ID" — a tracking number that follows the request across every hop, so later you can ask "what happened for trace X?" and get the whole story. Diver has identity for a *run* (each run gets an id, a parent id, a root id), but that identity stops at the run's own boundary: when the run talks outward through ACP (session.prompt), A2A (tasks.submit), or MCP (client.start), the tracking number is left at the door. Worse, the adapter `probe()` contracts don't even declare "I can't propagate a trace", so the drops are silent — nothing says "your trace ended here." The driver proves it: a probe that asks for propagation across all three transports gets 3/3 drops, and nothing in the seam admits the limitation. (Distinct from task-01/51-style fan-out designs, which concern multi-agent agreement — this is identity continuity across hops.)

## What this task attempts

- **Goal:** verify that trace/correlation IDs survive the three outbound transports: ACP, A2A, MCP.
- **Mechanism:** `lua/gauntlet/task_66.lua` drives the REAL modules (`ai.acp.client`, `ai.a2a.client`, `ai.rose.native.mcp`) with a scripted fake: a `with_trace()` wrapper demands a `trace_id` field on the call payload; each transport's real API is invoked with the trace and the payload is inspected for propagation.
- **Success criterion:** the trace_id arrives across ACP/A2A/MCP, or a sourced honest FAIL — the design says the task fails until propagation is fixed, documenting the hole.
- **Non-goals:** fixing diver on gauntlet authority; inventing a trace envelope.

## What happened

Honest FAIL at `where = "seam"`, first attempt:

- **V1:** `session.prompt(session_key, run.goal, cb)` — ACP's real signature — has no parameter slot for a trace; a traced call drops the trace_id silently.
- **V2:** `tasks.submit({agent, message, timeout_ms, on_done})` — A2A's real payload — has no trace field; the hop carries identity-free.
- **A1:** an explicit continuity check across all three transports records 3/3 drops.
- **A2:** the adapter `probe()` contracts neither declare propagation nor admit inability to propagate — the drops are silent by contract design.

## Full technical depth

Diver's run identity lives in `run.id` / `run.parent_id` / `run.root_id` (created in the session layer), but the three outbound transports build their payloads without any of it. ACP's `session.prompt` takes `(session_key, goal, callback)`; A2A's `tasks.submit` takes the `{agent, message, timeout_ms, on_done}` table; MCP's `client.start(server_spec, callback)` takes the server spec and callback. None of the three accepts or forwards a trace envelope, and no wrapper injects one — the tracking number genuinely ends at the run boundary. The probe contracts (`probe()`) declare health/capabilities but have no propagation declaration, so a caller cannot even discover the limitation mechanically.

Diver-owned (flagged, never fixed on gauntlet authority): add a trace envelope to the three outbound payloads, and declare propagation capability (or inability) in the adapter `probe()` contract so drops are loud.

## Sources

- `~/workspace/repos/diver/lua/ai/acp/client.lua` — `session.prompt(session_key, run.goal, cb)`
- `~/workspace/repos/diver/lua/ai/a2a/client.lua` — `tasks.submit({agent, message, timeout_ms, on_done})`
- `~/workspace/repos/diver/lua/ai/rose/native/mcp.lua` — `client.start(server_spec, callback)`
- `~/workspace/gauntlet-design-tasks-21-70.md` — task-66 design (Wave 12)
