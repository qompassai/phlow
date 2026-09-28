# task-90: namespaced cross-protocol dispatch

**Kind:** nvim-lua (validation) · **Status:** fail (open) · **Wave:** 86–90 · **Commits:** pending (wave 86-90)

## ELI5

Imagine two phone books — one for doctors, one for plumbers — and a rule that says every entry must carry its book's name: `doctor:Smith` vs `plumber:Smith`, so a name collision can never send you to the wrong profession. Diver's harness has no such rulebook: there is no dispatcher that understands `mcp:summarize` vs `a2a:summarize`. Each run is simply bound to one adapter (`spec.adapter`), calls stay native to that adapter, and the tool registry *rejects* any name with a colon — `register_tool(reg, 'mcp:summarize')` fails the `^[a-z][a-z0-9_]*$` name pattern. A tool literally named `a2a:send` advertised over MCP is called as the literal name on the MCP wire — the prefix is never parsed, never dispatched to A2A. So a cross-protocol mixup is impossible — but only because the addressing it would need is *inexpressible*, not because it is guarded. The design's criteria — namespaces assigned by the router, ambiguous unqualified names rejected — have no mechanism to assert against.

## What this task attempts

- **Goal:** verify a namespaced cross-protocol dispatcher exists — every target carries its protocol namespace, cross-protocol collisions cannot cause cross-protocol dispatch, unqualified ambiguous names are rejected, namespaces are router-assigned never parsed from advertised names — or document the absence with module evidence.
- **Mechanism:** `lua/gauntlet/task_90.lua` drives the REAL `ai.harness` modules headless in four scenarios: `distinct` (the real `ai.harness.adapters.mcp` / `ai.harness.adapters.a2a` are distinct registrations — verified via `debug.getinfo(mcp.start,'S').source` — and an mcp run against the mock completes with only mcp/supervisor-sourced events; trace: `adapters_disjoint=true`, `cross_protocol_traffic=false`); `no-namespace` (`ai.harness` exposes only setup/run/cancel/resume/version; the registry only adapter/tool/workflow registration; `register_tool` rejects `mcp:summarize`; trace: `dispatch_entry=false`, `colon_names_rejected=true`); `spoof` (a mock tool literally named `a2a:send` is called with the literal name on the MCP wire — `prefix_parsed=false`, `cross_protocol_dispatch=false`); `ambiguous` (a bare-name duplicate is rejected as 'already registered', but a cross-protocol collision is inexpressible — `bare_duplicate_rejected=true`, namespaced lookup resolves to nothing). `src/tasks/task_90.rs` runs the driver scenarios and probes the machine-readable `dispatch-trace.json`.
- **Success criterion:** namespaced dispatch verified, or the absence documented with module evidence.
- **Non-goals:** adding a dispatcher on gauntlet authority (it is Diver-owned — flagged, never fixed here).

## What happened

Honest FAIL at `where = "seam"`, first attempt — the seam is ABSENT as designed:

- **V1:** adapters are disjoint — distinct registrations, protocol-pure start paths, per-run adapter binding; no call crosses protocols.
- **V2:** no namespace entry point — no dispatch function, no namespace registry; `proto:name` addressing is rejected by the name pattern.
- **A1:** the spoofed prefix stays opaque — the literal name `a2a:send` travels on the MCP wire, never parsed; safe only because no namespace machinery exists at all.
- **A2:** ambiguous names are inexpressible — a bare duplicate fails closed, but a cross-protocol collision cannot be expressed, let alone rejected as ambiguous.

## Full technical depth

The driver resolves `ai.harness`, `ai.harness.registry`, and the real adapter modules through the rtp shim (read-only; Matt's diver files are never touched) and writes machine-readable traces the Rust harness probes independently. The verdict logic lives in `src/tasks/task_90.rs`: the driver's per-scenario pass/fail is about the *mechanism* (are the adapters the real modules? does the run complete protocol-pure?), while the harness probes assert the *absence* (dispatch_entry=false, colon_names_rejected=true, prefix_parsed=false, bare_duplicate_rejected=true) and the task-level verdict reports the honest seam absence.

The MCP adapter's own header documents the design intent that was probed: "tool calls stay on the native ai.mcp.tools API". Routing is per-run adapter binding via `spec.adapter` (or capability negotiation in `ai.harness.adapter.negotiate`, task-64's seam) — never a namespaced cross-protocol dispatch. The design's threat model (a collision across protocols causing cross-protocol dispatch) is currently impossible *vacuously*: the addressing it would need cannot be written.

Distinct from task-64 (deterministic adapter *selection*) and task-86 (MCP capability *negotiation*): this is *addressing* — how a target names its protocol — and there is no namespaced addressing to select or negotiate with.

Diver-owned finding: flagged, never fixed on gauntlet authority — whether the harness should gain a namespaced cross-protocol dispatcher (router-assigned namespaces, ambiguous-name rejection) is Matt's call.

## Sources

- `~/workspace/repos/diver/lua/ai/harness/adapters/mcp.lua`, `~/workspace/repos/diver/lua/ai/harness/adapters/a2a.lua` — the real adapter modules under test (distinct registrations; calls stay native to their adapter)
- `~/workspace/repos/diver/lua/ai/harness/registry.lua` — the name pattern `^[a-z][a-z0-9_]*$` that rejects `proto:name`
- `~/workspace/gauntlet-design-tasks-71-100.md` — task-90 design (Wave 86–90)
