# task-47: file descriptor exhaustion

**Kind:** rust · **Status:** pass (with a documented API-shape caveat — see below) · **Wave:** 46–50 · **Commits:** pending (wave 46-50)

## ELI5

Every program the agent launches costs the operating system a
handful of "handles" (file descriptors): one for the child's
input, one for its output, one for its error stream, and more
while it runs. If the agent launches thousands of programs at
once, it can run the machine out of handles — the classic
`EMFILE` ("too many open files") crash. The defense is a bouncer
at the door: only N programs may run at once, only M may wait in
line, and anyone past the line is turned away *politely* — with a
written note saying "you were dropped", not a crash.

phlow has exactly this bouncer in
`phlow-tuios/src/hooks.rs`: `HookManager` guards its subprocess
fan-out with two counting semaphores — a queue semaphore
(`HOOK_QUEUED_MAX` = 64, fires wait in line) and a run semaphore
(`HOOK_CONCURRENT_MAX` = 8, children actually running). Both are
acquired *before* the process spawns; a fire that finds the queue
full never spawns and is recorded as a `HookOutcome` with
`dropped: true` — the polite written note. The driver fires
10,000 hooks at the real manager while sampling `/proc` for live
children and open FDs: at most 8 children ever overlap, FDs stay
near baseline, excess fires shed as recorded drops.

## What this task attempts

- **Goal:** run the design's file-descriptor-exhaustion scenarios
  against the real hook fan-out: normal fan-out under the cap, a
  10,000-fire storm, crashing tools that must not leak permits,
  and a workspace-wide audit proving no spawn path bypasses the
  limiter.
- **Mechanism:** the `task_47.rs` driver registers `/bin/sleep`
  hooks on the real `HookManager`, fires them (including
  fire-and-forget storms from multiple threads), and samples
  `/proc/self/fd` and `/proc` process counts. The spawn audit
  walks every non-gauntlet crate's `src` tree for `Command::new`
  and classifies each site.
- **Success criterion:** concurrent children never exceed
  `HOOK_CONCURRENT_MAX`, FDs stay within baseline + headroom,
  no spawn errors under the storm, permits return to baseline
  after crashes, and every spawn path is accounted for.
- **Non-goals:** proving anything about a real operator's
  machine — the measurement is this VM's `/proc`, Linux-only.

## What happened

Pass — on the first and only attempt. All four cases hold:

- `normal_fanout_runs` (V): five registered hooks all complete
  cleanly, zero dropped, zero errors.
- `spawn_paths_go_through_limiter` (V): the workspace walk finds
  exactly three spawn sites — the permit-gated hook fan-out, the
  synchronous one-child-at-a-time checks runner (`spawn_pinned`
  in `phlow-checks/src/runner.rs`: one child per call, deadline
  wait, process-group kill, reap — no queue exists to bound), and
  one CLI spawn verified mechanically test-only (the version
  cross-check `#[test]` in `phlow-cli/src/lib.rs`). No other
  acquisition path exists; a new spawn site fails this case.
- `storm_10000_spawns_capped` (A): 10,000 fires; the run
  semaphore caps concurrent children at 8, FDs stay within
  baseline + 256, zero spawn errors, excess fires shed as
  recorded `dropped: true` outcomes. `fire` never blocks — the
  storm returns immediately and the shedding is visible in the
  outcomes log.
- `crashed_tools_return_to_baseline` (A): sixteen crashing tools
  (`/bin/false`, exit 1, plus a signal-killed one) under a full
  queue — every child reaped (no zombies), permits return to
  baseline, the manager accepts new work afterward.

## The fix — what changed and why

Nothing in the product changed. One honest caveat is documented
rather than hidden: the design sketches a typed `TooManyHandles`
error surfaced to the producer. The real API expresses the same
rejection as a recorded `HookOutcome { dropped: true }` in the
outcomes log — explicit and inspectable, never a silent drop and
never EMFILE — but the *shape* differs from the design's sketch.
The driver records the verdict as a pass with this deviation
stated, because the load-shedding is real, bounded, and
observable; the difference is API shape, not missing protection.

## Full technical depth

The ordering inside `fire` is the whole argument: queue-permit
`acquire` → (on failure) record dropped outcome and return →
run-permit `try_acquire_owned` → (on failure) release queue
slot, record dropped, return → `spawn_hook` (the single
`Command::new`, with both permits held) → reap on every path
(`wait`, timeout kill, drop guard). Permits are RAII: a crashing
tool cannot leak them, and the A2 case proves the manager
returns to baseline.

The audit's test-only classification is mechanical, not
hand-waved: the driver walks up from the `Command::new` line to
the enclosing `fn` and requires a `#[test]` attribute — so a
future production spawn in that file would fail the case.

Test-hygiene note: the storm case measures process-global
`/proc` counts, so the three subprocess-driving tests in
`tests/task_47.rs` serialize on a static mutex. That lock is in
the test file, not the product — each case still drives its own
manager with its own semaphore pair.

Banked for Matt (product decision, NOT auto-implemented):
whether the dropped-outcome shape should also surface as a
typed `TooManyHandles` error to the producer, matching the
design sketch's API more literally. The protection exists; the
question is only API shape.

## Sources

- Primary: `crates/phlow-tuios/src/hooks.rs` (`HookManager`,
  `HOOK_QUEUED_MAX`, `HOOK_CONCURRENT_MAX`, `HookOutcome`,
  `fire`, `spawn_hook`).
- Primary: `crates/phlow-checks/src/runner.rs`
  (`spawn_pinned`: synchronous single child).
- Primary: `crates/phlow-cli/src/lib.rs` (test-only spawn,
  verified by attribute).
- Driver: `crates/phlow-gauntlet/src/tasks/task_47.rs` (real
  manager, `/proc` sampling, source-walk audit).
- Tests: `crates/phlow-gauntlet/tests/task_47.rs` (2V/2A).
