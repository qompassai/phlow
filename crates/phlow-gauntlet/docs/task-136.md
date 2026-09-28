# task-136: target queueing with bounded concurrency

**Kind:** rust · **Status:** pass · **Wave:** 24

## ELI5

Imagine a bug-bounty program with 10 websites to test but only 3
testers. The rule is simple: never have more than 3 tests running at
the same time. This task checks that the scheduler — the part of the
machine that hands out work — actually enforces that rule, not just on
a good day but always: with 3 slots it never runs 4 probes at once and
finishes all 10 targets; with 1 slot everything runs one after another;
if a worker silently dies, its slot is freed and the queue keeps
moving; and asking for 0 slots is refused outright instead of being
read as "no limit".

## What this task attempts

- **Goal:** pin the scheduler's concurrency bound as structural, not advisory.
- **Mechanism:** `Scheduler::tick` in `crates/phlow-gauntlet/src/bounty/sched.rs`, driven by a scripted probe-cycle simulator in `src/tasks/task_136.rs` (MOCK: `ManualClock`, fixed probe durations, scripted crash).
- **Success criterion:** V1 peak concurrent == 3 with 10/10 finished; V2 peak == 1; A1 crashed run Failed by the heartbeat watchdog, slot released at exactly `timeout + 1` ticks, queue drained; A2 `Scheduler::new(k=0)` refused at construction.
- **Non-goals:** real subprocesses (task 137's job), testing windows and rate limits (task 140's job), finding dedup (task 139's job).

## What happened

First attempt, all four cases pass. V1: K=3 over 10 targets with 2-tick
probes — peak concurrent exactly 3, 10 finished, 0 failed, drained in 8
ticks. V2: K=1 — peak exactly 1, 6/6 finished (strictly serial). A1:
the t00 worker crashes at launch (never progresses, never heartbeats);
the watchdog marks its run `Failed` with reason `heartbeat-timeout` at
tick 5 — exactly `HEARTBEAT_TIMEOUT_TICKS (3) + 1` after its last
heartbeat — releases the scheduler slot, and t03 launches on that same
tick; the queue drains (3 finished, 1 failed, peak stayed ≤ 2). A2:
`Scheduler::new(k=0)` returns `Err("bounty: max_concurrent must be >= 1")`
— fail closed, zero never means unlimited.

## Full technical depth

The bound lives in `Scheduler::tick`: `while self.in_flight < self.max_concurrent`
emits `Launch` actions, and `in_flight` only moves on `note_run_finished`.
The driver simulator keeps its own running set in lockstep with the
scheduler's count (finish → `set_state(Finished)` + `note_run_finished`),
so any drift between the two would show up as a peak violation — it
doesn't. The heartbeat watchdog is driver-side (the scaffold has no
liveness concept): each tick, runs silent longer than
`HEARTBEAT_TIMEOUT_TICKS` are failed and their slot released via
`note_run_finished`. The measured release tick (last heartbeat + 4) is
the exact consequence of the strict `>` comparison in the sweep, which
the test pins. The K=0 refusal is the scaffold's own constructor guard.

## Sources

- Primary: `crates/phlow-gauntlet/src/bounty/sched.rs` (`Scheduler::tick`, `Scheduler::new`).
- Bounded-executor pattern: `tokio::sync::Semaphore` documentation (a semaphore with guaranteed release is the same structural bound); heartbeat supervision mirrors Erlang/OTP supervisor trees (liveness via heartbeats, failed workers restarted/replaced).
