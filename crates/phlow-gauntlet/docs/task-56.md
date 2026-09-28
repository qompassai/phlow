# task-56: approval timeout defaults deny

**Kind:** nvim-lua · **Status:** pass · **Wave:** 56–60 · **Commits:** pending (wave 56-60)

## ELI5

A robot asks a human "can I do this dangerous thing?" and waits. If
the human answers YES in time, it goes ahead. If the human says NO,
it stops. And if the human says nothing at all and the timer runs
out — the robot must treat the silence as NO, never as "sure, go
ahead." Silence is a denial by default, because a bug that lets
unanswered requests through would let the robot do anything by just
waiting for the human to look away. This task proves diver's approval
timer really does default to deny.

## What this task attempts

- **Goal:** prove the real approval path fails closed on timeout —
  the design's security contract ("approval requests must time out
  with a default-deny, never grant on timeout"), with positive proof
  that the blocked tool never ran.
- **Mechanism:** `lua/gauntlet/task_56.lua`, a headless-Neovim driver
  that uses the real queue + supervisor APIs against the real diver
  Lua tree: `ai.harness.approval` (`request`, `decide`, `get`,
  `pending`, `sweep_expired`) and `supervisor.tick` (which invokes
  `sweep_expired`). A marker file proves the blocked tool did not run.
  Scenarios are selected via `GAUNTLET_SCENARIO`: `responds-in-time`
  (V), `terminal-within-deadline` (V), `timeout-never-grants` (A),
  `late-approval-cannot-resurrect` (A). The driver also writes a
  `report.md` per scenario.
- **Success criterion:** the design's default-deny pass criteria
  (timely approval grants; silent request terminal by deadline; a
  timeout never grants; a late approval cannot resurrect an expired
  request).
- **Non-goals:** renaming diver's `expired` state or changing its
  approval semantics — the naming mismatch below is documented, not
  "fixed."

## What happened

Pass — the security requirement holds, with one honest naming note.
Only `pending` approvals can be decided; a timeout produces the
terminal state `expired`, not literal `denied` — but `expired` is the
deny-equivalent: the tool never runs, and a late `decide(...,
'approved', ...)` is rejected as already expired. The driver:

- `timely_approval_grants_and_tool_proceeds` (V): the
  `responds-in-time` control — an approval decided before the
  deadline grants, and the marker file proves the tool ran.
- `silent_request_is_terminal_by_deadline_plus_epsilon` (V): a
  silent request is terminal by deadline + epsilon — the record
  leaves `pending` and reaches a terminal deny-equivalent state.
- `timeout_defaults_to_deny_never_grants` (A): the timeout never
  grants — no default-allow-on-timeout bug; the tool's marker file is
  absent.
- `late_approval_cannot_resurrect` (A): a late approval after expiry
  is rejected ("already expired"); the record stays expired.

Honest naming note (documented, not repaired): the module header
comment says requests "expire to denied," but the code's terminal
state is named `expired` (the record type is
`pending|approved|denied|expired`). The driver asserts the SECURITY
property — default-deny on timeout — and names the state exactly as
the module does rather than inventing a `denied` state the module
never produces.

## The fix — what changed and why

No fix — the security requirement passes as implemented. The
gauntlet-side work was making the evidence honest:

- **Changed:** `crates/phlow-gauntlet/lua/gauntlet/task_56.lua` (new)
  and `crates/phlow-gauntlet/src/tasks/task_56.rs` (new) — the driver
  uses the real queue + supervisor APIs rather than a test double,
  with a marker file for positive proof that the blocked tool did not
  run, and documents the `expired`-vs-`denied` naming mismatch.
- **Why:** a test double could "prove" default-deny without the
  product ever enforcing it. Driving `supervisor.tick` through the
  real `sweep_expired` path ties the verdict to the product's actual
  expiry machinery.
- **Source:** diver `lua/ai/harness/approval.lua`
  (`request`/`decide`/`get`/`pending`/`sweep_expired`) and
  `lua/ai/harness/supervisor.lua` (`M.tick`).
- **Validation agents:** the 2 validation tests
  (`timely_approval_grants_and_tool_proceeds`,
  `silent_request_is_terminal_by_deadline_plus_epsilon`) assert the
  timely path works and the silent path terminates.
- **Adversarial agents:** the 2 adversarial tests
  (`timeout_defaults_to_deny_never_grants`,
  `late_approval_cannot_resurrect`) assert the timeout never grants
  and late approval cannot resurrect the record.

## Full technical depth

The driver bootstraps the real harness from `DIVER_LUA_DIR` (the
same pattern as the other nvim-lua tasks: a scratch rtp shim of two
symlinks, no diver file touched). For each scenario it creates a real
approval queue, issues `approval.request` with a short timeout, and
drives expiry by calling `supervisor.tick` directly — the same
function the production supervisor loop calls. The `get` record's
`state` field is asserted, and the tool-side marker file is checked
for presence (granted scenarios) or absence (denied/expired
scenarios). The adversarial facets cover the two dangerous
directions: timeout-becomes-grant (the default-allow bug) and
late-decide-after-expiry (resurrection). Both hold shut.

The one mismatch the probe found is vocabulary, not security: the
header comment promises "expire to denied," but the record type is
`pending|approved|denied|expired` and the expiry path writes
`expired`. `expired` is a deny-equivalent — the approval is over, the
tool never ran, no further `decide` can change it — but a downstream
consumer matching on the literal string `denied` would miss expired
records. That consumer-side risk is banked as a product note, not
changed here.

## Sources

- Primary: diver `lua/ai/harness/approval.lua` (the approval queue:
  `request`, `decide`, `get`, `pending`, `sweep_expired`;
  `expired` terminal state) and `lua/ai/harness/supervisor.lua`
  (`M.tick` driving `sweep_expired`).
- Driver: `crates/phlow-gauntlet/lua/gauntlet/task_56.lua` (real-API
  driver with marker-file proof).
- Driver: `crates/phlow-gauntlet/src/tasks/task_56.rs` (nvim-lua
  runner; scenario selection via `GAUNTLET_SCENARIO`).
- Tests: `crates/phlow-gauntlet/tests/task_56.rs` (2V/2A).
