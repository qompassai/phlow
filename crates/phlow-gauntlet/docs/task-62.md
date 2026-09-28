# task-62: replanning on partial failure

**Kind:** nvim-lua · **Status:** fail (open) · **Wave:** 61–65 · **Commits:** pending (wave 61-65)

## ELI5

A plan has 5 steps. Step 2 fails. A smart system doesn't restart from step 1 and doesn't blindly continue — it writes a *new* plan for steps 3–5 that works around the failure, and if the failure poisoned step 1's result, it smartly re-does step 1 too. This task asks: does diver have that "replan" ability? The answer is no. Diver can *resume* (re-run the same thing) and *retry* (try the same thing again, with backoff), but it cannot write a new plan for the remainder. The design explicitly anticipated this: "if the harness only supports resume, document the gap."

## What this task attempts

- **Goal:** locate the planner's replan entry point and verify semantic replanning on partial failure.
- **Mechanism:** `lua/gauntlet/task_62.lua` scans harness export tables + a bounded whole-word source-text scan of `lua/ai/harness` for `replan`/`revise_plan`/`new_plan`/`plan_v2`; extracts the real bodies of `M.resume` and `M.retry_run` from `supervisor.lua` and asserts what they do.
- **Success criterion:** a replan entry point emitting plan v2 for the remainder, or a sourced honest FAIL documenting the resume-only gap.
- **Non-goals:** inventing a replanner; fixing diver on gauntlet authority. Distinct from task-05 (resume *continues* the same plan) and task-21 (saga compensates *backward*).

## What happened

Honest FAIL at `where = "seam"`, first attempt. Zero replan hits in 5 modules' export tables and the whole-word `replan` scan of `lua/ai/harness`. The body reads prove the point:

- `M.resume`: rejects unknown run ids, requires a terminal run, then `launch(sup, run, run.adapter)` — the SAME run table, SAME spec, SAME adapter. Body contains no plan token.
- `M.retry_run`: enforces "retry attempt ceiling exceeded", transitions the SAME run to `retry_wait` with exponential backoff + jitter; `tick()` promotes when due. Body contains no plan token.

Neither can emit "plan v2 covering steps 3–5 with a workaround"; neither is semantic about which completed steps stay valid (a step-2 failure invalidating step 1's result cannot trigger a re-do of step 1). The rose planner→coder→reviewer flow has role phases, not step plans 1..5, and coder retry appends feedback to the same task.

## Full technical depth

The design's scenarios need a step-addressable plan with a replan entry point:

1. **Default** (all steps succeed — no replan): vacuous without steps.
2. **Step 2 fails → plan v2 for steps 3–5, steps 1–2 not re-executed:** impossible — `retry_run` re-queues the *same* run (the failed step retried as-is); `resume` re-launches the *same* run_id from scratch (worse: it re-executes everything, the opposite of "never blindly re-executed"… though honestly resume requires a *terminal* run, so it re-runs a finished run — still the same spec, not a remainder plan).
3. **Adversarial (step-2 failure invalidates step 1 → re-do step 1):** impossible — nothing in the recovery path evaluates which completed steps stay valid; the bodies contain no plan token at all.

The whole-word token check (`%f[%a]replan%f[%A]`, lowercased) avoids false positives like "explain". Fail-closed: replan machinery appearing flips the verdict to `where = "recon"`. Diver-owned finding — flagged, never fixed on gauntlet authority. Whether diver wants a semantic replan entry point (vs resume/retry) is banked for Matt.

## Sources

- `~/workspace/repos/diver/lua/ai/harness/supervisor.lua` — `M.resume` (re-launch same run), `M.retry_run` (bounded same-run retry)
- `~/workspace/repos/diver/lua/ai/rose/agent.lua` — planner/coder/reviewer role flow, feedback-appended coder retry
- `~/workspace/gauntlet-design-tasks-21-70.md` — task-62 design (Wave 12)
