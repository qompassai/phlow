# task-64: deterministic tool selection

**Kind:** nvim-lua · **Status:** pass (with 2 banked caveats) · **Wave:** 61–65 · **Commits:** pending (wave 61-65)

## ELI5

When two tools both fit a job, the system must pick the same one every time — not flip a coin, not depend on the order things happened to load. Diver's one capability-based selection point (`ai.harness.adapter.negotiate`: "first adapter in sorted name order whose capabilities satisfy the need") does exactly that: it sorts names first, so Lua's unpredictable hash order can never leak into the choice. The driver proved it with 100 repeated ambiguous selections — same winner every time, even when the tools were registered in reverse order.

## What this task attempts

- **Goal:** verify tool selection is deterministic under ambiguity (the design's 100-run stability proof).
- **Mechanism:** `lua/gauntlet/task_64.lua` drives the REAL `ai.harness.adapter.negotiate` with mock adapters: single-match selection; ambiguity with reversed registration order; 100 runs with alternating insertion order; an intermittently failing probe.
- **Success criterion:** selection is a pure function of (needs, adapter set) — proven by the 100-run battery.
- **Non-goals:** intent-string matching (diver dispatches by exact name everywhere else — `rose/tools.lua M.call`, `mcp/tools.lua describe`, registry `get_tool` — so `negotiate` is the documented seam mapping, not an invention).

## What happened

PASS on the core dimension, first attempt:

- **single-match:** the only capability-satisfying adapter (`beta`) was selected.
- **ambiguity-sorted-name:** with `zeta` registered first and both satisfying, `alpha` won — the documented precedence rule ("Choose the first adapter (in sorted name order) whose probed capabilities satisfy every requested need") beat registration order.
- **hundred-run-stability:** 100/100 runs chose `alpha` across alternating insertion orders — no hash-order or timing dependence. Determinism holds by mechanism (`table.sort` before first-match) and by measurement.
- **probe-flap-boundary:** an adapter whose probe fails every other call alternates between winning and being skipped. `negotiate` treats probe failure as "unavailable" and skips — never raises. This is an INPUT change (probe outcomes are inputs to the pure function), not nondeterminism; the probe contract ("probe failures become unavailable capabilities") makes it explicit.

**Banked caveats (diver-owned, flagged, never fixed on gauntlet authority):**

- **C1:** `negotiate` returns only the adapter — no rationale record is produced or logged per selection. The rule is *documented* (docstring + registry header: "listed in sorted order so behavior is deterministic and auditable"), not *logged*. The design's "every ambiguous selection logs which rule fired" is unmet.
- **C2:** precedence is hardcoded sorted-name order; there is no precedence config. Selection is a pure function of (needs, adapter set, probe outcomes) — fewer degrees of freedom, still pure.

## Full technical depth

`M.negotiate(adapters, needs)`: collects names via `pairs`, `table.sort(names)`, then probes each in order and returns the first whose probed capabilities satisfy every `needs[key] == true` over `types.CAPABILITY_KEYS` (`streaming`, `cancellation`, `resume`, `permissions`, `artifacts`, `remote`, `tools`). Probe failures are swallowed to "unavailable" (`else _ = err`). The real supervisor uses it at launch: `adapter.negotiate(adapters, { cancellation = true })` (`supervisor.lua`). Mock adapters honor the `AiHarnessAdapter` contract (`name`, `probe`, `start`, `cancel`, `close`; probe returns all 7 capability keys as booleans) so `M.validate`/`M.probe` exercise the real path. The 100-run battery alternates insertion order per iteration to defeat any accidental order dependence; the flap adapter raises inside `probe` on even calls (caught by `negotiate`'s `pcall`).

The "explained" half of the design's dimension is met documentarily but not observably — that's C1. Whether diver wants per-selection rationale logging and a precedence config is banked for Matt.

## Sources

- `~/workspace/repos/diver/lua/ai/harness/adapter.lua` — `M.negotiate`, `M.probe`, `M.validate`
- `~/workspace/repos/diver/lua/ai/harness/types.lua` — `CAPABILITY_KEYS`
- `~/workspace/repos/diver/lua/ai/harness/supervisor.lua` — real `negotiate` call site at launch
- `~/workspace/repos/diver/lua/ai/harness/registry.lua` — "listed in sorted order so behavior is deterministic and auditable"
- `~/workspace/gauntlet-design-tasks-21-70.md` — task-64 design (Wave 12)
