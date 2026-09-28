# task-52: straggler mitigation

**Kind:** nvim-lua · **Status:** fail (open) · **Wave:** 51–55 · **Commits:** pending (wave 51-55)

## ELI5

If one worker is just slow (not dead), you do not want the whole job
to wait for it: after a timeout, launch a second copy and take
whichever finishes first — "speculative execution". The guarantee is
that only ONE result commits ("single commit"): if the slow worker
and its duplicate both finish, the second result is discarded, never
applied twice.

## What this task attempts

- **Goal:** prove diver's harness mitigates stragglers by speculating
  duplicates past a timeout — the design names the worker
  orchestration seam — driving the REAL supervisor with a
  driver-controlled-latency mock adapter.
- **Mechanism:** `lua/gauntlet/task_52.lua`, a headless-Neovim
  behavioral probe: it requires the real `ai.harness.supervisor`,
  installs a mock adapter whose worker completion time is set by the
  driver, and steps the simulated clock to observe whether the slow
  worker ever gets a speculative duplicate.
- **Success criterion:** the design's speculation pass criteria
  (duplicate launched at the speculation timeout; fast path
  completes; exactly one result commits; generation counter
  distinguishes the attempts).
- **Non-goals:** inventing speculation. The design says an evidenced
  `where = "seam"` failure is the correct result when the design says
  an absent seam is valid.

## What happened

Fail, open seam — behavioral, not just recon. The probe drives the
real supervisor: 2 fast workers complete, the slow worker stays
pending while the clock steps 10x past the fast path, and the
supervisor never launches a second attempt — it has no speculation
timeout and no hedging API. The probe:

- `task_reports_seam_absence` (V): task-level verdict is `fail` at
  `"seam"` — "no speculative execution"; the evidence aggregates all
  four probe scenarios.
- `straggler_gets_no_speculative_duplicate` (V): the straggler probe
  passes (the probe characterizes correctly): exactly 1
  `run.started` after 10x the fast path; p99 is bounded by the slow
  worker, not by a speculation timeout.
- `single_commit_holds_and_double_finish_is_refused` (A): after the
  straggler completes there is exactly 1 `run.finished`, and a second
  finish is refused as an invalid transition — single commit holds
  vacuously, since only one attempt ever launches.
- `no_speculation_api_on_the_supervisor` (A): the export-table scan
  names the needles (speculat/hedge/duplicate/backup/straggler/timeout)
  and lists the real supervisor exports; zero hits. The fail-closed
  arm reports `where = "recon"` if one ever appears.

Diver-owned finding (flagged, never fixed on gauntlet authority): the
absence is in the diver repo's supervisor. The supervisor does have
retry backoff with jitter for crashed runs (task-19 finding) — but
that is recovery after failure, not speculation on latency. Whether
diver wants speculative execution is a diver product decision, banked
for Matt — not gauntlet work.

## The fix — what changed and why

No fix — this is a documented design gap, never fixed under gauntlet
authority. The gauntlet-side work was making the absence check honest:

- **Changed:** `crates/phlow-gauntlet/lua/gauntlet/task_52.lua` (new)
  and `crates/phlow-gauntlet/src/tasks/task_52.rs` (new) — a
  behavioral probe against the real supervisor, not a vocabulary
  scan; distinguishes "seam absent" (observed behavior) from "probe
  crashed" and "premise changed".
- **Why:** straggler mitigation is a behavior, and a vocabulary scan
  cannot observe its absence honestly. The four-scenario probe shows
  what the supervisor actually does with a slow-but-alive worker.
- **Source:** diver `lua/ai/harness/supervisor.lua` (real run
  lifecycle: create/start/finish/retry_run; no speculation verbs).
- **Validation agents:** the 2 validation tests
  (`task_reports_seam_absence`,
  `straggler_gets_no_speculative_duplicate`) assert the task verdict
  and the no-duplicate observation.
- **Adversarial agents:** the 2 adversarial tests
  (`single_commit_holds_and_double_finish_is_refused`,
  `no_speculation_api_on_the_supervisor`) assert single commit on the
  real state and the zero-API-hit export scan.

## Full technical depth

The mock adapter takes a table of per-worker latencies; `tick`
advances the simulated clock and completes any worker whose deadline
passed. The straggler scenario: workers A and B fast (finish by
t=2s), worker C slow (deadline t=200s). At t=20s — 10x the fast path
— exactly 1 `run.started` exists for C and no second attempt has been
created; the supervisor has no timeout that fires on latency. At
t=200s the single attempt completes and its result commits; a second
`finish` on the same run errors "invalid transition" (single commit
holds because nothing speculative ever existed). The supervisor's
export list (cancel, check_generation, consume, create, finish, get,
list, new, resume, retry_run, spawn_child, start_run, tick) carries
no speculative-execution verb, and `retry_run` is crash-recovery with
backoff, not hedging. The 90th-percentile-vs-p99 latency criterion
collapses to "p99 = the slow worker", which is exactly the failure
mode the design forbids: the tail dominates the job.

What is missing for the design's speculation: a speculation timeout
that launches a duplicate attempt for a slow-but-alive worker, a
generation counter distinguishing attempts, and single-commit logic
that discards the loser's result. The design gap: either diver's
supervisor grows hedging for tail latency or the current no-hedge
behavior is documented as the intended boundary.

## Sources

- Primary: diver `lua/ai/harness/supervisor.lua` (driven live with a
  mock latency-controlled adapter).
- Driver: `crates/phlow-gauntlet/lua/gauntlet/task_52.lua` (behavioral probe).
- Driver: `crates/phlow-gauntlet/src/tasks/task_52.rs` (nvim-lua runner).
- Tests: `crates/phlow-gauntlet/tests/task_52.rs` (2V/2A).
