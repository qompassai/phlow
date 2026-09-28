# Learning doc template — copy to `task-NN.md` and fill in

> Every section is required. ELI5 first, then full depth. Cite primary
> sources for every protocol/API claim. Document failures with evidence,
> not adjectives.

# task-18: worker crash recovery

**Kind:** nvim-lua · **Status:** fail → fixed · **Wave:** 4b · **Commits:** pending (wave 4b)

## ELI5

Imagine a kitchen where a head chef (the supervisor) hands dishes to cooks
(workers). Sometimes a cook faints mid-dish — that's a crash. The head chef
has a rule: wake the cook up and let them try the dish again, but at most a
fixed number of total tries. After the last allowed try, the chef stops,
declares the dish failed, and writes down *everything* that went wrong — the
fainting, the burn, the dropped pan — in one readable list so the next person
knows exactly why. "Correct" here means: transient faints get retried and the
dish finishes; a cook who keeps fainting gets exactly the allowed number of
tries and then a clear, unambiguous FAILED verdict with the full cause list —
never silently dropped, never retried forever.

## What this task attempts

- **Goal:** drive diver's real worker-supervisor retry path through repeated
  crashes and prove the retry ceiling terminates with an unambiguous failed
  verdict and a legible cause chain.
- **Mechanism:** the real `ai.harness.supervisor` module from the diver repo
  (`lua/ai/harness/supervisor.lua`), executed under headless Neovim 0.13;
  the gauntlet Lua driver (`crates/phlow-gauntlet/lua/gauntlet/task_18.lua`)
  injects crash diagnostics and calls the real `M.retry_run`, `M.tick`, and
  `M.finish`.
- **Success criterion:** all 4 scenarios pass and the CLI golden output shows
  `verdict=failed`, `attempts=4/4`, the four crash causes (SIGSEGV, SIGKILL/OOM,
  heartbeat loss, adapter-init failure), the retry-ceiling cause, and the
  verdict is not readable as `timed_out` or `cancelled`.
- **Non-goals:** real process crashes (no OS signals are sent — diagnostics
  are injected), wall-clock backoff timing (a simulated clock drives `tick`),
  and multi-worker scheduling.

## What happened

Passed after one fix iteration. Final evidence from headless Neovim runs
(`NVIM v0.13.0-dev-1721+g7dbca1c4e2`): `persistent-crash` pass,
`transient-crash` pass, `duplicate-crash` pass, `crash-storm` pass, and an
unknown scenario name fails explicitly instead of silently doing nothing.
The CLI report prints `verdict=failed`, `attempts=4/4`, and a cause chain
listing SIGSEGV, SIGKILL/OOM killer, heartbeat loss, adapter init failure,
and the retry-ceiling refusal — with the verdict string distinct from
`timed_out` and `cancelled`.

## Where it went wrong

- **Stage:** first driver run, `persistent-crash` scenario.
- **Symptom:** the driver hit `'retry attempt ceiling exceeded'` one crash
  earlier than it expected — it had modeled the bound as "3 retries" and
  asked for a retry the real code was already refusing.
- **Evidence:** the refusal string comes from the supervisor itself, and the
  bound is a small constant:
  `lua/ai/harness/supervisor.lua:20` → `local RETRY_ATTEMPTS_MAX = 4`;
  `lua/ai/harness/supervisor.lua:346-347` →
  `if run.attempt >= RETRY_ATTEMPTS_MAX then return nil, 'retry attempt ceiling exceeded' end`.
- **Root cause:** the ceiling counts *total attempts* (4), not *retries*.
  Attempt 1 is the first try; crashes on attempts 1–3 are retried (3
  successful retries); the retry requested after the 4th crash is refused
  because `run.attempt` (4) has reached the ceiling. My driver had assumed
  the 3rd crash would be the last retryable one — an off-by-one between
  "retries allowed" and "attempts allowed", verified against the two cited
  lines, not guessed.

## The fix — what changed and why

- **Changed:** `crates/phlow-gauntlet/lua/gauntlet/task_18.lua` — the
  `persistent-crash` loop now crashes on attempts 1–3, accepts the three
  retries, asserts the 4th retry request is refused with the ceiling error,
  and then explicitly finishes the run as failed with the accumulated cause
  chain.
- **Commit:** pending (wave 4b).
- **Why:** the driver must mirror the real counting or it tests a fantasy.
  Aligning to total-attempts means the test exercises the exact refusal the
  supervisor performs in production. The alternative — changing the
  supervisor's constant or check — was rejected: the task tests the real
  supervisor, and diver owns that file.
- **Source:** `lua/ai/harness/supervisor.lua:20` (ceiling = 4 total attempts),
  `:340-347` (`M.retry_run` refusal).
- **Validation agents:** headless Neovim scenario runs — `persistent-crash`
  now shows attempts `4/4` with the ceiling cause in the chain;
  `transient-crash` shows a crash on attempt 1 followed by success on the
  retry, proving retries actually recover.
- **Adversarial agents:** `duplicate-crash` — a second crash report for the
  same attempt spends no retry budget (idempotent diagnostics);
  `crash-storm` — a burst of rapid crashes cannot push attempts past 4 and
  the terminal `run.finished` event fires exactly once (the supervisor's own
  contract at `supervisor.lua:77`: "Terminal states emit run.finished
  exactly once"). Both passed; neither found a way to escape the bound.
- **Citations:** ceiling and refusal — `lua/ai/harness/supervisor.lua:20`,
  `:340-347`; exactly-once terminal event — `:77`, `:103`; tick-driven
  retry scheduling — `:468`.

## Full technical depth

The supervisor owns a run record with an `attempt` counter starting at 1.
`M.retry_run(sup, run_id, reason)` (supervisor.lua:340) first checks
`run.attempt >= RETRY_ATTEMPTS_MAX` (4) and refuses with
`'retry attempt ceiling exceeded'` — this is the only retry budget in the
system; there is no separate per-reason budget. On success it schedules the
next attempt via the backoff path driven by `M.tick(sup, now_ns)` (:468),
which the driver advances with a simulated clock so no wall-clock waiting
is needed.

Two findings matter for anyone building on this:

1. **The supervisor does not finish the run at the ceiling.** `retry_run`
   only *refuses* — the run sits in a crashed-but-unfinished state until
   something calls `M.finish(run, 'failed', cause_chain)`. In production
   that something is the monitor loop; in this task it is the driver. A
   monitor that only calls `retry_run` and never finishes on refusal would
   leak crashed runs forever. The driver therefore treats "retry refused"
   as the signal to finish failed, and the finished event carries the whole
   cause chain (SIGSEGV → SIGKILL/OOM → heartbeat loss → adapter-init
   failure → ceiling refusal), which is what makes the CLI verdict
   unambiguous.
2. **Terminal events are exactly-once by contract** (supervisor.lua:77,
   :103). The `crash-storm` scenario leans on this: even when crash
   diagnostics arrive faster than the retry loop can process them, the
   attempt counter cannot pass 4 and `run.finished` fires once. Duplicate
   crash reports for an already-counted attempt are absorbed without
   spending budget — diagnostics are idempotent, retries are not free.

The CLI golden test asserts the operator-facing shape: `verdict=failed`
as a literal string (not `timed_out`, not `cancelled`), `attempts=4/4`,
and every cause visible in order. That string-level check is the whole
point: an operator triaging at 3am must not have to infer the verdict.

## Sources

- Primary: `~/workspace/repos/diver/lua/ai/harness/supervisor.lua`
  - `:20` — `RETRY_ATTEMPTS_MAX = 4` (total attempts, not retries)
  - `:77` — "Terminal states emit run.finished exactly once"
  - `:103` — `M.finish` emits the terminal `run.finished` event
  - `:340-347` — `M.retry_run`: ceiling check and refusal string
  - `:435-460`, `:468` — `M.tick`: retry scheduling over the clock
- Task code: `crates/phlow-gauntlet/lua/gauntlet/task_18.lua`,
  `crates/phlow-gauntlet/src/tasks/task_18.rs`,
  `crates/phlow-gauntlet/tests/task_18.rs`
