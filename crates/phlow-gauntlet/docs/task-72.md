# task-72: context handoff fidelity

**Kind:** nvim-lua (adversarial) · **Status:** fail (open) · **Wave:** 71–75 · **Commits:** pending (wave 71-75)

## ELI5

Context handoff is "what exactly crosses the line when a parent hands work to a child." The design wants a sealed envelope: the goal, the constraints, what's left of the budget, and the child's narrowed tool allowlist — with authority only ever narrowing downward. Diver has no envelope at all. A child run is just the spawn spec minus the goal (required non-empty by validation, then dropped from the run record — the boundary doesn't even carry the goal), plus a fresh full budget (never the parent's remainder — two siblings each get full budgets, so the parent's spending double-counts by construction), no tool allowlist on the run (policy allowlists are supervisor-global, untouched by spawn), and no size bound (a 10MB blob in `extensions` round-trips silently). Authority narrowing is unrepresentable at spawn: the spec has no allowlist field and `spawn_child` performs no narrowing step, so a widened child cannot be rejected. There is no handoff to be faithful to — every fidelity property fails at the seam. The design explicitly allows this outcome: "the gap is documented as the finding."

## What this task attempts

- **Goal:** verify the delegation boundary carries a handoff envelope (goal, constraints, budget remainder, narrowed tool allowlist) with monotonically narrowing authority — with oversized handoffs failing explicitly, not silently — or document the gap.
- **Mechanism:** `lua/gauntlet/task_72.lua` drives the REAL `ai.harness.supervisor`, `ai.harness.budget`, and `ai.harness.policy` in headless Neovim with a mock sink and mock registry (runs are never started): handoff-shape (enumerate the child run record's actual fields), budget-not-split (parent spends; children get fresh full defaults; two siblings), oversized-blob (10MB `extensions.blob`), no-allowlist (policy rule + spawn spec + child run inspected for allowlists).
- **Success criterion:** an envelope containing goal + constraints + budget remainder + narrowed allowlist; oversized handoffs rejected explicitly — or the gap documented as the finding.
- **Non-goals:** inventing a handoff envelope for diver on gauntlet authority; starting real runs.

## What happened

Honest FAIL at `where = "seam"`, first attempt — the gap IS the finding, exactly as the design allows:

- **V1:** the child run record's fields are `id, parent_id, root_id, workflow, adapter, workspace, budget, extensions, acceptance` — and `spec.goal`, though REQUIRED non-empty by `validate_run_spec`, is dropped from the run table. The boundary does not carry the goal.
- **V2:** the parent spends 900 of 1000 token units; a child spawned with no explicit budget gets full defaults (used=0) — `budget.new(spec.budget or DEFAULT_BUDGET_LIMITS)` — and two siblings each get full defaults. No shared remainder exists; the parent's budget double-spends by construction.
- **A1:** a 10MB `extensions.blob` is stored on the child run silently — all 10485760 bytes round-trip; no bound, no explicit error, no truncation.
- **A2:** allowlist narrowing is unrepresentable at spawn: the policy allowlist lives in supervisor-wide rules (`policy.lua` `rule.tools`), the spawn spec has no allowlist field, and `spawn_child` performs no narrowing step — nothing was narrowed for the child, and a widened child could not be rejected at spawn.

## Full technical depth

The spawn path is `supervisor.spawn_child` → `supervisor.create` in `lua/ai/harness/supervisor.lua`. `create()` builds the budget via `budget.new(spec.budget or DEFAULT_BUDGET_LIMITS)` — a fresh budget, never the parent's remainder; budget.lua's own split semantics are parent-owned and never consulted at spawn. `validate_run_spec` enforces `spec.goal` non-empty but `create()` does not store it on the run — validation without retention. Tool allowlists are global policy-rule data (`rule.tools` in policy.lua), enforced (where they are enforced) at the supervisor level, not per-run delegated authority — there is no `run.allowlist` field to narrow. `extensions` passes through without any size check, so an oversized context is stored. The design's "authority narrows monotonically across the boundary" presupposes a boundary object; the actual boundary is the spec table itself, and four of the five envelope fields have no carrier.

Diver-owned (flagged, never fixed on gauntlet authority): a handoff envelope needs (a) goal + constraints carried on the run record, (b) budget remainder sharing (or an explicit no-sharing contract), (c) a per-run allowlist with a spawn-time subset check, and (d) an explicit size bound on the spec — otherwise every fidelity property is untestable.

## Sources

- `~/workspace/repos/diver/lua/ai/harness/supervisor.lua` — `M.create`, `M.spawn_child`, `validate_run_spec`
- `~/workspace/repos/diver/lua/ai/harness/budget.lua` — `budget.new`, split semantics
- `~/workspace/repos/diver/lua/ai/harness/policy.lua` — global `rule.tools` allowlists
- `~/workspace/gauntlet-design-tasks-71-100.md` — task-72 design (Wave 71–75)
