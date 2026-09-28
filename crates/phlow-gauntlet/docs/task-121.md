# task-121: deny-all default and explicit opt-in

**Kind:** nvim-lua · **Status:** partial (open, diver gap: `policy_example.lua` not shipped) · **Wave:** 121–125 · **Commits:** pending (wave 121-125)

## ELI5

The harness ships with a rulebook that says "no to everything" and an
empty list of exceptions. There is supposed to be a second rulebook
sitting *next to* it — "allow looking around, allow harmless local
changes" — that the operator can pick up with one explicit line, because
"harmless" is currently just the adapter's promise about itself. Today
the first rulebook is exactly as designed, the guard refuses everything
it should refuse, and nobody can sneak the second rulebook in by default
— but the second rulebook itself was never written. "Correct" means: the
default stays deny-all with zero rules, the example stays opt-in, and the
one-line opt-in actually works when Phase 2 ships the example.

## What this task attempts

- **Goal:** prove the default posture is deny-all with no implicit
  opt-in, and pin the exact gap where Decision 1's example module should
  be.
- **Mechanism:** diver's `ai.harness` — `init.lua` `M.setup` (policy
  wiring), `policy.lua` `M.new` (default posture) and `M.decide`
  (fail-closed), the (absent) `ai.harness.policy_example` module — via the
  driver `crates/phlow-gauntlet/lua/gauntlet/task_121.lua`.
- **Success criterion:** fresh `setup({})` yields `default = 'deny'`,
  `rules = {}`; `decide` denies on empty rules, nil policy
  ('no policy configured'), malformed requests, and unknown risk classes;
  the setup path never requires the example module implicitly; the
  documented one-line opt-in loads a module that allows `observe` and
  still denies `network`.
- **Non-goals:** designing the policy language, or vetting whether
  `local_reversible` is truly reversible. The "unverified claim" rationale
  is the *reason* for explicit opt-in, not something this task proves.

## What happened

Partial — three of four scenarios pass today; the fourth is the
Decision-1 gap:

- `default` passes: `setup({})` yields `default = 'deny'`, `rules = {}`
  (init.lua "Fail closed" wiring; policy.lua `M.new`).
- `decide-fail-closed` passes: `decide` denies on empty rules ('no rule
  matched'), nil policy ('no policy configured'), non-table requests
  ('malformed request'), and unknown risk ('unknown risk class').
- `no-implicit-example` passes: static scan of init/supervisor/policy
  sources shows no `policy_example` reference; `package.loaded` is clean
  after setup; post-setup rules are exactly empty. The regression guard
  holds — a future "helpful" auto-enable fails this test by construction.
- `opt-in-absent` fails with `where = "policy-example-absent"`:
  `require('ai.harness.policy_example')` fails — no such file exists
  anywhere under diver `lua/` (repo-wide find).

## The fix — what changed and why

No fix — the gap is diver-owned (Phase-2 Decision 1), and diver findings
are never fixed under gauntlet authority. The gauntlet-side work was
getting the evidence right:

- **Changed:** `crates/phlow-gauntlet/lua/gauntlet/task_121.lua` (new) —
  four scenarios: deny-all characterization, decide fail-closed battery,
  implicit-load regression guard, opt-in gap record.
- **Why:** the task is about *default posture* — what the system does when
  the operator configures nothing — and about keeping an opt-in from
  decaying into a default. The guard scenario is the durable artifact:
  without it, a well-meaning change could silently enable the example.
- **Source:** `~/workspace/repos/diver/lua/ai/harness/init.lua` `M.setup`
  (policy wiring), `~/workspace/repos/diver/lua/ai/harness/policy.lua`
  `M.new` (default posture) and `M.decide` (fail-closed).
- **Validation agents:** the 2 validation tests
  (`default_deny_all_no_rules`, `decide_fail_closed_characterization`)
  assert the deny-all default and the fail-closed battery.
- **Adversarial agents:** the 2 adversarial tests
  (`no_implicit_example_regression_guard`, `opt_in_absent_gap_recorded`)
  assert the setup path never loads the example implicitly and record the
  missing module with its acceptance criterion.

## Full technical depth

`M.setup({})` calls `policy_mod.new(opts.policy)` with `opts.policy ==
nil`; `M.new` normalizes to `{ default = 'deny', rules = {} }`. `M.decide`
denies before any rule matching when the policy is nil ('no policy
configured'), the request is not a table ('malformed request'), or the
risk class is unknown ('unknown risk class'); with empty rules it falls
through to `policy.default`, i.e. deny ('no rule matched'). The example
module is Phase-2 surface: per the spec it ships as
`lua/ai/harness/policy_example.lua` returning
`{ default = 'deny', rules = { { risk = 'observe', decision = 'allow' }, { risk = 'local_reversible', decision = 'allow' } } }`,
enabled by `setup({ policy = require('ai.harness.policy_example') })`.
Repo-wide find confirms no `policy_example*` file exists anywhere in
diver; the harness sources contain no reference to it.

Phase-2 acceptance (banked, diver-owned): the documented one line loads
the module; `decide` allows `observe` requests and still denies `network`
(remote adapters classify `'network'`, matching no allow rule); the
regression guard keeps passing (example never loaded implicitly).

## Sources

- Primary: `~/workspace/repos/diver/lua/ai/harness/init.lua` `M.setup`
  (policy wiring, "Fail closed" comment).
- Primary: `~/workspace/repos/diver/lua/ai/harness/policy.lua` `M.new`
  (default posture) and `M.decide` (fail-closed battery).
- Spec: `~/workspace/your_files/diver-harness-phase2-spec.md` Decision 1
  and the `policy_example.lua` listing (lines 150-164).
- Driver: `crates/phlow-gauntlet/lua/gauntlet/task_121.lua` (four
  scenarios; three pass, one `where = "policy-example-absent"`).
- Tests: `crates/phlow-gauntlet/tests/task_121.rs` (2V/2A).
- Diver probed: `c84352cc850d507df477706b9166b6541ebe9e1c` (main; no
  Phase-2 branch exists).
