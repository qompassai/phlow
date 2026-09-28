# task-03: approval gate blocks unapproved tool use

**Kind:** nvim-lua · **Status:** pass · **Wave:** 1 ·
**Commits:** (uncommitted; do not commit per program rules)

## ELI5

The agent harness has a security checkpoint: before an agent's tool call is
allowed to do something real (like write a file), a policy decides whether
the call is safe to allow, needs a human's explicit approval, or must be
denied outright. This task proves that checkpoint actually works. A tool call
that a human approves must really run; a tool call nobody approves must never
run — not "we didn't see it run", but *provably* never ran, because the gate
returned before the execute step and the only trace the tool could leave (a
marker file) is verifiably absent. Think of it like a bank vault: approval is
the combination, and we prove the door stayed shut by showing the money is
still inside *and* the lock log says "never opened".

## What this task attempts

- **Goal:** Prove the harness approval gate enforces human-in-the-loop: an
  approved tool call proceeds; an unapproved one never executes.
- **Mechanism:** `crates/phlow-gauntlet/lua/gauntlet/task_03.lua` (driver,
  headless Neovim) drives diver's real `lua/ai/harness/policy.lua`
  (`policy.decide` — "the only path from a model proposal to a side effect")
  and `lua/ai/harness/approval.lua` (the async approval queue:
  `request`/`decide`/`pending`/`sweep_expired`) via
  `crates/phlow-gauntlet/src/tasks/task_03.rs`
  (`run` → `run_scenario(ctx, "default")` → `run_nvim_lua_driver_with_env`
  with `GAUNTLET_SCENARIO`). Tests in `crates/phlow-gauntlet/tests/task_03.rs`.
- **Success criterion:** 4 driver verdicts, all `pass`: (1) approver grants →
  tool executes (marker file written, approval record `approved` with
  `decided_by` set); (2) no approver → call blocked, tool provably never
  executed (`executed=false` flag *and* marker file absent), approval expires
  to `denied` via the real `supervisor.tick()` path; (3) approver denies →
  blocked, denial recorded exactly; (4) unknown tool → policy denies with
  `no rule matched`, no approval ever requested.
- **Non-goals:** No live LLM backend is involved (no model proposes tools in
  the sandbox); the harness's protocol adapters (acp/a2a/mcp/...) are not
  exercised; the approval UI surface is not tested — only the decision
  queue and the policy gate.

## What happened

Passed on the first full run after two real issues were found and fixed
during development (see below). `cargo test -p phlow-gauntlet --test task_03`:
4/4 green (2 validation, 2 adversarial). `cargo clippy --all-targets`
zero warnings; `cargo fmt --check` clean on all task-03 files;
`luac -p` clean on the driver. Direct headless-Neovim runs of all four
scenarios each printed exactly one JSON verdict line with `outcome: pass`
and exit code 0.

## Where it went wrong

(Iteration 1 — design-time finding, not a test failure.)

- **Stage:** Reading the harness before designing the attempt.
- **Symptom:** `policy.decide` and `approval.request` have **zero callers**
  anywhere in diver's `lua/ai/harness/` (verified with `grep -rn` across the
  module tree). The supervisor owns `sup.policy` and `sup.approvals` and
  drives `approval.sweep_expired` on `tick()`, but no harness module wires
  *tool proposal → policy.decide → approval.request → wait → execute*.
  There is no tool executor in the harness at all, and no live human
  backend in this sandbox.
- **Evidence:** `grep -rn "policy.decide\|approval.request" lua/ai/harness/`
  returns only the definitions in `policy.lua:175` and `approval.lua:46`;
  `supervisor.lua` references `sup.policy`/`sup.approvals` but never calls
  `policy.decide`.
- **Root cause:** The harness (v0.1.0) ships the *decision point* and the
  *decision record* as real, tested modules, but the *wiring* between a
  model tool proposal and those modules is expected to live in the adapter
  or the embedding application — which does not exist in this sandbox.

(Iteration 2 — driver name rejected by the framework gate.)

- **Stage:** `cargo test` (first run).
- **Symptom:** All 4 tests failed with
  `where: spawn / how: rejected driver script name 'task_03.lua' (want task-NN.lua)`.
- **Root cause:** At the time, `is_driver_script_name` in
  `crates/phlow-gauntlet/src/lib.rs` accepted only the `task-NN.lua` shape
  while the brief's contract named the driver `task_03.lua`. The
  coordinator then settled the convention repo-wide: driver filenames use
  UNDERSCORE (`task_NN.lua`, matching the Rust module filenames), and the
  framework gate was updated to accept only that shape. Final state:
  driver is `lua/gauntlet/task_03.lua`, the Rust module passes
  `"task_03.lua"`, and the task id keeps its hyphen (`task-03`).
  No framework files were touched by this worker.

## The fix — what changed and why

- **Changed (design):** Instead of inventing a fake approval path, the
  driver implements the gate *exactly* at the documented enforcement point:
  `policy.decide(sup.policy, request)` first; on `approval`, enqueue with
  the real `approval.request` on the real supervisor-owned queue, then let a
  declared test-side stand-in act *only* through the real
  `approval.decide(queue, id, decision, by)` API (the actual grant/deny
  path); execute the tool only after observing state `approved`. When the
  approval is still pending, the gate drives the real
  `supervisor.tick()` past the deadline to exercise the harness's own
  fail-closed rule ("requests expire to denied, never to approved").
- **Why:** This is the closest real seam. The stand-in approver is the one
  declared mock, and it is declared in every verdict's evidence because a
  live human cannot exist in this sandbox. Everything else — the policy
  table (installed via the public `harness.setup({policy=...})`), the
  approval queue, the expiry sweep, the run the approval attaches to — is
  the harness's own code.
- **Source:** `lua/ai/harness/policy.lua` header ("The only path from a
  model proposal to a side effect. … Default-deny"); `approval.lua` header
  ("Requests expire to denied, never to approved"); `supervisor.lua`
  `tick()` calling `approval.sweep_expired`; `types.lua` `RISK_CLASSES`
  and the `tool.approval_requested` event kind.
- **Validation:** 4 headless-Neovim scenario runs (one JSON verdict line
  each, exit 0); `cargo test -p phlow-gauntlet --test task_03` 4/4;
  clippy `-D warnings` clean; `cargo fmt --check` clean; `luac -p` clean.
- **Adversarial:** the two adversarial tests *are* the red team — denial
  and unknown-tool paths must fail closed. A fifth probe was done by hand:
  `no-approver` with a `tick()` past the deadline proves expiry yields
  `expired` (treated as denied), never a silent grant. The policy was
  installed with `default='deny'` so any rule miss denies.
- **New convention:** coordinator-settled: Lua driver filenames use
  underscore (`lua/gauntlet/task_NN.lua`, matching the Rust module names);
  the task id keeps its hyphen (`task-03`); the framework's
  `is_driver_script_name` gate accepts only `task_NN.lua`. Evidence: the
  gate source in `src/lib.rs` and this task's rename cycle.

## Full technical depth

Data flow, per scenario. The Rust side (`task_03.rs`) is a thin launcher:
`run_scenario` passes `GAUNTLET_SCENARIO` through
`run_nvim_lua_driver_with_env`, which spawns
`nvim --headless -l <crate>/lua/gauntlet/task-03.lua` with
`DIVER_LUA_DIR`, `GAUNTLET_WORK_DIR` (= `<work_dir>/task-03`), and the
scenario in the environment, enforces `ctx.timeout`, and parses the single
JSON verdict line into `TaskOutcome::Pass/Fail`.

Inside the driver:

1. `vim.opt.runtimepath:append(DIVER_LUA_DIR)`, then
   `require('ai.harness')` → `harness.setup({policy = {default='deny',
   rules={{risk='local_reversible', tools={'fs.write'},
   decision='approval'}}}})`. This is the public API path a real
   deployment uses; fail-closed is the default posture, so the explicit
   `default='deny'` documents intent rather than changing behavior.
2. The driver reaches the real supervisor via `harness._state.supervisor`
   (declared seam: the public API exposes no approval surface) and creates
   a real run (`supervisor.create`, state `created`, generous `time_ms`
   budget so the expiry-driving tick cannot trip budget exhaustion — the
   budget is not under test). The run is never started: no adapter backend
   exists in the sandbox, and starting one is not needed for the gate.
3. The gate: `policy.decide(sup.policy, request)` →
   - `deny` (e.g. unknown tool `fs.exec`/risk `process`: "no rule
     matched") → return blocked; no approval requested
     (verified: `approval.pending` count unchanged).
   - `approval` → `approval.request(sup.approvals, run.id, request,
     {timeout_ms=1000})` → stand-in approver acts via the real
     `approval.decide` (grant / deny / absent) → if still `pending`,
     `supervisor.tick(sup, deadline_ns + 1)` → `sweep_expired` flips it to
     `expired` (fail-closed) → execute only on `approved`.
4. The "tool" is `io.open(work_dir .. '/tool-executed.txt', 'w')` plus an
   `executed` flag. Non-execution is proved positively: gate returned
   before the execute call site *and* `executed == false` *and* the marker
   file is absent.
5. Verdict: exactly one `vim.json.encode` line
   `{id, outcome, [where, how], evidence[]}` (≤64 lines), always exit 0.

Recorded state per scenario: grant → approval `approved`,
`decided_by='task-03:stand-in-approver'`; absent → `expired`,
`decided_by=nil`; denied → `denied` with `decided_by` set and the run
still `created` (never started — documented exactly, not assumed).

## Sources

- Primary: `~/workspace/repos/diver/lua/ai/harness/{policy,approval,supervisor,types,init}.lua`
  (read in full before designing); `crates/phlow-gauntlet/src/lib.rs`
  `run_nvim_lua_driver_with_env` / `is_driver_script_name`;
  `crates/phlow-gauntlet/lua/gauntlet/task_03.lua` (this task's driver).
- Secondary: `~/workspace/skills/tiger-style-rust/SKILL.md`,
  `~/workspace/skills/tiger-style-lua/SKILL.md` (style constraints only).

## Open questions

1. The harness has no tool executor wiring `policy.decide` →
   `approval.request` → execute. Is that wiring intended to live in a
   future adapter, or should `policy.lua`'s "the only path" claim be
   backed by a harness-owned `tool_gate` module? Worth raising with Matt.
