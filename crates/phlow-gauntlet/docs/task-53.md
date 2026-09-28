# task-53: quorum reads and writes

**Kind:** rust · **Status:** fail (open) · **Wave:** 51–55 · **Commits:** pending (wave 51-55)

## ELI5

When data lives on several machines, you do not need every machine to
agree: you write to enough of them (a "write quorum") and read from
enough of them (a "read quorum") that the two groups overlap — so a
read always sees the latest write even if some machines are down. The
rule of thumb is W + R > N: with 5 replicas, writing to 3 and reading
from 3 guarantees overlap. "Correct" here means the design's numbers
hold: a 3-node write is durable once 2 confirm, and a read after a
network partition still returns the last confirmed value.

## What this task attempts

- **Goal:** find a replicated state store in phlow and show its quorum
  reads/writes meet the design's durability numbers.
- **Mechanism:** `src/tasks/task_53.rs`, a Rust driver with four
  scenarios: (V1) scan the real phlow crate sources for replication
  vocabulary (quorum/replica/replication); (V2) drive the real
  `phlow_experiment::Scheduler` — admit one write, look up one read —
  showing single-node semantics and no quorum knobs on any API; (A1)
  scan for peer/partition primitives; (A2) re-verify task-31's
  finding that no per-record version exists for read-repair.
- **Success criterion:** the design's quorum pass criteria
  (3-node write durable at 2 acks; partitioned read returns last
  confirmed value; read-repair convergence).
- **Non-goals:** inventing a replicated store. The design says an
  evidenced `where = "seam"` failure is the correct result when the
  design says an absent seam is valid.

## What happened

Fail, open seam — no replicated state store exists in any phlow
crate. All real stores are single-node: the experiment scheduler's
write quorum is degenerately W=1, R=1, N=1, and no API carries a
quorum parameter. The driver:

- `single_node_stores_located` (V): the replication vocabulary scan
  returns zero hits across all phlow crates (excluding the gauntlet
  crate itself, whose driver docs legitimately carry the vocabulary);
  the real stores are
  documented as single-node.
- `single_node_read_write_semantics` (V): one local write admitted,
  one local read returned, zero replication hooks fired — the write
  is "durable" the moment it lands in the single process.
- `partition_is_vacuous` (A): no peer set, no partition primitive —
  the partitioned-read scenario cannot be constructed.
- `no_version_for_stale_read_prevention` (A): task-31's finding
  re-verified — no per-record version, so read-repair has no version
  to compare.

Banked product decision for Matt (NOT auto-implemented): whether
phlow should ever gain replicated state — and if so, where the
replication seam belongs. The driver flags the question and stops.

## The fix — what changed and why

No fix — this is a documented design gap, never fixed under gauntlet
authority. The gauntlet-side work was making the absence check honest:

- **Changed:** `crates/phlow-gauntlet/src/tasks/task_53.rs` (new) —
  vocabulary scans over real crate sources plus a real
  `phlow_experiment::Scheduler` drive; distinguishes "seam absent"
  from "probe crashed" and "premise changed".
- **Why:** a claim of "no replicated store" must be evidenced against
  the real code (and against the one misleading token: the word
  "replicates" in experiment docs), and the degenerate quorum
  (W=1/R=1/N=1) must be shown behaviorally, not asserted.
- **Source:** `phlow-experiment` crate sources (scheduler); crate
  manifests of all phlow crates for the token scans.
- **Validation agents:** the 2 validation tests
  (`single_node_stores_located`,
  `single_node_read_write_semantics`) assert the zero-hit scan and
  the single-node read/write behavior.
- **Adversarial agents:** the 2 adversarial tests
  (`partition_is_vacuous`,
  `no_version_for_stale_read_prevention`) assert the partition
  scenario is unconstructable and read-repair has no version input.

## Full technical depth

The driver scans every `.rs` file under each phlow crate for the
exact tokens `quorum`, `replica`, `replicate`, `replication` (the
scan explicitly ignores the experiment-docs "replicates" wording).
Zero hits. The real read/write seam — `phlow_experiment::Scheduler`
— admits one write and answers one lookup locally, with no peer set,
no replica count, no write-quorum/read-quorum parameters, and no
read-repair: durability means "present in this process". The peer
scan (exact tokens `peer`, `partition`, `majority`) also returns
zero, so the design's partition scenario is vacuous — there is no
network to partition. A2 re-runs the task-31 per-record-version check at write-ordering
level: no monotonic counter, vector clock, CAS tag, or lamport
timestamp exists on the record. Present but not write-ordering: the
constant `schema_version` 1 (a format version, never bumped by a
write) and `baseline_revision`/`candidate_revision` (git SHAs
identifying the code under evaluation — content labels, no ordering
semantics a read-repair could compare). Last-writer-wins is the only
conflict resolution.

What is missing for the design's quorums: a replicated store
primitive, quorum parameters (N, W, R) on the write/read paths, a
partition model with partition-tolerant reads, and per-record
versions enabling read-repair convergence. The design gap: either
phlow grows replicated state (with an explicit quorum seam) or the
single-node architecture is documented as the intended boundary.

## Sources

- Primary: `phlow-experiment` crate sources (`src/scheduler.rs`,
  manifests); token scans over all phlow crates.
- Driver: `crates/phlow-gauntlet/src/tasks/task_53.rs` (rust driver).
- Tests: `crates/phlow-gauntlet/tests/task_53.rs` (2V/2A).
