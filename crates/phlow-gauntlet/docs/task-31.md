# task-31: optimistic concurrency

**Kind:** rust · **Status:** fail (seam absent — last-writer-wins documented as the finding) · **Wave:** 31–35 · **Commits:** pending (wave 31-35)

## ELI5

Optimistic concurrency is the "check the version before you save"
pattern. Two writers both read version 7 of a record. The first writer
saves and the record becomes version 8. The second writer tries to save
with version 7 — the store notices the version moved, rejects the write
with "expected 8, got 7", and nothing is lost: the second writer re-reads
(version 8 now), re-applies their change, and saves as version 9. The
key properties: no acknowledged write is ever silently overwritten, and
every rejection names the expected vs actual version so the writer knows
what happened.

## What this task attempts

- **Goal:** locate phlow's versioned state store (run-metadata / state
  store with versioning) and drive the design's scenarios against the
  real compare-and-swap logic: two writers read version 7, first writes
  → version 8, second with version 7 → rejected with `VersionMismatch`;
  retry with a fresh read → version 9; adversarial: a writer that
  ignores the version field is rejected, never applied blind.
- **Mechanism:** the real `EvaluationRecord` and the real experiment
  `Scheduler` — no mocks. The design names the fallback explicitly: "if
  last-writer-wins, document as the finding."
- **Success criterion:** no lost updates (the loser's intent is never
  silently overwritten); every rejection names expected vs actual
  version.
- **Non-goals:** inventing a versioned store. The gauntlet measures what
  exists; a documented failure with evidence is a success.

## What happened

Fail at `"seam"` — on the first and only attempt, honestly. There is no
versioned compare-and-swap write path anywhere in the workspace:

- `record_mutations_are_last_writer_wins` (V): writer A
  `set_candidate_revision('rev-a')` → Ok; writer B
  `set_candidate_revision('rev-b')` → Ok. Final: `'rev-b'`. Writer A's
  *acknowledged* write was silently lost — 2 acknowledged, 1 surviving,
  0 rejections. `EvaluationRecord` carries no per-record version or
  revision counter (only the constant `schema_version`), so there is no
  version to offer and none to check.
- `publish_generation_pin_never_bumps` (V): admit a node (generation
  0), `publish_result` with generation 0 → Ok; the node's generation is
  still 0 afterwards. The generation check pins the node's *delegation
  depth* — fixed at admission, never bumped by any write — so there is
  no version 8 / version 9 and no retry-with-fresh-read path. A
  stale-handle guard, not optimistic concurrency.
- `ignored_generation_rejected_with_expected_vs_got` (A): publish with
  generation 7 on a generation-0 node → `StaleGeneration { node,
  expected: 0, got: 7 }` — the rejection names expected vs actual, and
  the node is untouched (non-terminal, no digest recorded): never
  applied blind. The closest existing mechanism behaves — but it guards
  handle staleness, not write versions.
- `second_publish_rejected_never_overwritten` (A): publish once → Ok;
  publish again → `DuplicateResult`; the first digest (`digest-first`)
  stands. At-most-once publication holds — but again, there is no
  version to retry a stale write against.

The design's pass criteria are not met (an acknowledged write *was*
silently overwritten), so the verdict is `fail` at the seam, with
last-writer-wins documented as the finding — exactly the fallback the
design names.

## The fix — what changed and why

No product fix was made — and none should be made on gauntlet
authority. The gauntlet-side work was pinning the real behavior exactly:

- **Changed:** `crates/phlow-gauntlet/src/tasks/task_31.rs` (new) —
  drives the real `EvaluationRecord` and the real `Scheduler` through
  the real mutation and publication paths; the doc comment names the
  finding (LWW) and the partial mechanism (generation pin) so no one
  reads "fail" as "nothing version-like exists".
- **Why:** an optimistic-concurrency claim needs a version that bumps
  on write. The cases prove (1) record mutations have no version at
  all, and (2) the one version-like check never bumps — so the honest
  verdict is seam-absent, not a faked pass on the generation pin.
- **Source:** `crates/phlow-experiment/src/record.rs`
  (`EvaluationRecord`: no version field; `set_candidate_revision`
  overwrites),
  `crates/phlow-experiment/src/control_plane.rs`
  (`Scheduler::publish_result`, `StaleGeneration`, `DuplicateResult`;
  `generation` is delegation depth, fixed at admission),
  `crates/phlow-experiment/src/error.rs` (`StaleGeneration {
  expected, got }`).
- **Validation agents:** the 2 validation tests
  (`record_mutations_are_last_writer_wins`,
  `publish_generation_pin_never_bumps`) pin the LWW finding and the
  never-bumping pin.
- **Adversarial agents:** the 2 adversarial tests
  (`ignored_generation_rejected_naming_expected_vs_got`,
  `second_publish_rejected_and_task_fails_at_seam`) try to smuggle a
  stale write through blind and to overwrite a published result — both
  are refused — and the folded-in task-level assertion pins the
  `fail`-at-`seam` verdict.

## Full technical depth

`EvaluationRecord`'s fields are plain data: `schema_version: u32`
(stamped with the `SCHEMA_VERSION` constant at construction),
`candidate_revision: Option<String>`, and various `Vec`s. Every
mutation method (`set_candidate_revision`, `record_event`,
`add_security_result`, …) validates its input shape and overwrites or
appends unconditionally. There is no revision counter, no
compare-and-swap, no `VersionMismatch`-style error anywhere in the
workspace (verified by source scan).

`Scheduler::publish_result(node_id, generation, result_digest,
terminal)` checks, in order: terminal state, known node, run not
cancelled, `generation == node.generation()` (`StaleGeneration {
expected, got }` on mismatch), not already published
(`DuplicateResult`), node not already terminal. The `generation` it
pins is `NodeParams.generation` — "Delegation generation (depth); the
scheduler bounds it" — set once at admission and never mutated by any
method on `SchedulerNode` (no setter exists). So the check rejects
results from superseded delegation depths, which is valuable, but it
cannot express "the record changed since you read it": nothing ever
changes the generation.

Consequence for callers: concurrent writers to the same record field
must coordinate out-of-band; the store will not tell them they
collided. If Matt ever wants optimistic concurrency here, the shape is
clear — a per-record revision counter bumped on every mutation plus a
`write_if_version` entry point returning a typed mismatch — but that is
a product decision, banked, not a gauntlet fix.

## Sources

- Primary: `crates/phlow-experiment/src/record.rs`
  (`EvaluationRecord` struct — no version field; mutation methods).
- Primary: `crates/phlow-experiment/src/control_plane.rs`
  (`Scheduler::publish_result`, generation pin, `StaleGeneration`,
  `DuplicateResult`; `NodeParams.generation` as delegation depth).
- Primary: `crates/phlow-experiment/src/error.rs`
  (`StaleGeneration { node, expected, got }`).
- Driver: `crates/phlow-gauntlet/src/tasks/task_31.rs` (real record +
  real scheduler, four cases).
- Tests: `crates/phlow-gauntlet/tests/task_31.rs` (2V/2A).
