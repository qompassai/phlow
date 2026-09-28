# task-71: delegation depth attribution

**Kind:** nvim-lua (adversarial) · **Status:** fail (open) · **Wave:** 71–75 · **Commits:** pending (wave 71-75)

## ELI5

Depth attribution is "who spawned whom, how deep." When an agent delegates to a subagent, which delegates to another, the system needs to know how deep that chain goes — because a chain with no bottom is a fork bomb with extra steps. The design demands that the supervisor itself computes depth from its own records (it owns the tree of who spawned whom) and enforces a depth limit at spawn time with a named error when the limit is hit. Diver's supervisor genuinely owns the tree: every run records `parent_id` (who spawned it) and `root_id` (the top of the tree), and `spawn_child` sets the parent correctly. But it never *reads* the tree back: no depth is ever computed, no limit is named, no typed error exists. The data for attribution exists; the attribution itself doesn't. A child can also lie about its depth in its extensions — the lie passes through unread — and `parent_id` can name any existing run at all, not just the actual spawner. The design explicitly allows this outcome: "the gap is documented as the finding."

## What this task attempts

- **Goal:** verify depth of the LIVE spawn tree is attributed by the supervisor from its own run ancestry (never self-reported by the child), with a named bound and a typed rejection at spawn time (or document the gap).
- **Mechanism:** `lua/gauntlet/task_71.lua` drives the REAL `ai.harness.supervisor` in headless Neovim with a mock sink and mock registry (runs are never started): chain-depth (parent→child→grandchild walks to depths 0/1/2 from the supervisor's own parent_id tree), depth-unbounded (30-deep chain), forged-depth (child forges `extensions.claimed_depth = 0`; spawn names a FOREIGN parent_id), no-depth-error (source scan + chain to the total-run cap).
- **Success criterion:** supervisor-computed depth on every run + a named depth bound enforced at spawn with a typed error — or the gap documented as the finding.
- **Non-goals:** inventing a depth policy for diver on gauntlet authority; starting real runs.

## What happened

Honest FAIL at `where = "seam"`, first attempt — the gap IS the finding, exactly as the design allows:

- **V1:** the parent→child→grandchild chain walks to depths 0/1/2 from the supervisor's own parent_id tree — the DATA for supervisor-side attribution exists — but no run carries a supervisor-computed depth field, so the data is never attributed.
- **V2:** a 30-deep linear chain spawns with zero resistance; the only refusal is the total-run cap (`runs_max = 256`, 'supervisor run bound exceeded'). No `delegation_depth_exceeded` error exists.
- **A1:** the attribution-integrity attack succeeds twice over — the child forges `extensions.claimed_depth = 0` (the lie passes through unread; true ancestry depth is 2), and a spawn naming a FOREIGN parent_id (an unrelated live run) is accepted silently. Attribution is caller-asserted, never supervisor-verified.
- **A2:** the spawn path contains no depth machinery at all: zero "depth" mentions in the loaded supervisor.lua; a chain to the cap is refused only with the total-run error. No rejection ever names an ancestry chain.

## Full technical depth

The tree is real mechanism: `M.create` sets `run.parent_id = spec.parent_id` and `run.root_id` (rooting through the parent), `M.spawn_child` asserts the parent is non-terminal and delegates to `create`, and `parent.children` is maintained. A bounded O(depth) walk over `parent_id` links would give exact depths — the gauntlet runs that walk externally and validates it (0/1/2 on the chain, 0..29 on the deep chain). The supervisor never runs it: `validate_run_spec` requires `spec.goal` non-empty but neither it nor `create` touches depth; `spawn_child` checks only that the asserted parent exists and is non-terminal. `extensions` is passed through verbatim, so a forged `claimed_depth` is stored alongside the run as if it meant something. The design's "the supervisor recomputes depth from its own run tree and rejects the lie" has no implementation anywhere in the spawn path — source-verified.

Diver-owned (flagged, never fixed on gauntlet authority): depth attribution needs (a) a supervisor-computed depth on every run, walked from the run's own parent_id chain, (b) a named depth bound enforced in `spawn_child`, (c) a typed `delegation_depth_exceeded` rejection, and (d) verification that the asserted parent_id is the actual spawner — otherwise the tree the supervisor keeps is write-only bookkeeping.

## Sources

- `~/workspace/repos/diver/lua/ai/harness/supervisor.lua` — `M.create`, `M.spawn_child`, `validate_run_spec` (zero "depth" mentions)
- `~/workspace/gauntlet-design-tasks-71-100.md` — task-71 design (Wave 71–75)
