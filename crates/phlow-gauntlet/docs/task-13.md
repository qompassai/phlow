# task-13: scheduler overload

**Kind:** rust · **Status:** pass · **Wave:** 3 · **Commits:** none (no commit per task brief)

## ELI5

A scheduler is the part of a system that decides which work gets in and in
what order. When too much work arrives at once — a "storm" — a good
scheduler does three things: it keeps its waiting line from growing without
limit (bounded queue), it says "no" out loud instead of quietly losing work
(explicit rejection), and when the storm passes it goes back to normal
(recovers). This task checks phlow's scheduler does all three. The twist:
phlow's scheduler doesn't actually *run* anything — it is the bouncer at
the door, not the workers inside. So the overload test storms the door:
160 submissions against 16 slots, plus five repeated waves, and checks the
bouncer counts every single one, rejects the excess with a named error,
never lets the line exceed 16, and works normally again afterwards.

## What this task attempts

- **Goal:** prove phlow's scheduler degrades gracefully under overload —
  bounded queue, explicit rejection (no silent drops), no panics or stuck
  state, full recovery when load subsides.
- **Mechanism:** `crates/phlow-experiment/src/control_plane.rs` —
  `Scheduler::new` / `Scheduler::admit` / `Scheduler::publish_result` /
  `Scheduler::queue_len`, driven by a scenario binary that compiles phlow's
  *own* `error.rs` + `control_plane.rs` via `#[path]` (the real sources, no
  mocks). The driver is `crates/phlow-gauntlet/src/tasks/task_13.rs`; the
  integration tests are `crates/phlow-gauntlet/tests/task_13.rs`.
- **Success criterion:** all four cases pass — normal load completes,
  burst at exactly capacity is absorbed, 10x overload is shed with explicit
  `QueueFull` errors and a conservation identity
  (admitted + rejected == submitted), and sustained overload drains to a
  responsive scheduler.
- **Non-goals:** there is no executing scheduler in phlow to soak with
  threads — `workers_max` is an admission limit, not a thread pool — so
  this task does not (and cannot) test concurrent execution, worker crash
  recovery, or scheduling latency under CPU contention. Those would need an
  executor that does not exist yet.

## What happened

Pass on the mechanism's first attempt (iteration 2 overall — see below).
All four overload cases pass against the real scheduler sources:

- `normal_load`: 8/8 admitted, queue peak 8, all 8 published exactly once,
  queue drained to 0. Avg admit latency 991 ns.
- `burst_at_capacity`: 16/16 admitted at exactly capacity 16, peak 16,
  drained to 0. Avg admit 931 ns.
- `overload_10x`: 160 submissions → 16 admitted, 144 rejected with explicit
  `ExperimentError::QueueFull`, 0 other error kinds, 160/160 accounted
  (conservation holds — nothing silently dropped). Queue peak 16, never
  above capacity. All 16 admitted nodes still retrievable and still
  `Admitted` after the storm. RSS 2208 → 2292 KiB across the storm
  (+84 KiB; the queue itself is bounded, growth is allocator noise).
- `sustained_overload`: 5 rounds × 32 submissions → 80 admitted,
  80 explicitly rejected, peak queue 16, whole storm 0 ms; after the load
  subsided a probe node admitted and published normally — queue empty,
  scheduler fully responsive.

Gates: `cargo fmt -p phlow-gauntlet -- --check` clean,
`cargo clippy -p phlow-gauntlet --all-targets -- -D warnings` zero
warnings, `cargo test -p phlow-gauntlet --test task_13` 6/6 green.

## Where it went wrong

- **Stage:** iteration 1 — my own integration-test helper, not the task
  mechanism.
- **Symptom:** 4 of 6 tests failed with
  `case must produce a verdict: Spawn { what: "normal_load", detail: "Not a directory (os error 20)" }`.
- **Evidence:** the helper `build_harness()` returned the *binary* path
  from `ensure_scenario_binary()`, but each test then joined `"scenario"`
  onto it again, producing `<binary-file>/scenario` — spawning a path
  through a regular file yields ENOTDIR.
- **Root cause:** misnamed helper return (I treated the binary path as the
  work dir). The driver path (`run()`) was unaffected — its end-to-end
  test passed on the same run, confirming the scenario binary itself
  compiled and ran correctly.
- **Fix:** `build_harness()` now returns the binary path directly; the four
  case tests use it as-is. One-line semantic fix, verified by 6/6 green on
  iteration 2. No scheduler or driver code changed.

## The fix — what changed and why

No fix to the scheduler or the driver was needed: phlow's admission
control already implements the required overload behavior (bounded queue +
explicit `QueueFull`), and the task's job was to verify it honestly rather
than to change it. The only fix in this task's history is the test-helper
path bug above.

- **Changed:** `crates/phlow-gauntlet/tests/task_13.rs` — `build_harness()`
  returns the binary path; call sites use it directly.
- **Commit:** none (no commit per task brief).
- **Why:** the double-join produced a non-existent executable path; using
  the path `ensure_scenario_binary()` already returns removes the
  duplication. The alternative (returning the work dir and joining in each
  test) was rejected because it re-derives a path the builder already
  knows — one source of truth for the binary location.
- **Source:** Rust `std::process::Command` semantics — spawning a path
  whose prefix component is a file fails with ENOTDIR ("Not a directory",
  os error 20); the Rust standard library docs for `Command::new` note the
  program path must resolve to an executable.
- **Validation agents:** the 6 integration tests themselves (4 case tests +
  driver end-to-end + metadata), plus the three gates above.
- **Adversarial agents:** the two adversarial cases *are* the red team —
  the 10x storm asserts the conservation identity
  (admitted + rejected == submitted) that would catch any silent drop, and
  the sustained case asserts post-storm responsiveness via a probe
  admit+publish. Additionally the scenario asserts every admitted id still
  resolves with state `Admitted` after the storm (no corruption).
- **New convention (if any):** when a gauntlet task cannot add a crate
  dependency (disjoint file ownership), drive the target crate's real
  sources by compiling a scenario binary with `rustc` and `#[path]`
  includes — but only when the target files import nothing beyond `std`
  (verified by inspection: `control_plane.rs` imports `crate::error` +
  `std::collections`; `error.rs` imports `std::fmt`). This keeps the test
  honest (real code, no copies) without touching `Cargo.toml`. Evidence:
  this task's 6/6 green run.
- **Citations:** `crates/phlow-experiment/src/lib.rs` ("no scheduler
  execution"); `crates/phlow-experiment/src/control_plane.rs` (Scheduler:
  "spawns nothing, runs nothing, and holds no threads"; `admit` rejects
  with `QueueFull`); `crates/phlow-experiment/src/error.rs`
  (`QueueFull` display: "scheduler queue is full at {capacity} nodes; node
  rejected").

## Full technical depth

**The recon.** The task brief warned that `phlow-experiment` is
"scaffolding with NO scheduler execution" and told me to find the real
scheduler or document its absence. I searched all 24 crates:
`tokio::spawn`/`JoinSet` appear nowhere in scheduler-adjacent code (the
only spawns are a msgpack-transport worker with a capacity-1 channel in
`phlow-runtime/src/transport/msgpack.rs`, scoped thread pools in
`phlow-inference`/`phlow-checks`, and process spawns — none of them a
scheduler). The crate's own `lib.rs` line 10 says it enables "no runtime
concurrency, no scheduler execution". So: **no executing scheduler exists
in phlow @ 87c182d**, and the brief's caution is correct — there is no
thread pool to soak.

**What does exist.** `control_plane::Scheduler` is the invariant core a
future async executor must preserve: a `BTreeMap<NodeId, SchedulerNode>`
plus a `VecDeque<NodeId>` admission queue. `admit()` validates in order —
state is `Proposed`, generation within `depth_max`, **queue length below
`queue_capacity` else `Err(QueueFull { capacity })`**, no duplicate id, run
not cancelled — then flips the node to `Admitted` and pushes it. The queue
can therefore never exceed `queue_capacity` through the public API; that
is the bound this task overloads. `publish_result()` enforces at-most-once
publication (generation match, no double publish, terminal state required)
and removes the node from the queue, which is the drain path.
`cancel_run()` drains a run's queued nodes and records the run id
(bounded at `CANCELLED_RUNS_MAX` = 1024) so late results are rejected.

**How the driver reaches it.** `phlow-gauntlet` depends only on
`serde_json`, and the task brief forbids touching `Cargo.toml`, so the
driver cannot `use phlow_experiment::...`. Instead
`ensure_scenario_binary()` writes a scenario program into the task work
dir and compiles it with `rustc --edition=2024 -O`, with the two source
files pulled in as

```rust
#[path = "<abs>/crates/phlow-experiment/src/error.rs"]
mod error;
#[path = "<abs>/crates/phlow-experiment/src/control_plane.rs"]
mod control_plane;
```

`control_plane.rs`'s `use crate::error::ExperimentError` resolves to the
scenario crate's own `mod error` — the real code, compiled as-is. Both
files were verified to import only `std`, so no dependency resolution is
needed. A live recon guard (`recon()`) re-reads `lib.rs` and
`control_plane.rs` on every run, asserts the "no scheduler execution" /
"spawns nothing, runs nothing" statements are still present, and scans all
`phlow-experiment` sources for `tokio::` / `thread::spawn` — if the crate
ever gains an executor, the task fails closed instead of passing on a
stale premise. Children (rustc, scenario) run under bounded wall-clocks
(90 s compile, 30 s per case) with poll-and-kill, mirroring the gauntlet's
nvim-driver pattern; output is capped at 1 MiB.

**The four cases** (each a separate scenario argv; JSON verdict per case):

1. `normal_load` (V): 8 nodes (< 16 capacity) → all admitted, avg admit
   991 ns, queue peak 8; all 8 published with `Succeeded`; `queue_len()==0`,
   `published_count()==8`.
2. `burst_at_capacity` (V): exactly 16 submissions → 16 admitted, peak 16,
   drained to 0.
3. `overload_10x` (A): 160 submissions → 16 admitted, 144
   `QueueFull`, 0 other errors; conservation identity 16+144=160;
   `queue_peak` never exceeded 16; every admitted id still resolves via
   `node()` with state `Admitted`; drain → 0. RSS 2208→2292 KiB.
4. `sustained_overload` (A): 5 rounds × 32 submissions with a drain per
   round → 80 admitted, 80 `QueueFull`, peak 16, 0 ms total; then a probe
   node admits and publishes — the scheduler is responsive, nothing stuck.

**What "graceful" means here, concretely.** Bounded: `queue_peak ≤ 16` in
every case including the storm. Explicit: every non-admitted submission
carries a typed `QueueFull` error — the conservation identity proves no
silent drop (a dropped submission would break admitted+rejected=160).
No panics: the scenario is pure logic and any panic would surface as a
non-zero exit → driver `Fail`. No deadlock: single-threaded logic plus
driver-side timeouts. Recovery: post-storm probe admit+publish succeeds
and `queue_len()==0`.

**Limits of this verification.** This proves the *admission boundary*
degrades gracefully. It says nothing about a future executor's behavior
under load (worker starvation, priority inversion, deadline handling) —
that executor does not exist yet, and this task must not be read as
covering it.

## Sources

- Primary: `crates/phlow-experiment/src/control_plane.rs` — `Scheduler`
  struct and docs ("spawns nothing, runs nothing, and holds no threads"),
  `Scheduler::admit` (queue-full check order), `Scheduler::publish_result`,
  `SchedulerLimits` (`queue_capacity`, `QUEUE_CAPACITY_DEFAULT = 16`).
- Primary: `crates/phlow-experiment/src/error.rs` —
  `ExperimentError::QueueFull` and its `Display` impl.
- Primary: `crates/phlow-experiment/src/lib.rs` — "no runtime
  concurrency, no scheduler execution".
- Primary: `crates/phlow-gauntlet/docs/00-design.md` — task-13 row:
  "submissions beyond `QUEUE_CAPACITY` → bounded rejection; scheduler
  state stays consistent".
- Secondary: Rust `std::process::Command` docs (spawn path resolution;
  ENOTDIR on a file prefix component) — cited for the test-helper fix.
