# Learning doc template — copy to `task-NN.md` and fill in

> Every section is required. ELI5 first, then full depth. Cite primary
> sources for every protocol/API claim. Document failures with evidence,
> not adjectives.

# task-21: saga compensating transactions

**Kind:** nvim-lua · **Status:** fail (open) · **Wave:** 21–25 · **Commits:** pending (wave 21-25)

## ELI5

A saga is a long job broken into steps where each step has an undo button.
Imagine booking a trip: reserve a flight, then a hotel, then a car. If the
car rental fails, you don't leave the flight and hotel booked — you run the
undo buttons in *reverse order*: cancel the hotel, then cancel the flight.
That reverse-order undo list is called "compensation". "Correct" for this
task means: there is a real coordinator that runs the steps, and if a step
fails, it runs every earlier step's undo in reverse. Without a coordinator,
the steps are just names on a list and nothing undoes anything.

## What this task attempts

- **Goal:** drive a real saga coordinator that executes a 4-step saga and,
  on failure, runs compensations in reverse order.
- **Mechanism:** diver's `ai.harness` workflow layer — `ai.harness.registry`
  (`~/workspace/repos/diver/lua/ai/harness/registry.lua`),
  `ai.harness.run` (`init.lua`), `types.validate_run_spec` (`types.lua`) —
  via the driver `crates/phlow-gauntlet/lua/gauntlet/task_21.lua`.
- **Success criterion:** a saga-shaped definition executes steps in order
  and, on a forced failure, compensates in reverse.
- **Non-goals:** inventing a coordinator in the driver. The design forbids
  inventing absent seams; an evidenced `where = "seam"` failure is the
  correct result when the design says an absent seam is valid.

## What happened

Fail, open seam — on the first and only attempt. Diver's `ai.harness` has
a workflow *naming* layer but no workflow *runner*:

- `register_workflow` / `get_workflow` store and return a table
  (`name → { adapter = ... }`); the registered saga definition comes back
  intact, steps included — it just never executes.
- `types.validate_run_spec` requires `spec.workflow` to be a non-empty
  string and rejects a missing label: `workflow` is a validated *label*,
  not a reference to executable work.
- `harness.run` passes the spec straight to `supervisor.create`; it never
  consults the workflow registry (verified by reading
  `lua/ai/harness/init.lua`).
- No `run_workflow`, `execute_workflow`, saga coordinator, step runner, or
  compensation primitive exists anywhere in the harness surface.

Every driver scenario therefore reports `fail` with `where = "seam"`. The
four scenarios document the absence from four angles: `default` (naming
layer works, label validates, nothing runs), `naming-layer`
(`harness.run` creates exactly one normal run and never consults the
registry), `no-executor` (no executor callable exists on the harness
surface), `inert-def` (a saga-shaped definition stays inert).

## The fix — what changed and why

No fix — this is a documented diver finding, and diver findings are never
fixed under gauntlet authority. The "fix" for the gauntlet side was getting
the evidence right, which is recorded in the driver itself:

- **Changed:** `crates/phlow-gauntlet/lua/gauntlet/task_21.lua` (new) —
  four scenarios, each ending in `seam_absent()` with file-level evidence
  instead of building a coordinator.
- **Why:** building a coordinator in the driver would invent the seam the
  design says is valid to lack; the honest result is the evidenced
  `where = "seam"` verdict.
- **Source:** `~/workspace/repos/diver/lua/ai/harness/registry.lua`
  (register/get_workflow), `init.lua` (`M.run` → `supervisor.create`,
  no registry consult), `types.lua` (`validate_run_spec`).
- **Validation agents:** the 2 validation tests (`default_reports_seam_absence_with_source_evidence`,
  `run_entry_point_also_reports_seam`) assert the verdict shape and the
  evidence citations.
- **Adversarial agents:** the 2 adversarial tests
  (`naming_layer_scenario_registry_is_never_consulted`,
  `no_executor_scenario_finds_no_hidden_coordinator`) probe for a hidden
  executor path and for registry consultation smuggling steps through the
  label — both come back empty, as designed.

## Full technical depth

The harness's workflow layer is a registry: `register_workflow(registry,
name, def)` stores `def` under `name`; `get_workflow(registry, name)`
returns it. Nothing reads the def back for execution. A run spec carries
`workflow = "some-name"`; `types.validate_run_spec` checks only that the
string is non-empty (a missing label is rejected with a validation error —
the driver asserts both directions). `harness.run(spec)` calls
`supervisor.create(sup, spec)` and then `supervisor.start_run` with
`spec.adapter`: the adapter runs, not the workflow. The supervisor knows
nothing about steps or compensation — it tracks run state, budgets, and
deadlines.

A saga coordinator would need, at minimum: a step list interpreter that
invokes each step as a run, a journal of completed steps, and on failure a
reverse-order compensation dispatcher. None of that exists; the gap is in
diver, not in phlow's Rust crates. The diver finding: `ai.harness` needs
either a real workflow runner (making `register_workflow` meaningful) or
the workflow layer documented as a naming-only convenience. Until then,
any "saga" built on it is a label on a single run.

## Sources

- Primary: `~/workspace/repos/diver/lua/ai/harness/registry.lua`
  (`register_workflow`, `get_workflow` — naming layer only).
- Primary: `~/workspace/repos/diver/lua/ai/harness/init.lua` (`M.run` —
  passes spec to `supervisor.create`, never consults the registry).
- Primary: `~/workspace/repos/diver/lua/ai/harness/types.lua`
  (`validate_run_spec` — `workflow` is a non-empty-string requirement).
- Driver: `crates/phlow-gauntlet/lua/gauntlet/task_21.lua` (four
  scenarios, all `where = "seam"`).
- Tests: `crates/phlow-gauntlet/tests/task_21.rs` (2V/2A).
