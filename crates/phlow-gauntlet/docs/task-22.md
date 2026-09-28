# Learning doc template — copy to `task-NN.md` and fill in

> Every section is required. ELI5 first, then full depth. Cite primary
> sources for every protocol/API claim. Document failures with evidence,
> not adjectives.

# task-22: DAG diamond dependencies

**Kind:** rust · **Status:** fail (open) · **Wave:** 21–25 · **Commits:** pending (wave 21-25)

## ELI5

A DAG (directed acyclic graph) is a recipe with steps that depend on each
other: to run step D you need steps B and C done first, and B and C both
need step A done. Drawn out, A→{B,C}→D looks like a diamond. A "DAG
executor" reads those arrows and runs steps in a safe order — never D
before B and C. "Correct" for this task means the diamond's steps get
*admitted in dependency order*, a step whose dependencies never ran gets
refused, and the diamond's outputs join exactly once at D.

## What this task attempts

- **Goal:** execute a diamond A→{B,C}→D through a real dependency-aware
  executor and prove ordering, refusal, and exactly-once join.
- **Mechanism:** `phlow_experiment::control_plane::Scheduler`
  (`crates/phlow-experiment/src/control_plane.rs`) and
  `SchedulerNode.dependency_ids`, driven by a scenario binary that compiles
  phlow's actual `error.rs` + `control_plane.rs` via `#[path]` — real
  sources, no mocks.
- **Success criterion:** topological execution order with refusal of
  unready nodes and exactly-once result join at D.
- **Non-goals:** inventing an executor. The design says an evidenced
  `where = "seam"` failure is the correct result when the design says an
  absent seam is valid.

## What happened

Fail, open seam — on the first and only attempt. Phlow has no DAG
executor. The experiment `Scheduler` is an *admission ledger*: `admit`
stores nodes (with their `dependency_ids` metadata intact) and
`publish_result` records terminal digests, but nothing orders execution
by edges — `Scheduler::admit` never reads `dependency_ids`, there is no
topological ordering, no ready-set, no execute step (the crate documents
"no scheduler execution"). The four cases pin that honest finding:

- `diamond_admit` (V): the diamond admits fine and `dependency_ids` is
  stored faithfully on every node — the metadata is real.
- `dependencies_not_enforced` (V): admitting D with `dependency_ids`
  pointing at never-admitted ghost nodes still succeeds — the behavioral
  proof the edges are inert.
- `duplicate_admit_rejected` (A): re-admitting a node is refused
  (`DuplicateNode`); a second result publish is refused
  (`DuplicateResult`) with the committed digest immutable — the ledger's
  own exactly-once properties hold.
- `shared_output_read` (A): two "racing" readers of A's output both see
  the same committed digest — no torn read.

The task-level `run` reports `fail` with `where = "seam"`: the diamond's
execution-ordering pass criteria cannot be evaluated.

## The fix — what changed and why

No fix — this is a documented design gap, never fixed under gauntlet
authority. The gauntlet-side work was refusing to pretend:

- **Changed:** `crates/phlow-gauntlet/src/tasks/task_22.rs` (new) —
  the scenario template's doc comment states plainly "phlow ships no DAG
  executor"; the cases prove metadata fidelity, non-enforcement, and the
  ledger's exactly-once properties without claiming any of that is
  execution.
- **Why:** asserting a pass from "admits fine" would launder an admission
  ledger into an executor. The honest verdict is the seam failure with
  the recon evidence attached.
- **Source:** `crates/phlow-experiment/src/control_plane.rs`
  (`Scheduler::admit` stores `dependency_ids`; no read of it in the admit
  path; crate docs "no scheduler execution").
- **Validation agents:** the 2 validation tests
  (`diamond_admit_stores_edges_faithfully`,
  `dependencies_are_not_enforced_by_admit`) assert metadata fidelity and
  the non-enforcement behavior.
- **Adversarial agents:** the 2 adversarial tests
  (`duplicate_publish_is_rejected_digest_immutable`,
  `shared_output_read_is_stable_and_task_reports_seam`) try to double-fire
  the join and tear a shared read — both hold — and the folded-in
  task-level assertion pins `where = "seam"`.

## Full technical depth

`SchedulerNode` carries `dependency_ids: Vec<NodeId>` (bounded). `admit`
validates the node's shape (ids, capabilities, budget limits) and inserts
it into the ledger keyed by node id; a duplicate id is refused with
`DuplicateNode`. `publish_result` records a terminal digest for a node in
a terminal state and generation; a second publish for the same node and
generation is refused with `DuplicateResult`; stale generations and
non-terminal states are refused (`StaleGeneration`, `NotTerminal`). Reads
(`node()`) return the committed record — immutable once published, so
concurrent readers see one value.

What is missing for a DAG executor: a ready-set (nodes whose dependencies
are all terminal), a topological scheduler that only dispatches ready
nodes, and a join primitive that gates D on B and C. Today D can be
admitted — and "complete" — with B and C never having existed. The design
gap: either the scheduler grows edge enforcement or the dependency
metadata is documented as informational only.

## Sources

- Primary: `crates/phlow-experiment/src/control_plane.rs`
  (`Scheduler::admit`, `publish_result`, `SchedulerNode.dependency_ids`).
- Primary: `crates/phlow-experiment/src/error.rs` (`DuplicateNode`,
  `DuplicateResult`, `StaleGeneration`, `NotTerminal`).
- Driver: `crates/phlow-gauntlet/src/tasks/task_22.rs` (scenario
  template + four cases + `seam_finding`).
- Tests: `crates/phlow-gauntlet/tests/task_22.rs` (2V/2A).
