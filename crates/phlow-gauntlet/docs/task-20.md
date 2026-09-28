# Learning doc template — copy to `task-NN.md` and fill in

> Every section is required. ELI5 first, then full depth. Cite primary
> sources for every protocol/API claim. Document failures with evidence,
> not adjectives.

# task-20: deadline expiry (at-most-once)

**Kind:** rust · **Status:** fail → fixed · **Wave:** 4b · **Commits:** pending (wave 4b)

## ELI5

An experiment is a kitchen timer with a dish attached. When the timer rings
(the deadline expires), the dish's state becomes final: `timed_out`. After
that, three things must hold. First, the result is recorded exactly once —
if a second copy of the result shows up later (a duplicate), it is refused
and the original record is untouched. Second, a late cancellation can't
bring the dish back — `timed_out` stays `timed_out`, and a result arriving
for an already-cancelled run is refused too. Third, stale news is ignored:
a result stamped with an old generation number, or a result for a dish that
isn't finished yet, changes nothing. "Correct" means the state machine
enforces all of this, and the operator's screen shows `timed_out` as its
own thing — clearly different from `failed` and from `cancelled`.

## What this task attempts

- **Goal:** drive the real experiment scheduler through deadline expiry and
  prove terminal-state discipline: at-most-once result recording, no
  resurrection, stale/duplicate rejection.
- **Mechanism:** the real `phlow_experiment::Scheduler`
  (`crates/phlow-experiment/src/control_plane.rs`); the gauntlet driver
  (`crates/phlow-gauntlet/src/tasks/task_20.rs`) admits nodes, publishes
  terminal results, cancels runs, and asserts states and errors.
- **Success criterion:** 4/4 scenarios pass and the CLI golden output shows
  `verdict=timed_out` with the verdict string distinct from `failed` and
  `cancelled`.
- **Non-goals:** a clock-watching executor — the `Scheduler` is pure
  invariant logic with no timer thread (stated honestly below); the driver
  models the host observing expiry and publishing the terminal `TimedOut`
  result, which is the host's job in the real system.

## What happened

Failed on the first run — all 4 tests — then passed after a one-line fix.
Final evidence: `double_publish_rejected_and_record_unchanged`,
`late_cancel_cannot_resurrect_and_late_result_rejected`,
`stale_generation_and_nonterminal_publish_rejected`, and
`cli_report_shows_timed_out_distinct_from_failed_and_cancelled` all pass;
the CLI prints `operator view: verdict=timed_out` alongside the explicit
`failed`/`cancelled` contrast lines.

## Where it went wrong

- **Stage:** first `cargo test -p phlow-gauntlet --test task_20` run —
  all 4 tests panicked before asserting anything.
- **Symptom:** every test died in the `late_cancel` phase with:
  `task-20: node admits: RunCancelled { run: "run-latecancel" }`
  (panic at `crates/phlow-gauntlet/src/tasks/task_20.rs:86`).
- **Evidence:** the phase first admitted `run-latecancel`, published its
  `TimedOut` result, then called `cancel_run("run-latecancel")` — which
  records that run id in the scheduler's cancelled set. The phase then
  tried to admit a *second* node under the *same* run id for the
  "live node, late result" subcase. `admit_node` checks the cancelled set
  first (`control_plane.rs:870`: `return Err(ExperimentError::RunCancelled
  { ... })`), so admission itself was refused before the subcase began.
- **Root cause:** my driver reused a cancelled run id for a node that was
  supposed to be live. The scheduler was behaving correctly — admitting
  work under a cancelled run *should* fail — and the test setup was wrong,
  not the code under test. Verified against `control_plane.rs:845-870`
  (`admit_node` docs and the `RunCancelled` guard), not guessed.

## The fix — what changed and why

- **Changed:** `crates/phlow-gauntlet/src/tasks/task_20.rs` — the "live
  node whose run is cancelled, then a late result arrives" subcase now
  admits its node under a fresh run id (`"late-result"`, i.e.
  `run-late-result`) instead of reusing `"latecancel"`, with a comment
  explaining why the ids must differ.
- **Commit:** pending (wave 4b).
- **Why:** the subcase needs a node that is genuinely live at admission
  time and only *becomes* cancelled afterward; reusing an already-cancelled
  run id tests admission-under-cancellation (a different, already-covered
  rejection) instead of late-result-after-cancellation. The alternative —
  clearing the cancelled set or skipping the admit guard — was rejected:
  the guard is the behavior under test, and weakening the driver to dodge
  it would prove nothing.
- **Source:** `crates/phlow-experiment/src/control_plane.rs:845-870`
  (`admit_node` rejects nodes for cancelled runs).
- **Validation agents:** `cargo test -p phlow-gauntlet --test task_20` —
  4/4 green after the fix; full package suite 108 passed, 0 failed.
- **Adversarial agents:** `double_publish` — second publication gets
  `DuplicateResult` and the stored digest/generation are byte-identical to
  the first (at-most-once, not last-write-wins);
  `stale_generation_and_nonterminal` — old-generation publish gets
  `StaleGeneration`, publish to a non-terminal node gets `NotTerminal`,
  neither mutates anything. Both passed; neither found a path to overwrite
  or resurrect a terminal record.
- **Citations:** publish guards — `control_plane.rs:893-933`
  (`NotTerminal` :901, `RunCancelled` :915, `StaleGeneration` :926,
  `DuplicateResult` :933); `cancel_run` — `:962`.

## Full technical depth

The `Scheduler` is a pure state machine over experiment nodes — there is
no clock, no timer thread, no background executor. Deadline expiry is
*observed by the host*, which then calls `publish_result` with
`NodeState::TimedOut`; the scheduler's job is making that terminal state
stick and refusing everything that would contradict it. The task models
exactly this division: the driver plays the host, the scheduler enforces
the invariants.

The state model (`control_plane.rs:65-105`): a node moves through
non-terminal states until a terminal one (`Succeeded`, `Failed`,
`TimedOut`, `Cancelled`, …) is published. `NodeState::TimedOut` renders as
`"timed_out"` (`:124`) — a distinct operator string from `"failed"` and
`"cancelled"`, which the CLI golden test asserts literally.

`publish_result` (`:893-933`) is a gauntlet of guards, checked in order:
node must exist and be non-terminal (`NotTerminal`, `:901`); the run must
not be cancelled (`RunCancelled`, `:915`); the generation must be current
(`StaleGeneration`, `:926`); and no result may already be recorded
(`DuplicateResult`, `:933`). The ordering matters: a duplicate publish to
a terminal node fails at the terminal check before it can even reach the
duplicate check — either way the stored record is never overwritten, and
the test asserts the digest and generation are unchanged, proving
at-most-once rather than last-write-wins.

`cancel_run` (`:962`) transitions live nodes of a run to cancelled and
returns the count transitioned; cancelling an already-terminal
(`timed_out`) run transitions 0 nodes and leaves the state alone — late
cancellation cannot resurrect. And a result arriving after cancellation is
refused with `RunCancelled`, keeping the node's cancelled state.

The honest limitation, stated plainly: because the scheduler has no
executor, this task does not test *detection* of expiry (no clock to race,
no timeout to fire). It tests what happens *once expiry is observed* —
the terminal-state discipline that makes at-most-once true. Detection
belongs to the host layer, which is out of scope for this task by design.

## Sources

- Primary: `~/workspace/repos/phlow/crates/phlow-experiment/src/control_plane.rs`
  - `:65-105` — `NodeState` enum and terminal-state classification
  - `:124` — `TimedOut => "timed_out"` operator string
  - `:845-870` — `admit_node`, including the `RunCancelled` guard
  - `:886-933` — `publish_result` and its four guards
    (`NotTerminal` :901, `RunCancelled` :915, `StaleGeneration` :926,
    `DuplicateResult` :933)
  - `:962` — `cancel_run` transition counting
- Task code: `crates/phlow-gauntlet/src/tasks/task_20.rs`,
  `crates/phlow-gauntlet/tests/task_20.rs`
