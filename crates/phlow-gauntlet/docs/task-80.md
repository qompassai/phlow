# task-80: local model selection tradeoffs

**Kind:** nvim-lua (adversarial) · **Status:** fail (open) · **Wave:** 76–80 · **Commits:** pending (wave 76-80)

## ELI5

Model selection is "picking which local model should handle this tool call, weighing speed against quality." The design wants a selector that reads model cards and measured benchmarks, trades quality for latency and memory explicitly, and records its rationale. Diver has a selector — but it solves a different problem. `ai.harness.adapter.negotiate(adapters, needs)` picks the FIRST adapter in sorted name order whose probed boolean capabilities cover every requested need. The vocabulary is seven booleans (streaming, cancellation, resume, permissions, artifacts, remote, tools). There is no model registry, no model card, no latency/memory/quality signal, no benchmark, no rationale — the function returns the adapter table, full stop. Driver probes against the real module prove it: five negotiate() calls all select the first-sorted satisfying adapter, even when driver-side metadata rates the loser higher quality; a source scan of the real module finds zero tradeoff vocabulary.

## What this task attempts

- **Goal:** verify per-tool local model selection on latency/memory/quality tradeoffs with model cards, measured benchmarks, and explicit rationale — or document the gap.
- **Mechanism:** `lua/gauntlet/task_80.lua` (driven by `src/tasks/task_80.rs`) plays the harness against the REAL adapter module: selection (3 adapters, distinct boolean profiles + driver-side quality notes the module never reads; 5 negotiate() probes asserting first-sorted-wins), modelscan (adapter.lua path resolved via debug.getinfo — never hardcoded — plus sibling types.lua, scanned for tradeoff vocabulary). Machine-readable traces land in the work dir for the harness probes: capability_needs_boolean_only (every needs value and every probed capability is boolean; no tradeoff fields) and no_tradeoff_record (zero tradeoff hits; the only `model` hits are telemetry event-kind strings; selections record names only).
- **Success criterion:** a model selector with tradeoff inputs and rationale, or the gap documented as the finding (flagged Diver-owned, never fixed on gauntlet authority).
- **Non-goals:** touching diver on gauntlet authority (diver-owned finding: flagged, never fixed).

## What happened

Honest FAIL at `where = "seam"`, first attempt — the selector is real but solves a different problem:

- **V1:** the selection strategy is real and mechanical — all 5 probes select the first name in sorted order among satisfying adapters; `zeta-slow` (higher driver-side quality note) loses every tie to `alpha-fast`: quality is not an input.
- **V2:** the model-selection scan completes — adapter.lua resolved via debug.getinfo, tradeoff-vocabulary hits: 0; `model` hits are only `model.requested` / `model.stream_delta` / `model.completed` telemetry event-kind strings.
- **A1:** the contract is boolean-only — all 5 probes' needs tables hold only booleans; all 3 adapters expose only the 7 boolean capability keys; no latency/memory/quality/benchmark fields anywhere.
- **A2:** no tradeoff record exists — selections record adapter names only, no rationale, no quality, no latency, no memory per decision.

## Full technical depth

The seam is `~/workspace/repos/diver/lua/ai/harness/adapter.lua::M.negotiate(adapters, needs)`: names sorted, each adapter probed via `M.probe` (which enforces all `CAPABILITY_KEYS` are booleans), first satisfying returned, else `nil, 'no adapter satisfies the requested capabilities'`. `max_input_bytes` is an optional integer capability the negotiation never reads. The driver's allowlist probe demonstrates the harness has no allow concept — the caller filters the adapters table before negotiating. The docstring itself documents the strategy ("Choose the first adapter (in sorted name order)…"), so the behavior is specified, not accidental — the finding is that the specified behavior is boolean capability coverage, and the design's model selector (latency/memory/quality tradeoffs with explicit rationale) is a different component that does not exist. Whether diver should gain one is Matt's call.

Diver-owned finding: flagged, never fixed on gauntlet authority. The gauntlet documents the gap and stops.

## Sources

- `~/workspace/repos/diver/lua/ai/harness/adapter.lua` — `M.negotiate`, `M.probe`, `M.validate`
- `~/workspace/repos/diver/lua/ai/harness/types.lua` — `CAPABILITY_KEYS` (7 booleans)
- `~/workspace/gauntlet-design-tasks-71-100.md` — task-80 design (Wave 76–80)
