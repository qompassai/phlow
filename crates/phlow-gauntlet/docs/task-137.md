# task-137: cancellation mid-run with no zombies

**Kind:** rust · **Status:** pass · **Wave:** 24

## ELI5

When you cancel a running security test, two things must happen: the
test stops, and no ghost of it keeps running. On Linux, a stopped
program whose parent never collected its exit status becomes a
"zombie" — dead but still listed. This task checks the cancel button
does the full job: remove a waiting test from the queue without ever
starting it; for a running test, kill the real process and then
collect it (reap it), verified by checking the operating system
itself — after cancel, the process ID no longer exists. Cancelling
something already finished is politely refused with a typed error, and
cancelling twice still only kills once.

## What this task attempts

- **Goal:** the cancel path (queue removal + process kill + ledger update) leaves no orphaned or zombie processes, verified at the OS level.
- **Mechanism:** a driver-local `ProbeManager` in `src/tasks/task_137.rs` owning real `sleep` children (`std::process::Child`), the scaffold's `TargetQueue` and `RunLedger`; zombie checks via `/proc/<pid>/stat` state parsing.
- **Success criterion:** V1 queued cancel dequeues with zero processes spawned; V2 running cancel kills + reaps, `/proc/<pid>` absent afterward; A1 finished cancel → typed `CancelError::AlreadyTerminal`, kill count unchanged; A2 double cancel → one kill, one reap, one ledger `Cancelled` entry.
- **Non-goals:** revocation policy (task 133's job), concurrency bounds (task 136's job).

## What happened

First attempt, all four cases pass against real `sleep 600` children.
V1: cancelling queued t01 dequeues it — spawned/killed/reaped all 0,
run `Cancelled` with reason `operator-cancel`. V2: cancelling the
running probe kills and reaps the child; the transient zombie state
(`Z` in `/proc/<pid>/stat`) is observed between kill and wait on the
fast path, and afterward the PID has no `/proc` entry — no zombie
remains. A1: cancelling the finished run returns
`Err(AlreadyTerminal { run_id })`; kill count unchanged; ledger still
`Finished`. A2: double cancel — first kills, second returns
`AlreadyTerminal`; killed == 1, reaped == 1, exactly one ledger
`Cancelled` entry. A `Drop` backstop reaps any straggler so a failed
assertion mid-case cannot leak a zombie.

## Full technical depth

The mechanism is kill-then-reap ordering. POSIX defines a zombie as a
child that has terminated but whose parent has not yet waited for it;
only `wait`/`waitpid` reaps it (IEEE Std 1003.1, `wait`). Rust's
`Child::kill` sends SIGKILL and `Child::wait` reaps — the driver does
them back-to-back with nothing in between that could lose the child,
because the `ProbeManager` owns every `Child` from spawn (in `children:
HashMap<String, Child>`) and removes it exactly once, either on the
cancel path or on scripted completion. The `/proc/<pid>/stat` parser
finds the state character after the *last* `)` because the comm field
itself may contain parentheses and spaces. The typed
`CancelError::{AlreadyTerminal, UnknownRun, Os}` keeps domain refusals
separate from OS failures. The 200-round 1ms poll for the `Z` state is
bounded evidence of the ordering, not load-bearing: the load-bearing
assertion is the post-reap absence of `/proc/<pid>`.

## Sources

- Primary: IEEE Std 1003.1-2017, `wait` ("a zombie process ... shall be removed" only after the parent waits); Rust `std::process::Child::{kill, wait, id}` documentation; Linux `proc_pid_stat(5)` man page (process state codes, comm field format).
