# phlow-gpu-worker: placement and adaptation decisions

## Why this crate exists

`phlow-compute-cuda` defines *how* to talk to a device; it says nothing
about *who* is allowed to launch, in what order, or how a launch is
cancelled. `phlow-gpu-worker` owns that lifecycle: one job at a time, an
explicit `Idle / Busy / Stopped` state machine, and generation-based
cancellation. Splitting dispatch from the backend trait keeps
`phlow-compute-cuda` dependency-free of lifecycle policy, and keeps the
worker testable against the deterministic simulator.

## What was adapted (from NVlabs cuda-oxide host runtimes, Apache-2.0 — concepts only)

- Kernel launches go through a host-side worker rather than being fired
  directly at the device. (cuda-oxide's `cuda-core` / `cuda-async`
  runtimes)
- Cancellation invalidates outstanding work instead of merely forgetting
  it: every `cancel`/`stop` bumps a generation, and stale handles are
  rejected with typed errors.
- Backend failures surface to the caller with the worker left in a known
  state (idle); no job id is consumed by a failed submit.

No cuda-oxide code was ported; only the host-runtime shape was adapted.

## Deliberately excluded

- **Threads and async.** The worker is synchronous and single-threaded.
  There is no background executor, no `tokio` dependency, and no queue —
  one job at a time is the whole scheduling policy. A concurrent
  dispatcher would be a new crate, not a change to this state machine.
- **Streams, events, and priorities.** Real driver runtimes multiplex
  work across streams; this worker models the ownership discipline, not
  the scheduling.
- **Preemption.** `cancel` drops the pending result and invalidates
  handles; it does not interrupt a running kernel (the simulator runs to
  completion synchronously).

## License basis

cuda-oxide is Apache-2.0. Only its concepts were adapted; no upstream
code, text, or API surface was copied. This crate is original work.
