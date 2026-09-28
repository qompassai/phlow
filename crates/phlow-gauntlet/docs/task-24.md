# Learning doc template — copy to `task-NN.md` and fill in

> Every section is required. ELI5 first, then full depth. Cite primary
> sources for every protocol/API claim. Document failures with evidence,
> not adjectives.

# task-24: priority preemption

**Kind:** rust · **Status:** fail (open) · **Wave:** 21–25 · **Commits:** pending (wave 21-25)

## ELI5

Priority is a pecking order for jobs: the urgent job jumps the queue ahead
of the routine one. Preemption is stronger — it's tapping a running job on
the shoulder, pausing it mid-work, running the urgent job, and then
*resuming* the paused one where it left off. "Correct" for this task
means: the scheduler understands priority levels, and an urgent arrival
can preempt a running job and later resume it with its state intact.

## What this task attempts

- **Goal:** prove phlow's scheduler honors run priority and supports
  preemption with resume.
- **Mechanism:** a case-insensitive source scan of every non-gauntlet
  `crates/*/src` tree for `priorit`/`preempt`, structural checks on
  `SchedulerLimits` and `NodeParams`
  (`crates/phlow-experiment/src/control_plane.rs`), and behavioral probes
  against the real `Scheduler` (scenario binary compiling the real
  `error.rs` + `control_plane.rs` via `#[path]`).
- **Success criterion:** priority affects admission/order, and a
  preempted run resumes with state intact.
- **Non-goals:** inventing priority support. The design says an evidenced
  `where = "seam"` failure is the correct result for an absent seam, and
  diver-style findings are documented, never fixed under gauntlet
  authority.

## What happened

Fail, open seam — on the first and only attempt. Phlow has no
run-priority or preemption support:

- The source scan finds zero `priorit`/`preempt` hits across all
  non-gauntlet crate sources.
- `SchedulerLimits` fields are exactly `{workers_max, queue_capacity,
  children_per_task_max, depth_max, task_deadline_ms,
  aggregate_tool_calls_max, aggregate_output_bytes_max}` — no priority,
  quantum, or aging knob.
- `NodeParams` has no priority field.
- The only interruption primitive, `cancel_run`, is *terminal*: the run
  moves to `Cancelled` with no resume path, and the run id stays poisoned
  (re-admit → `RunCancelled`).

The four behavioral cases document the absence precisely:

- `admission_is_priority_blind` (V): a "low-priority" node admitted before
  a "high-priority" one — both `Admitted`, FIFO order kept; priority
  labels are inert strings.
- `limits_have_no_priority` (V): the tuning surface enumerates with no
  priority/preemption/quantum/aging knob.
- `cancellation_is_terminal` (A): `cancel_run` moves the run to `Cancelled`;
  a late publish is refused (`RunCancelled`) — preemption's "resume with
  state intact" half does not exist.
- `cancelled_run_stays_poisoned` (A): after cancellation, re-admit and
  re-cancel are refused; the id is poisoned monotonically — no
  preempt/un-preempt cycle exists to starve or age within.

The task-level `run` reports `fail` with `where = "seam"`.

## The fix — what changed and why

No fix — this is a documented design gap, never fixed under gauntlet
authority. Two gauntlet-side corrections were made before any test ran:

- **Changed:** `crates/phlow-gauntlet/src/tasks/task_24.rs` — the recon
  source scan now skips `crates/phlow-gauntlet` itself.
- **Why:** the scan is case-insensitive over production sources; the
  gauntlet's own task-24 driver and tests mention "priority"/"preemption"
  constantly, so scanning the harness would self-falsify the absence
  claim. The evidence line now reads "non-gauntlet crates" to say exactly
  what was scanned.
- **Source:** the scan code itself (`recon()` in `task_24.rs`) plus the
  driver/test sources it would have matched.
- **Validation agents:** the 2 validation tests
  (`admission_is_priority_blind`, `limits_carry_no_priority_knob`) assert
  the priority-blind behavior and the knob-less tuning surface.
- **Adversarial agents:** the 2 adversarial tests
  (`cancellation_is_terminal_no_resume_path`,
  `cancelled_run_stays_poisoned_and_task_reports_seam`) try to resume a
  cancelled run and to cycle a run through preempt/un-preempt — both
  refused — and the folded-in task-level assertion pins
  `where = "seam"`.

## Full technical depth

`SchedulerLimits` is a plain struct of numeric caps; admission checks a
node's declared budget against them and refuses over-limit nodes, but no
field ranks nodes against each other. `NodeParams` carries identity,
capabilities, budget, and `dependency_ids` — nothing ordinal. The
scheduler's queues are FIFO admission ledgers; there is no priority queue,
no quantum, no aging.

Interruption exists in exactly one form: `cancel_run` transitions a run to
`Cancelled` (terminal) and poisons the id — subsequent `admit` with the
same id fails with `RunCancelled`, and a second `cancel_run` moves zero
nodes. There is no suspend/resume, no checkpoint, no "preempted" state
distinct from `Cancelled`. A priority/preemption design would need at
minimum: a priority field on the node params, a priority-ordered
ready-set, a preempt operation that suspends a running node *without*
poisoning its id, and a resume path that restores its state. None exists.

## Sources

- Primary: `crates/phlow-experiment/src/control_plane.rs`
  (`SchedulerLimits`, `NodeParams`, `Scheduler::admit`, `cancel_run`).
- Primary: `crates/phlow-experiment/src/error.rs` (`RunCancelled`).
- Driver: `crates/phlow-gauntlet/src/tasks/task_24.rs` (`recon()` scan
  scope, scenario template, four cases, `seam_finding`).
- Tests: `crates/phlow-gauntlet/tests/task_24.rs` (2V/2A).
