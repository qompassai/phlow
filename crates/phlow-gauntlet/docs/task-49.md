# task-49: CPU fairness

**Kind:** rust · **Status:** fail (seam absent — no worker pool, no preemption, no timeslices, no task-tree fairness; banked product decision) · **Wave:** 46–50 · **Commits:** pending (wave 46-50)

## ELI5

When many jobs share one machine, a fair scheduler gives each
job its turn: round-robin, so no job can hog the CPU forever
while others starve. The design's weapon is the bully job — a
runaway worker that never yields — and the test is whether the
scheduler keeps the other jobs' progress fair anyway. It also
asks for fairness down the tree: a parent task's children
should share fairly too.

phlow has no fair scheduler to test. There is no worker pool, no
preemption, no timeslices or quanta, no round-robin policy, and
no task-tree fairness mechanism anywhere in the Rust crates.
The closest scheduler-shaped API is `phlow-experiment::Scheduler`
— but it is an *admission* scheduler (it decides which
experiment nodes may run, topologically), not an *executing*
scheduler; it never runs tasks, so it cannot be fair or unfair
to them. The only async task machinery in the tree is the
msgpack transport's single current-thread worker — one thread,
not a pool, so the fairness question does not arise there. The
design's bully job has no scheduler to bully.

## What this task attempts

- **Goal:** locate the fair task scheduler in phlow's Rust
  crates and run the design's scenarios: a runaway worker that
  never yields (progress stays fair across workers), a full task
  tree under load (children share fairly), and the scheduler's
  own admission path.
- **Mechanism:** the `task_49.rs` driver probes the live working
  tree — runtime tokenized vocabulary scans over every
  non-gauntlet `crates/*/src/**/*.rs` for scheduling-discipline
  tokens (`preempt`, `timeslice`, `time_slice`, `fairness`,
  `quantum`), worker-pool tokens (`worker_pool`, `thread_pool`,
  `task_pool`), and actual code *use* of `tokio` (`use tokio`
  or `tokio::` paths — prose mentions don't count). `quantum`
  hits are classified by hand: every one is the post-quantum
  cryptography term (ML-DSA-65 / ML-KEM), not a scheduling
  quantum.
- **Success criterion:** a scheduler exists with a named
  fairness policy, the bully job cannot starve others, and
  tree-level fairness holds.
- **Non-goals:** building the scheduler. Whether phlow needs a
  fair task executor — as opposed to the current protections,
  which are deadlines and concurrency bounds, not scheduling
  fairness — is banked for Matt as a product decision, not
  auto-implemented.

## What happened

Fail at `"seam"` — on the first and only attempt, honestly.
There is no scheduler to be fair:

- `no_scheduling_discipline_tokens` (V): zero scheduling-
  discipline tokens across the workspace — no preemption, no
  timeslices, no fairness machinery named anywhere. The
  `quantum` hits are all cryptography (post-quantum signatures/
  KEX), classified line by line.
- `no_worker_pool_to_schedule` (V): zero worker-pool tokens;
  the only real `tokio` code use is the msgpack transport's
  single current-thread worker (the task-25 channel) — one
  thread, not a pool, so no fairness question arises. Every
  other `tokio` mention is doc prose explicitly saying async is
  future work.
- `runaway_never_yields_has_no_target` (A): with no preemption,
  no quanta, and no pool, there is no scheduler to starve and no
  mechanism whose fairness could be measured. The case documents
  the missing target rather than inventing a timing measurement.
- `runaway_tree_fairness_has_no_target` (A): no task-tree
  primitives exist to attach tree fairness to.

The probe is careful about what "fairness" would *not* be: the
deadline and budget APIs adjacent to this design (timeouts,
queue bounds) have no fairness semantics — a fair policy cannot
be bolted onto a deadline, and the driver says so explicitly
rather than letting a deadline masquerade as a scheduler.

## The fix — what changed and why

Nothing changed: the seam is absent, so there is nothing to fix
without a product decision. The driver is new in this wave
(`src/tasks/task_49.rs`), plus four integration tests
(`tests/task_49.rs`). No production code was touched. (The
probe excludes the whole `phlow-gauntlet` directory from its
scans: other task drivers mention `tokio` and fairness-adjacent
words, and the system under test is the product, not the
gauntlet.)

## Full technical depth

Two classification details matter for trusting the zero:

1. `quantum` is genuinely ambiguous in this repo — it ships
   post-quantum cryptography (ML-DSA-65, ML-KEM, hybrid KEX).
   The driver reads each hit line and requires a crypto marker
   (`quantum-proof`, `post-quantum`, `ml-dsa`, `ml-kem`);
   anything else would be unexplained and fail the case. All
   hits classified as crypto.
2. `tokio` appears in several crates' doc comments — but every
   one says async is *future work* ("Sync only: the async Tokio
   executor arrives in Phase 4", "wiring into an async runtime
   is future work"). The driver counts only code use (`use
   tokio` / `tokio::` paths), which leaves exactly the
   transport's single current-thread worker.

What fairness would need (banked for Matt, not implemented
here): a real task executor with a named policy (round-robin or
otherwise), preemption or timeslice accounting so a runaway
cannot starve others, and — if the design's tree scenario
matters — fairness propagated down the task tree. Today's
protections (deadlines, concurrency bounds, the hook
semaphores from task-47) bound *how much* runs, not *how
fairly* it shares.

## Sources

- Primary: the live working tree — `crates/*/src/**/*.rs`
  excluding `phlow-gauntlet` (vocabulary scans: zero
  scheduling-discipline hits, zero pool hits, one real tokio
  user).
- Primary: `crates/phlow-runtime/src/transport/msgpack.rs`
  (single current-thread worker — not a pool).
- Primary: `crates/phlow-experiment` (`Scheduler`: admission
  DAG, not an executing scheduler).
- Driver: `crates/phlow-gauntlet/src/tasks/task_49.rs`
  (bounded live-workspace probe with line-level hit
  classification).
- Tests: `crates/phlow-gauntlet/tests/task_49.rs` (2V/2A).
