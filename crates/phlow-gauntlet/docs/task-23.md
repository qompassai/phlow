# Learning doc template — copy to `task-NN.md` and fill in

> Every section is required. ELI5 first, then full depth. Cite primary
> sources for every protocol/API claim. Document failures with evidence,
> not adjectives.

# task-23: bounded dynamic fan-out

**Kind:** nvim-lua · **Status:** pass · **Wave:** 21–25 · **Commits:** pending (wave 21-25)

## ELI5

Fan-out is one job splitting into many child jobs at runtime — like a
manager hiring a team after seeing the workload, not before. "Bounded"
means there's a hard cap on how many can exist, so a runaway job can't
create children forever and eat the machine. "Correct" for this task
means: a parent can spawn children on demand up to a live bound, and an
attacker who asks for 10,000 children gets a clean "no" for every one past
the cap — no hang, no crash, no memory explosion.

## What this task attempts

- **Goal:** spawn children dynamically through the real spawn path and
  prove the bound holds against an attacker-controlled N=10000.
- **Mechanism:** the real diver spawn path `supervisor.spawn_child` →
  `supervisor.create` (`~/workspace/repos/diver/lua/ai/harness/supervisor.lua`),
  whose bound is the total-run cap `RUNS_MAX = 256` (`run_count` never
  decrements). Driver: `crates/phlow-gauntlet/lua/gauntlet/task_23.lua`.
- **Success criterion:** 10,000 spawn attempts yield exactly 255 successes
  (the parent holds one of the 256 slots); every over-bound attempt fails
  with the explicit `'supervisor run bound exceeded'` error; a fork-bomb
  cascade terminates at the cap.
- **Non-goals:** per-parent live-child bounds and explicit depth bounds —
  neither exists. The design asked for a depth bound; the driver documents
  that the current mechanism only supplies the total-run cap, instead of
  claiming a bound that isn't there.

## What happened

Passed on the first test run after a pre-test driver correction (below).
Evidence from the three scenarios:

- `default` (V): 5 children spawned, started, and completed; the parent
  finished after its children; `parent_id` + `root_id` propagated to every
  child (`parent.children linked: 5 (structured ownership)`).
- `attacker-n` (A): `spawn attempts: 10000` → `spawned ok: 255;
  rejected: 9745`; every rejection carried the explicit
  `'supervisor run bound exceeded'` error; `run_count pinned at the cap:
  no unbounded growth, no OOM vector`; the spawn loop wall time was a
  few hundred ms — no hang.
- `recursive` (A): branching factor 3, cascade terminated exactly at the
  total-run cap (`no fork bomb`); every refusal was the explicit bound
  error; and the driver states the honest limitation: `no explicit depth
  bound exists — depth is bounded only by the total-run cap` (a linear
  chain could reach depth 255).

## Where it went wrong

The first draft of the driver would have failed `default` on every run —
caught by reading the supervisor source before any test executed, so no
test attempt ever ran against the broken version.

- **Stage:** parent setup in `start_parent`.
- **Symptom (predicted, then verified by source reading):** `spawn_child`
  rejected with `'parent run is already terminal'`, or the parent's
  explicit `finish` rejected with `'run is already terminal: completed'`.
- **Root cause:** the parent was started with the instant-completing fake
  adapter. `harness.run` calls `supervisor.start_run`, whose adapter
  immediately appends `model.completed`; on the next `supervisor.tick`,
  `drain_completions` finishes the run. The parent was therefore terminal
  before (or during) the children's lifecycle, and `M.create` refuses
  spawning on a terminal parent (`supervisor.lua`: `'parent run is already
  terminal'`).

## The fix — what changed and why

- **Changed:** `crates/phlow-gauntlet/lua/gauntlet/task_23.lua` — added a
  second fake adapter, `gauntlet_holding`, whose `start` deliberately
  appends nothing to the sink (no `model.completed` event, so
  `drain_completions` never auto-finishes it). `start_parent` uses
  `gauntlet_holding`; children keep `gauntlet_instant`.
- **Commit:** pending (wave 21-25).
- **Why:** the parent must stay `running` while children spawn — that is
  the actual precondition `M.create` enforces. The alternative (not
  starting the parent) would leave it `created` and non-terminal, which
  also passes the spawn check, but a *started but unfinished* parent is
  the realistic fan-out shape and additionally exercises the
  `finish`-refuses-live-children rule in `default`.
- **Source:** `~/workspace/repos/diver/lua/ai/harness/supervisor.lua` —
  `M.create` (`'parent run is already terminal'`),
  `drain_completions` (`model.completed` → `M.finish`), `M.finish`
  (`'parent run owns live children'`, `'run is already terminal'`).
- **Validation agents:** the 2 validation tests
  (`small_n_fan_out_spawns_and_completes`,
  `children_carry_parent_and_root_ids`) assert the spawn/complete/finish
  order and the ownership linkage.
- **Adversarial agents:** the 2 adversarial tests
  (`attacker_n_10000_hits_the_live_bound`,
  `recursive_cascade_terminates_at_the_cap`) run the 10,000-attempt storm
  and the fork-bomb cascade against the real bound.
- **New convention (if any):** none — the holding-adapter pattern is
  task-local. (The task-17 lesson it reuses: per-test scratch dirs via
  pid + atomic counter, so parallel scenarios never share a workdir.)

## Full technical depth

`harness.run(spec)` → `supervisor.create` (validates the spec, checks
`sup.run_count >= sup.runs_max` → `'supervisor run bound exceeded'`,
links `parent_id`/`root_id`, initializes `children = {}`) →
`supervisor.start_run` (transitions `created` → `queued`, calls the
adapter's `start`). `spawn_child(sup, parent_id, spec)` stamps
`spec.parent_id` and delegates to `create`, so the spawn bound, the
terminal-parent refusal, and the unknown-parent refusal are all the
*creation* checks — there is no separate spawn gate.

The bound's shape matters: `RUNS_MAX = 256` caps *total runs ever
created* (`run_count` never decrements), which implies live runs can
never exceed it either. There is no per-parent live-child counter and no
depth counter — a linear chain of 255 single children is legal, and the
driver's `recursive` scenario documents exactly that instead of claiming
a depth bound the code doesn't have.

`M.finish` refuses while `has_live_children` (any non-terminal child)
and refuses an already-terminal run — the `default` scenario exercises
the first refusal implicitly (children are finished via ticks before the
parent's explicit finish) and would trip the second if the parent
self-completed, which is why the holding adapter exists.

## Sources

- Primary: `~/workspace/repos/diver/lua/ai/harness/supervisor.lua`
  (`M.create` bound + terminal-parent check, `spawn_child`,
  `drain_completions`, `M.finish`, `RUNS_MAX`).
- Primary: `~/workspace/repos/diver/lua/ai/harness/init.lua` (`M.run` —
  create + start_run).
- Driver: `crates/phlow-gauntlet/lua/gauntlet/task_23.lua` (three
  scenarios; `gauntlet_instant` vs `gauntlet_holding`).
- Tests: `crates/phlow-gauntlet/tests/task_23.rs` (2V/2A).
