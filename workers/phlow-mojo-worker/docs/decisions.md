# phlow-mojo-worker design decisions

Lane `lane/mojo`, 2026-09-27. Adaptation of Mojo worker concepts into Tiger
Style Rust: a Rust-side harness contract plus the interface boundary a
Mojo-implemented worker must satisfy. Concepts are re-expressed, never
ported: no Mojo code was translated and no Mojo compiles in this workspace.
Upstream facts were verified against primary sources on 2026-09-27.

## 1. Upstream sources

- **modular/modular** (https://github.com/modular/modular): Mojo compiler
  (`/Mojo`), standard library (`/Mojo/stdlib`), MAX accelerator library
  (`/max/kernels`); Apache 2.0 with LLVM Exceptions; Mojo open source
  since August 2026, 1.0 shipped with Modular 26.5.
- **Mojo docs** (https://docs.modular.com/mojo, v1.1.0): the docs index
  lists the GPU programming track, the Mojo manual, and a Python-interop
  page (Mojo calls Python modules; Python calls explicitly declared Mojo
  bindings — the mechanism a real Mojo worker would expose its entry
  points through).
- **Ownership model**: Mojo uses a Rust-inspired value-ownership model
  (official proposal
  https://github.com/modular/modular/blob/HEAD/Mojo/proposals/value-ownership.md
  documents the `^` move-transfer operator; the manual documents the
  `owned`/`inout`/borrow argument conventions). This maps directly onto
  the harness/worker split below: the harness owns task payloads, ids,
  and results; the worker owns only its internal state; `poll` borrows
  the task and must not retain it.
- **Async launch model** (https://docs.modular.com/mojo/manual/gpu/intro-tutorial.md,
  fetched verbatim): `enqueue_function` queues work asynchronously and
  `synchronize()` drains the queue. The harness mirrors the split:
  `submit` admits, `pump` drives progress, `take_result` collects.

## 2. Crate placement: `workers/phlow-mojo-worker` (top-level)

Not under `crates/`: the fifteen crates there form phlow's shared safe
runtime. A Mojo worker is an experimental *plugin* — a worker
implementation behind an interface boundary — not core runtime. The
top-level `workers/` directory names that role and keeps the experimental
surface out of the audited core tree. Added to the root workspace
`members`.

## 3. What is real vs. interface (read this first)

- **Real** (implemented, tested): `WorkerHarness` — lifecycle state
  machine (`Stopped`/`Running`/`Draining`), bounded intake queue with
  reject-on-full, harness-assigned `TaskId`s, generation tokens,
  in-flight map, bounded result queue with counted oldest-drop, and the
  cancellation policy. Plus `WorkerConfig` validation and the typed
  `WorkerTask` / `WorkerResult` / `TaskStatus` / `WorkerError`.
- **Interface** (boundary, not implementation): the `MojoWorker` trait
  (`init`, `poll`, `cancel`, `shutdown`) plus the documented Mojo-side
  obligations in `boundary.rs` (bounded poll quantum, prompt
  cancellation, output within `RESULT_BYTES_MAX`, no payload retention,
  idempotent shutdown). A real Mojo worker would expose these entry
  points through Mojo's declared bindings; a thin Rust adapter would
  implement the trait over them. Nothing here compiles or runs Mojo.
- **Test double**: `SimulatedMojoWorker` (`simulated` feature, on by
  default) implements the trait in ordinary Rust — bounded polls,
  per-task pending counts, prompt cancellation, deterministic
  byte-transform output. For tests and interface experiments only.

## 4. Key design choices

- **Poll-driven, not thread-driven.** The harness drives the worker with
  `pump`; the worker never spawns threads the harness must reap. One
  bounded step per `poll` keeps progress explicit and cancellation
  prompt, mirroring Mojo's enqueue/drain split without inventing a
  threading model for the Mojo side.
- **Generation tokens.** `restart` drains, bumps the generation, and
  re-inits. Results publish only when task id *and* generation match;
  anything else is dropped and counted (`stale_results_dropped`). A
  restart can never resurrect an old result — the drain empties the
  in-flight map *and* the publish path re-checks.
- **Cancellation cannot duplicate or resurrect.** Queued cancels publish
  one `Cancelled` result immediately; in-flight cancels mark and notify,
  publishing on the next pump; a cancelled task's `Ready` is dropped in
  favor of `Cancelled`; cancelling a finished task is `UnknownTask`.
- **Two full-queue policies, both visible.** Intake rejects
  (`QueueFull`, nothing admitted). Results replace-oldest with a
  `results_dropped` counter — results are retrievable, so the loss is
  observable, never silent.
- **Drain always terminates.** `DRAIN_ROUNDS_MAX` (4096) no-progress
  rounds, then leftovers are force-cancelled with `Cancelled` results;
  the worker is shut down and the harness returns to `Stopped` even when
  a poll fails mid-drain (the error is still reported).
- **Task kinds are a closed enum** (`Embed`, `Score`, `Transform`): the
  vectorizable offload shapes that motivate a Mojo worker. Adding a kind
  is a contract change, made visible by the compiler.

## 5. Deliberate non-goals

No threads, no async runtime, no subprocess supervision, no real Mojo FFI
(the boundary is a trait by design — the Mojo side does not exist yet),
no model loading, no device management. The task deadline in
`WorkerConfig` is advisory metadata a real Mojo worker would enforce.

## 6. Bounds

`PAYLOAD_BYTES_MAX` (1 MiB), `RESULT_BYTES_MAX` (4 MiB),
`QUEUE_TASKS_MAX` (1024), `REASON_CHARS_MAX` (256),
`TASK_DEADLINE_MAX` (24 h), `RESULTS_MAX` (64),
`PUMP_TASKS_MAX` (64), `DRAIN_ROUNDS_MAX` (4096),
`PENDING_POLLS_MAX` (1024).

## 7. Test inventory

20 tests in `tests/worker.rs`: **10 validation / 10 adversarial** (50/50).

Validation: start→drain lifecycle with worker shutdown observed;
submit accepted while running; result matches task id + generation with
the deterministic transform output; cancel of a queued task publishes
`Cancelled`; full queue rejects with typed `QueueFull`; drain processes
3 tasks then stops; restart bumps the generation; payload at the exact
byte limit accepted; simulated worker with 2 pending polls completes on
the third pump; init failure leaves the harness stopped.

Adversarial: submit while stopped; oversized payload rejected before
queue admission (queue untouched); double cancel (second is
`UnknownTask`, exactly one result); cancel of a completed task refused
(no resurrection); forged-generation result from a dishonest worker
dropped as stale and counted; poll failure keeps the task queued with
nothing published (retry succeeds); 65 results into 64 slots drops the
oldest with a visible count; double start rejected; in-flight cancel
with a slow worker publishes `Cancelled`; zero queue capacity / zero
payload budget / zero deadline rejected at config construction.
