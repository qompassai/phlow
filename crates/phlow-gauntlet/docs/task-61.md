# task-61: plan schema validation

**Kind:** nvim-lua · **Status:** fail (open) · **Wave:** 61–65 · **Commits:** pending (wave 61-65)

## ELI5

Before a robot runs a plan, someone should check the plan makes sense: every step exists, every tool it names is real, no step waits on itself in a circle. This task asks: does diver check plans like that *before* anything executes? The answer is no — because diver has no machine-readable plan at all. The "planner" writes plan *text* for humans to read, and the thing that runs work (the harness) runs *runs*, not plans. You can't validate a plan against a schema when there is no plan type for the schema to describe.

## What this task attempts

- **Goal:** find the planner → executor handoff and verify the plan is schema-validated before execution.
- **Mechanism:** `lua/gauntlet/task_61.lua` scans the real `lua/ai` tree — harness export tables plus a bounded source-text scan — for `validate_plan` / `plan_schema` / `plan_validator` / `schema_violation`; drives `ai.harness.registry.register_workflow` with a malformed plan shape; probes every plausible validator entry point.
- **Success criterion:** the design's structural bar — the executor's input type *is* the validated plan — or a sourced honest FAIL.
- **Non-goals:** inventing a plan schema; fixing diver on gauntlet authority.

## What happened

Honest FAIL at `where = "seam"`, first attempt. Zero plan-validation hits in 6 harness/rose modules' export tables and the bounded `lua/ai` text scan. The rose planner-role control sample (`ai.rose.plan`) was classified UNRELATED — it emits plan text for humans ("Output ONLY the plan as plain text", `lua/ai/rose/init.lua`), not a machine plan. Behaviorally: `register_workflow` ACCEPTED a def with a missing step, an unknown tool, cyclic deps, and a garbage field — only `adapter` is validated — and the def round-trips verbatim. Every validator entry point probed (`supervisor.validate_plan`, `harness.validate_plan`, `registry.validate_plan`, `registry.validate_workflow_plan`, `rose.validate_plan`) is nil.

## Full technical depth

The design's pass criterion is structural, not behavioral: "no executor ever sees an unvalidated plan (structural — the executor's input type *is* the validated plan); every rejection names the exact schema rule broken." Diver cannot meet it because the plan type doesn't exist:

1. **No plan representation.** `ai.rose`'s `M.plan(opts, callback)` runs a bounded agent run whose planner role returns `planner.text` — a string. The harness (`ai.harness.supervisor`) operates on *run records* (`M.new/create/start_run/finish/cancel/resume/retry_run`) — adapter-backed executions, not step plans with tools and dependencies.
2. **No plan schema.** The closest "plan" object is the registry's workflow def, and `register_workflow` validates only `def.adapter` (non-empty string) plus the name pattern. The probe's malformed def (steps `{{id=1},{id=3}}`, tools `{'nonexistent_tool'}`, deps `{[1]=3,[3]=1}`, `garbage_field=true`) registered fine and round-tripped with the garbage field intact — the registry stores defs, it doesn't validate plans.
3. **No validator.** Whole-word text scan for `validate_plan|plan_schema|plan_validator|schema_violation` across `lua/ai` (bounded: 256 files) returned zero; export-table scan of the six modules returned zero.

Fail-closed: if plan-validation machinery appears, the driver reports `where = "recon"` (premise changed). Diver-owned finding — flagged, never fixed on gauntlet authority. Whether diver *wants* a machine plan type with schema validation at the planner/executor seam is banked for Matt.

## Sources

- `~/workspace/repos/diver/lua/ai/harness/registry.lua` — `register_workflow` validates only `adapter`
- `~/workspace/repos/diver/lua/ai/rose/init.lua` — `M.plan` returns `planner.text` (plan text for humans)
- `~/workspace/repos/diver/lua/ai/harness/supervisor.lua` — run-record API, no plan input
- `~/workspace/gauntlet-design-tasks-21-70.md` — task-61 design (Wave 12)
