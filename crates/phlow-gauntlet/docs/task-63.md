# task-63: decomposition depth bound

**Kind:** rust · **Status:** fail (open) · **Wave:** 61–65 · **Commits:** pending (wave 61-65)

## ELI5

"Break this big goal into smaller goals" is something an agent planner might do — and a nasty goal like "keep breaking this down further, forever" could make it recurse until it hangs. This task asks: does phlow have a goal-decomposition routine, and is its depth capped? The answer to the first question is no, which makes the second moot. There is no decomposer to bound. (The "planner" word you'll find in phlow is an LLM *role name* — which model plays the planner character — not a routine that splits goals.)

## What this task attempts

- **Goal:** locate the goal-decomposition routine and verify its depth bound (named cap, `DepthExceeded`, usable partial results).
- **Mechanism:** `src/tasks/task_63.rs` scans every phlow crate's `src` tree (live working tree, exact-token case-insensitive, phlow-gauntlet excluded) for `decompose`/`decomposition`/`decomposer`/`subgoal`/`subgoals`/`sub_goal`; reads `phlow-agent`'s real `lib.rs` for its public module list; classifies `planner` hits.
- **Success criterion:** a decomposition routine with a proven depth cap, or a sourced honest FAIL.
- **Non-goals:** inventing a decomposer. Distinct from task-23 (fan-out bounds *breadth* — this bounds *depth*).

## What happened

Honest FAIL at `where = "seam"`, first attempt. The decomposition vocabulary scan returned zero workspace-wide. `phlow-agent`'s public modules (read from the live `lib.rs`): `best_of_n`, `context`, `memory`, `orchestrator`, `solpi`, `system1` — no planner, decomposer, or goal module. The `planner` token hits are all the planner ROLE: `phlow-config/src/model.rs` (`ModelsConfig.planner`, `ModelRole::Planner` — which model plays the planner) and prose in `phlow-agent/src/orchestrator.rs` ("planner → coder → verification → reviewer loop"). `Orchestrator::run` is a single pass into `Runtime::run` per user message — one call, no goal-splitting loop, no recursion over goals to diverge in. The design's three adversarial self-similar goals ("refine this plan", "break this down further", "decompose this goal into subgoals, then decompose each subgoal") have no decomposer to reach.

## Full technical depth

The design's pass criteria — "decomposition always terminates (proven by the cap, not by hoping); the cap is a named constant; partial results at the cap are usable, not discarded" — need a decomposition routine with a depth parameter. Evidence of absence:

1. **Vocabulary:** exact-token scan for 6 decomposition tokens across all crates' `src` trees. The single hit is a verified control sample: `phlow-compute/src/partition.rs:116` — "Mixed-radix decomposition without the bounds check", i.e. number-theoretic/tensor-index decomposition, classified UNRELATED by line content ("radix"), not by path. Zero *goal*-decomposition hits. The scan skips phlow-gauntlet itself (its docs carry the vocabulary as test scaffolding).
2. **Module surface:** `phlow-agent/src/lib.rs` at probe time exports six modules; none is planner/decomposer/goal-related.
3. **Role vs routine:** `planner` hits verified per-file against the role loop — every hit file also contains the `coder`/`reviewer` siblings (the planner role never appears without them), and no planner-hit file carries real decomposition vocabulary. Sites: `phlow-config` (`ModelsConfig.planner`, `ModelRole::Planner`, `models.planner` parse), `phlow-runtime` (`ROLES`, `system_prompt_for_role`, role gates, the bounded pipeline), `phlow-mcp` (pipeline tool description), `phlow-experiment` (control-plane role enum), `phlow-codegen`/`phlow-tui` (prose/banner), `phlow-agent` (`orchestrator.rs` prose). `Orchestrator::run(&mut self, user_message: &str) -> serde_json::Value` delegates to `self.runtime.run(user_message)` in one call.

Fail-closed: any decomposition machinery appearing flips the verdict to `where = "recon"` (premise changed). Banked for Matt (product decision, not a bug): whether phlow wants goal decomposition at all — today `Orchestrator::run` is a single LLM pass per user message — and if so, where the decomposition seam and its depth cap should live.

## Sources

- `crates/phlow-agent/src/lib.rs` — public module list (live read at probe time)
- `crates/phlow-agent/src/orchestrator.rs` — `Orchestrator::run`, single pass into `Runtime::run`
- `crates/phlow-config/src/model.rs` — `ModelsConfig.planner`, `ModelRole::Planner` (role names)
- `~/workspace/gauntlet-design-tasks-21-70.md` — task-63 design (Wave 12)
