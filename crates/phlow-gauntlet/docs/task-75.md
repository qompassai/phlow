# task-75: delegation cycle detection

**Kind:** nvim-lua (adversarial) · **Status:** fail (open) · **Wave:** 71–75 · **Commits:** pending (wave 71-75)

## ELI5

Cycle detection is "what stops a subagent from delegating back to its own boss." A depth limit alone can't catch it: two agents delegating to each other never get deep — they just spin forever. The design demands that every spawn checks the requester's ancestry: walk up the chain of who-spawned-whom and reject the spawn with a typed error naming the full cycle. Diver's spawn path performs no such check — the only "cycle" substring in supervisor.lua sits inside the word "lifecycle" in the header comment, and no `delegation_cycle` error exists anywhere. Strict graph cycles are structurally impossible there (a parent must already exist when you spawn, and every id is fresh — so no run can be its own ancestor), but that is *construction*, not a check: the property holds by accident of the id scheme, and the design's checkable cases sail through. The "escalation loop" — a child delegating back UP to its ancestor under a different task name — is accepted silently; detection by identity (not by name) does not exist. A spawn can even name any unrelated live run as parent_id — the ancestry set is never consulted at spawn. The gauntlet itself runs the O(depth) ancestry walk the design demands (iterative, capped, no recursion over attacker-controlled depth) and validates it on linear and branching trees under live Neovim — but the walk lives only in the gauntlet: the supervisor never runs it. The design explicitly allows this outcome: "the gap is documented as the finding."

## What this task attempts

- **Goal:** verify every spawn checks membership in the requester's ancestry set — rejections name the full cycle; the check is O(depth), documented, and detected by identity not by name — or document the gap.
- **Mechanism:** `lua/gauntlet/task_75.lua` drives the REAL `ai.harness.supervisor` in headless Neovim with a mock sink and mock registry (runs are never started): linear-chain (chain of 5; gauntlet's walk yields 0..4), walk-sound (branching tree of 6; every ancestry set exact), escalation-accepted (B, child of A, delegates back up to ancestor A under a different workflow name), no-cycle-machinery (source scan of the loaded supervisor.lua + foreign parent_id accepted).
- **Success criterion:** a per-spawn ancestry-membership check with a typed `delegation_cycle` rejection — or the gap documented as the finding.
- **Non-goals:** inventing the check for diver on gauntlet authority; creating real cycles (structurally impossible by construction).

## What happened

Honest FAIL at `where = "seam"`, first attempt — the gap IS the finding, exactly as the design allows:

- **V1:** a linear chain of 5 works; the gauntlet's O(depth) ancestry walk yields depths 0..4 with no false positives. The walk is sound — but it is the GAUNTLET's walk, not the supervisor's: no spawn ever checks membership in the requester's ancestry set.
- **V2:** the walk is validated on a branching tree (root, 2 children, 3 grandchildren): all 6 ancestry sets exact. The check the design demands is demonstrated correct under live nvim — and still never run by the supervisor.
- **A1:** the escalation loop is accepted: B (child of A) delegates back up to its ANCESTOR A under workflow "totally-different-workflow" — accepted silently. No `delegation_cycle` error, no ancestry-membership check, no identity-based detection.
- **A2:** the spawn path contains no cycle machinery at all — the only "cycle" substring is inside "lifecycle" (header comment), no `delegation_cycle` string — and the ancestry set is never consulted: a spawn naming an unrelated live run as parent_id is accepted. Strict cycles are structurally unrepresentable (fresh ids), but that is construction, not a check — and the design's upward/escalation cases ARE representable and unrejected.

## Full technical depth

The spawn path is `supervisor.spawn_child` → `supervisor.create` in `lua/ai/harness/supervisor.lua`. `spawn_child` sets `spec.parent_id` and delegates; `create` asserts the parent exists and is non-terminal, then mints a fresh id. Because ids are minted fresh and the parent must pre-exist, `parent_id` can never name a not-yet-created run — including the run being created — so a strict self-cycle or 2-cycle cannot be constructed. But nothing *checks*: `spawn_child` performs no ancestry walk, computes no ancestry set, and consults no membership relation. The caller-asserted `parent_id` is accepted on the two claims "exists" and "non-terminal" only — so upward delegation (child → ancestor) and cross-attachment (unrelated live run as parent) are both accepted, and identity-based cycle detection (the design's escalation loop, detected by run identity rather than task name) has no implementation. The gauntlet's walk is iterative and capped at 1024 (no recursion over attacker-controlled depth, per the design's O(depth) + no-livelock criteria) — it is the reference implementation the spawn path would need to run.

Diver-owned (flagged, never fixed on gauntlet authority): the spawn path needs (a) a per-spawn ancestry-membership check in `spawn_child`/`create`, (b) a typed `delegation_cycle` error naming the full cycle, (c) detection by run identity, not task name. The structural unrepresentability of strict cycles is a property worth keeping — but it does not cover escalation.

## Sources

- `~/workspace/repos/diver/lua/ai/harness/supervisor.lua` — `M.create`, `M.spawn_child` (the only "cycle" substring is inside "lifecycle")
- `~/workspace/gauntlet-design-tasks-71-100.md` — task-75 design (Wave 71–75)
