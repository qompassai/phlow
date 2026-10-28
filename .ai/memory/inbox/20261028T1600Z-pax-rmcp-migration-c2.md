# phlow-mcp rmcp migration (fleet standardization Stage C2)

Date: 2026-10-28. Agent: Pax (subagent). Program:
`~/workspace/mcp-1.0-program-20261028/` (FLOWGUARD-MAPPING.md is
the authority; GATE-REPORT-C2.md is the gate record).

## What changed

- phlow-mcp's hand-rolled dual-era MCP framing is replaced by
  rmcp 3.8.0 (pinned `=3.8.0`, features server/client/
  transport-io/transport-child-process) plus a byte-level era
  guard (`crates/phlow-mcp/src/guard.rs`, fleet pattern) that
  refuses `initialize` with the typed -32022 error, enforces the
  1 MiB line bound (drain-and-stay-alive), the fleet framing
  contract, request-id validity (GAP-1 fallback (ii): >i64 ids
  refused -32600 null id), and strict tools/call params (GAP-2).
- The domain dispatch (`src/server.rs`) is era-free:
  `McpServer::call_tool(name, args, caller)` returns
  `CallOutcome::{Result, Refusal}`; identity comes from the
  per-request `_meta` clientInfo (handler.rs). Tool result text
  keeps the json_ascii `dumps` bytes (GAP-3 option (i));
  tools/list re-baselined with full annotations on all 7 tools
  (GAP-6): wire-verified 2,253 B baseline → 2,846 B, the +593 B
  delta is exactly the annotation objects; discover and tool
  payloads are semantically identical to the pre-migration wire.
- The outbound client (`src/client.rs`) is rebuilt on rmcp's
  client, modern-only (GAP-4): `server/discover` lifecycle,
  legacy-only servers refused as `ClientError::Negotiation`.
  Profile loader, bounds, and error surface preserved.
  `examples/mcp_probe.rs` is the live-probe harness.
- phlow-cli `serve` now calls `phlow_mcp::serve_stdio`.
- Untouched (verified empty diff): phlow-attention,
  phlow-runtime, phlow-checks, phlow-approval, phlow-workspace.

## Benchmark ports (same treatment as task-89's subject change)

- gauntlet task-89 re-based from the deleted `McpServer::reply`
  negotiation seam onto `phlow_mcp::classify`; verdict flips to
  Pass (the documented fail-closed gap is what the migration
  closed). Cases renamed; docs/task-89.md records the resolution.
- gauntlet task-49 V2: the tokio classification gained the
  phlow-mcp rmcp framing as a named transport-class carve-out
  (like the msgpack worker's); the case's subject (no agent-task
  worker pool) is unchanged.

## Evidence commands

- Sandbox: `cargo test -p phlow-mcp` (51 green), `-p phlow-cli`
  (54 green), `-p phlow-gauntlet` (lib 338 green; bin has one
  pre-existing root-environment failure also present on base).
- Wire probes: `~/workspace/phlow-c2-20261028/probe.py` against
  baseline and migrated `phlow serve` binaries; captures in
  `baseline-wire-*.txt` / `migrated-wire-*.txt` there.
- Primo gates + Diver gate + rose smoke: GATE-REPORT-C2.md.
