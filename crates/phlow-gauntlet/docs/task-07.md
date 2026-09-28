# task-07: A2A task lifecycle

**Kind:** nvim-lua · **Status:** fail (open) — 3 of 4 scenarios pass; the
peer-dies scenario fails and exposes a real harness adapter bug (every failed
A2A task is misreported as completed). A second real bug (dropped `run.goal`)
forces a documented workaround in the driver · **Wave:** not named in task
brief · **Commits:** none (rules forbid committing)

## ELI5

Diver's agent harness can talk to remote AI agents using the A2A protocol —
think of it as hiring a contractor over the phone: you send them a job
description (`message/send` or a live `message/stream` call), they report
progress (`working` → `completed`), you can cancel the job (`tasks/cancel`),
and you can ask how it's going (`tasks/get`). This task checks that the
whole phone call works end to end against a fake contractor (a mock peer
speaking real A2A JSON-RPC), including the nasty cases: the contractor hangs
up mid-call, or shouts a status word that isn't in the dictionary. Correct
means: the real adapter and protocol code run, the wire shapes match the
protocol, cancel reaches the peer, death is detected fast (no hanging up
forever), and garbage status words are ignored. The task found two real
bugs in diver's harness instead of a clean pass — and that is the point of
the gauntlet.

## What this task attempts

- **Goal:** drive diver's real A2A adapter and protocol modules through the
  full task lifecycle against a mock JSON-RPC peer, in four scenarios.
- **Mechanism:** `crates/phlow-gauntlet/src/tasks/task_07.rs`
  (`run`/`run_scenario`) spawns headless nvim on
  `lua/gauntlet/task_07.lua`, which drives diver's
  `lua/ai/harness/adapters/a2a.lua`, `lua/ai/a2a/{client,tasks}.lua`, and the
  harness supervisor against a bounded `vim.uv` TCP mock peer; the driver
  prints exactly one JSON verdict via `io.stdout:write` and always exits 0.
- **Success criterion:** `default` reaches completed with kebab-case states
  quoted on the wire and wire shapes verified; `cancel` posts `tasks/cancel`
  to the peer and settles the run cancelled; `peer-dies` fails the run fast
  with no hang; `bad-state` ignores an unknown state string and still
  completes.
- **Non-goals:** GRPC/HTTP+JSON bindings (diver speaks only the JSON-RPC
  binding); real network peers; multi-agent fanout; the `tasks/get` polling
  loop the brief requested but diver does not implement (see below).

## What happened

Iteration 3. Final state: **3 of 4 driver scenarios pass; `peer-dies`
fails with a documented root cause.** The Rust integration tests
(`cargo test -p phlow-gauntlet --test task_07`, 2 validation + 2
adversarial) are 4/4 green — the adversarial peer-dies test pins the
failure diagnosis so the bug cannot slip by silently.

- `default`: pass. The mock peer serves a card with `capabilities.streaming
  = true`, so the real `tasks.lua` chooses `message/stream`. Wire log shows
  `rpc method=message/stream id=1`, `wire-shape-ok role=user kind=message
  part-kind=text goal-match`, and SSE `status-update`/`artifact-update`
  events quoting kebab-case states `working -> completed`. The run settles
  completed; one `model.completed` with `outcome=completed`.
- `cancel`: pass. `ai.a2a.tasks.cancel` posts `tasks/cancel` to the peer
  (wire log: `rpc tasks/cancel id=2`), the local task settles `cancelled`,
  and the harness run reaches cancelled with no stale completion.
- `peer-dies`: **fail (open)** — see "Where it went wrong". The A2A layer
  detects the death correctly (curl exit 18 on the truncated stream, local
  task `failed` with `stream ended: curl: (18) ...`); the harness run
  nevertheless reports `completed` because of the adapter bug. No hang:
  detection is deadline-bounded.
- `bad-state`: pass. The peer injects the unknown state string
  `frobnicate` in a `status-update`; `tasks.set_state` ignores it
  (verified: the only `state` transitions observed are the legal ones) and
  the run completes uncorrupted.

Two real diver bugs were found (diver is read-only here, so both are
documented, not fixed):

1. `supervisor.create` validates `spec.goal` but never copies it into the
   run table, so `run.goal` is nil and every adapter that reads it breaks.
   The driver reproduces `harness.run()` internally and restores
   `run.goal = spec.goal`; the real adapter and protocol modules are
   unmodified.
2. `adapters/a2a.lua` registers `on_done = function(result, task_err)` but
   `ai.a2a.tasks` documents and calls `on_done(task)` — so `task_err` is
   always nil and every failed A2A task is recorded `completed`. This is
   what makes `peer-dies` fail.

## Where it went wrong

- **Stage:** verdict — `peer-dies` scenario, harness-run outcome.
- **Symptom:** the local A2A task reaches `failed` (curl exit 18 on the
  truncated stream), but the harness run reaches `completed` and the sink
  shows `model.completed: outcome=completed error=nil`. The driver fails
  with:
  `HARNESS ADAPTER BUG: local a2a task is failed but the harness run is
  completed; adapters/a2a.lua on_done(result, task_err) mismatches
  ai.a2a.tasks on_done(task) (tasks.lua:170), so task_err is always nil and
  failures are reported completed`
- **Evidence:**
  - Peer log: `rpc method=message/stream id=1`, `wire-shape-ok role=user
    kind=message part-kind=text goal-match`, `sse: working (truncated
    stream: peer dies in 300ms)`, `peer-dies: destroyed connection
    mid-stream`.
  - A standalone probe against the same peer code: curl exits 18
    (`transfer closed with 1048515 bytes remaining to read`); a manual curl
    against the driver's own peer also exits 18. The mock is faithful.
  - Driver evidence: `local a2a task state: failed`, `local a2a task error:
    stream ended: curl: (18) transfer closed with 1048478 bytes remaining to
    read`, then `sink model.completed: outcome=completed error=nil`.
  - `default`, `cancel`, and `bad-state` are unaffected: `default`/`bad-state`
    genuinely complete (so `task_err == nil` gives the right answer by
    accident), and `cancel` never fires the adapter's `on_done`.
- **Root cause:** contract mismatch, verified against primary sources.
  `lua/ai/a2a/tasks.lua:43` documents `---@field on_done? fun(task:
  A2aTask)` and `tasks.lua:170` calls `on_done(task)`; the in-repo
  `lua/ai/a2a/fanout.lua:84` uses the correct `on_done = function(task)`
  shape. But `lua/ai/harness/adapters/a2a.lua:63` registers `on_done =
  function(result, task_err)` and computes `outcome = task_err == nil and
  'completed' or 'failed'` — its `task_err` parameter is always nil, so the
  outcome is always `'completed'`. Any failed remote task is silently
  promoted to success at the harness level.

## The fix — what changed and why

No diver fix was applied (read-only repo). What changed in this task's own
four files, per iteration:

1. **Dropped-goal workaround** (`lua/gauntlet/task_07.lua`): first run died
   with `ai/a2a/tasks.lua:322: message must be nonempty`. Root cause:
   `lua/ai/harness/supervisor.lua:116` (`M.create`) validates
   `spec.goal` (`types.lua:229-230`: must be a non-empty string) but the run
   table it builds has no `goal` field — `run.goal` is nil, and
   `adapters/a2a.lua:61` passes it as `message = run.goal` into
   `tasks.submit`, whose assert (`tasks.lua:322`) rejects it. Same nil
   breaks `acp.lua:94`, `herd.lua:54`, `rose.lua:57`. The driver now
   reproduces `harness.run()` internally (`supervisor.create`, record the
   dropped goal as evidence, `run.goal = spec.goal`, `supervisor.start_run`)
   instead of calling the broken `harness.run`. **Source:** the diver
   sources cited above (primary). Alternatives rejected: monkey-patching
   `supervisor.create` would modify diver's behavior under test and
   invalidate the exercise.
2. **Peer `Content-Length` discipline** (`lua/gauntlet/task_07.lua`):
   iteration 1 sent `Content-Length: 0` on the truncated stream because two
   header-writing paths raced; curl saw a *complete* empty body and the
   client treated the death as a clean empty stream. Fixed to a single
   header path (`sse_begin_truncated`) emitting `Content-Length: 1048576`
   with only one event before the close, so the truncation is unambiguous
   on the wire (verified: curl exit 18 against the driver's own peer).
   **Source:** the curl-exit-18 probes (primary evidence).
3. **`vim.defer_fn` argument order** (`lua/gauntlet/task_07.lua`): the
   300 ms death timer was first written `(300, fn)`; Neovim 0.13 takes
   `(fn, delay_ms)`. The peer never died and the stream timed out instead.
   **Source:** `:h vim.defer_fn` in the installed nightly (primary).
4. **Peer-dies scenario reframed** (`lua/gauntlet/task_07.lua`,
   `tests/task_07.rs`): iteration 2 asserted the harness run reaches
   `failed` and went 3/4 with `peer-dies` red for the wrong reason (a
   phantom "completed" nobody could explain). Instrumentation (a temporary
   `vim.system` exit-code wrapper, since removed) proved curl really exits
   18 in the integrated path; the phantom came from bug 2 above. The
   scenario now asserts the A2A layer's failure (local task `failed` with
   the curl-18 error, deadline-bounded = no hang) and then fails with the
   exact adapter-bug diagnosis when the run misreports `completed`. The
   Rust test `peer_death_exposes_adapter_misreport` pins that diagnosis:
   it passes only if the verdict is a Fail at `peer-dies` whose evidence
   contains the local-task failure, the stream error, the misreported
   `outcome=completed`, and the `on_done` mechanism. When the adapter is
   fixed, this test goes red on purpose.
   **Source:** diver `tasks.lua:43,170`, `fanout.lua:84`,
   `adapters/a2a.lua:63-71` (primary).

**Proposed diver fixes (for the coordinator, not applied):**

```lua
-- supervisor.lua, in the run table literal:
goal = spec.goal,
```

```lua
-- adapters/a2a.lua, on_done:
on_done = function(task)
    if my_generation ~= handle.generation or handle.closed then
        return
    end
    sink:append(run.id, 'model.completed', {
        adapter = 'a2a',
        outcome = task.state == 'completed' and 'completed' or 'failed',
        error = task.error,
        result = { task_id = task.id, state = task.state },
    }, { source = 'a2a' })
end,
```

**Validation:** `luac -p` (Lua 5.4.8) clean on the driver;
`cargo fmt -p phlow-gauntlet -- --check` clean;
`cargo clippy -p phlow-gauntlet --all-targets -- -D warnings` zero warnings;
`cargo test -p phlow-gauntlet --test task_07` → `4 passed; 0 failed`
(30.5 s wall clock; each scenario spawns headless nvim against a live mock
peer on 127.0.0.1).
**Adversarial:** `peer-dies` (death mid-stream; found bug 2) and
`bad-state` (unknown `frobnicate` state; ignored, run uncorrupted) are the
adversarial half. Both ran against the real adapter and protocol code —
nothing in the failure path was mocked.

## Full technical depth

The driver exercises the real path: `adapters/a2a.lua` `start()` →
`ai.a2a.tasks.submit({ agent = run.extensions.a2a.agent, message = run.goal
})` → `client.message_stream` (because the mock card sets
`capabilities.streaming = true`; `tasks.lua:218-219` chooses
`message/stream`, else `message/send`) → curl subprocess via `vim.system`
parsing SSE `data:` frames → `tasks.set_state` through the kebab-case state
machine (`working`, `input-required`, `completed`, `failed`, `canceled`;
unknown states deliberately ignored) → `finish_task` → `on_done` →
adapter's `model.completed` sink event → `supervisor.tick` drains it into
the run transition.

Two deliberate deviations from the brief, both evidence-driven:

- The brief asked for `message/send` followed by `tasks/get` polling.
  Current diver cannot do that: `client.lua` implements `tasks/get`
  (`client.lua:364`), but a repo-wide grep finds **no caller** outside the
  definition — the supervisor never polls. The only real multi-state
  lifecycle path is streaming, so the driver exercises streaming. The
  `message/send` single-shot path is not covered; neither is `tasks/get`.
- The brief assumed `harness.run` works. It does not, for any adapter that
  reads `run.goal` (bug 1 above). The driver's internal `harness.run`
  reproduction is the minimal workaround that keeps the real adapter and
  protocol code under test.

Mock fidelity: the peer is a bounded `vim.uv` TCP server (single process,
no threads) implementing card discovery at
`/.well-known/agent-card.json` and JSON-RPC `message/stream`,
`message/send`, `tasks/get`, `tasks/cancel`. It validates the exact wire
shapes derived from diver's source: lowercase `role = "user"`, message
`kind = "message"`, text-part `kind = "text"`, and rejects malformed
requests. Scenario scripts: `default` emits working → completed with an
artifact update; `cancel` emits working then honors `tasks/cancel`;
`peer-dies` sends `Content-Length: 1048576`, one `working` event, then
destroys the connection after 300 ms; `bad-state` injects `frobnicate` as a
status-update state before completing. All handles are closed, all waits
are deadline-bounded (`WAIT_TIMEOUT_MS`), and the driver works only under
`GAUNTLET_WORK_DIR`.

Timing: each scenario completes in well under the 120 s test timeout; the
peer-death detection lands in ~1 s (300 ms death + curl exit + task settle).

## Sources

- Primary: `~/workspace/repos/diver/lua/ai/a2a/tasks.lua` (state machine,
  `on_done(task)` contract at :43/:170, streaming choice at :218-219,
  `message must be nonempty` at :322); `client.lua` (`tasks/get` at :364
  with no callers, `message/stream` curl handling);
  `lua/ai/a2a/fanout.lua:84` (correct `on_done` shape);
  `lua/ai/harness/adapters/a2a.lua:60-75` (wrong `on_done` shape);
  `lua/ai/harness/supervisor.lua:116` (`create` drops `goal`);
  `lua/ai/harness/types.lua:229-230` (`goal` validation);
  `lua/ai/harness/adapters/{acp.lua:94,herd.lua:54,rose.lua:57}` (other
  `run.goal` readers).
- Primary evidence: headless-nvim driver runs (four scenario verdicts);
  standalone curl probes against the mock peer (exit 18 on truncation);
  the temporary `vim.system` exit-code wrapper (exit 18 in the integrated
  path; removed after use); `cargo test` output (`4 passed; 0 failed`).
- Secondary: task brief's scenario list and gate requirements — taken as
  given where they match the sources, overridden where the sources
  disagree (streaming vs `tasks/get` polling; `harness.run` workaround).
