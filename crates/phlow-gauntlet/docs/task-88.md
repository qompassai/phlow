# task-88: A2A error-shape propagation

**Kind:** rust (recon) · **Status:** fail at "seam" (open) · **Wave:** 86–90 · **Commits:** pending (wave 86-90)

## ELI5

The design asks: when the other side's AI agent hits an error, the error should arrive *typed* — which error code, what message, and what extra data — while the extra data is treated as untrusted (size-limited, quoted in logs, never interpreted as instructions). Unknown task states should be rejected as protocol violations, not silently ignored. And if the remote side claims something about *your* run, that claim should be recorded as *their* assertion — your own run's state stays authoritative.

The honest finding: there is no rust A2A client boundary at all. Bounded exact-token scans over every product crate's `src/**/*.rs` (the gauntlet crate itself excluded, because the harness's own probes legitimately use the design vocabulary) find zero hits for `a2a`, for task-state vocabulary, for agent-card handling, and for the A2A JSON-RPC methods. Remote error shapes cross no rust boundary, so there is nothing to preserve, bound, or quote. The only A2A boundary in the workspace is diver's Lua (`ai.a2a.client` / `ai.a2a.tasks`, exercised by task-07) — where the `bad-state` scenario showed unknown state strings are *ignored* by the task state machine rather than rejected as protocol violations (diver-owned, flagged).

## What this task attempts

- **Goal:** verify the A2A client boundary carries remote errors as typed errors (code/message/data preserved), treats attacker-controlled error `data` as untrusted, rejects unknown task states as protocol violations, and records remote claims about the local run as remote assertions — or document the seam's absence with bounded scans.
- **Mechanism:** `src/tasks/task_88.rs` runs four bounded source recons (the task-48 scan pattern: max 4000 files, 512 KiB per file, exact-token case-insensitive, gauntlet crate excluded): `no_a2a_client_boundary` (token `a2a` → 0 hits), `no_task_state_machine` (tokens `task_state`, `taskstate` → 0 hits), `no_agent_card_handling` (tokens `agent_card`, `agentcard` → 0 hits), `no_a2a_rpc_methods` (tokens `tasks/send`, `message/send`, `tasks/get`, `tasks/cancel` → 0 hits). Each case fails closed — premise changed — if a hit ever appears.
- **Success criterion:** the propagation discipline verified, or the seam's absence documented with scan evidence.
- **Non-goals:** inventing a rust A2A client on gauntlet authority (it is a product decision — banked for Matt, never implemented here).

## What happened

Honest FAIL at `where = "seam"`, first attempt — the seam is ABSENT in rust:

- **V1:** no A2A client boundary — zero `a2a` hits in any rust product crate.
- **V2:** no task-state machine — zero `task_state`/`taskstate` hits; unknown states have nothing to be rejected by.
- **A1:** no agent-card handling — a peer's card claims cross no rust boundary at all.
- **A2:** no A2A RPC methods — error responses to the A2A methods have no rust handler; attacker-controlled error `data` crosses no rust boundary.

## Full technical depth

The recon follows task-48's bounded-scan discipline: walk `crates/*/src/**/*.rs` from the workspace root (located via `CARGO_MANIFEST_DIR` + a `Cargo.lock` check), skip the gauntlet crate's own tree (the harness-probe principle: harness probes use design vocabulary; only product crates count — the task-82/task-48 regression that motivated it), tokenize on non-alphanumeric boundaries, and compare exact tokens case-insensitively. The task-level verdict reports the honest seam absence and banks the product decision: whether phlow should gain a rust A2A client boundary with typed error-shape propagation (code/message/data preserved; attacker data size-bounded, quoted, never interpolated; unknown states rejected; remote claims about the local run recorded as remote assertions only).

Distinct from task-07 (the Lua A2A boundary, which exists and was exercised there): this is the *rust* boundary, and there is none. The Lua-side observation — task-07's `bad-state` scenario showed the task state machine *ignores* unknown states — stands as the diver-owned adversarial note for this design point.

Product decision banked for Matt: whether phlow should gain a rust A2A client boundary with the full error-shape discipline. Not implemented on gauntlet authority.

## Sources

- `~/workspace/gauntlet-design-tasks-71-100.md` — task-88 design (Wave 86–90)
- `~/workspace/repos/diver/lua/ai/a2a/client.lua`, `~/workspace/repos/diver/lua/ai/a2a/tasks.lua` — the only A2A boundary in the workspace (Lua; task-07's `bad-state` scenario: unknown states ignored, not rejected)
