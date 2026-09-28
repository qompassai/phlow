# task-34: adversarial concurrent merge

**Kind:** rust · **Status:** fail (seam absent — no merge function; same-field concurrent write is silent last-writer-wins) · **Wave:** 31–35 · **Commits:** pending (wave 31-35)

## ELI5

When two writers change the same record at the same time, the system
has to *merge* their changes — and sometimes it has to admit they
conflict. The design's rules: if writer A changes field X and writer B
changes field Y, the merge keeps both (field-level resolution). If both
change field X to different values, the merge must either resolve it
deterministically or emit a conflict record naming both writers —
never silently keep only one. And if writer A deletes a field while
writer B updates it, the merge must not silently resurrect the field —
deletion wins explicitly, or the conflict is flagged.

## What this task attempts

- **Goal:** locate phlow's merge path (concurrent writers on one
  record), then drive the design's scenarios against the real merge
  logic: two writers each add a field → merge keeps both; two writers
  modify the same field differently → deterministic resolution or a
  conflict record naming both writers; adversarial: writer A deletes a
  field while writer B modifies it → deletion wins explicitly or the
  conflict is flagged, never silent resurrection.
- **Mechanism:** the real `EvaluationRecord` as the shared mutable
  state — no mocks for the merge logic (there is none to mock),
  scripted adversarial write plans.
- **Success criterion:** no silent write loss (zero lost updates across
  all runs); every conflict is either resolved deterministically or
  surfaced as a conflict record.
- **Non-goals:** inventing a merge. The design is explicit: if no merge
  function exists, that IS the test result — do not build one to make
  it pass.

## What happened

Fail at `"seam"` — on the first and only attempt, honestly. There is no
merge function anywhere in the workspace (verified by source scan:
`merge` appears only in unrelated utility code):

- `disjoint_fields_merge_cleanly` (V): writer A sets field `rev`,
  writer B records an event — both acknowledged, both survive. This
  holds trivially: the record's setters are independent.
- `concurrent_appends_both_survive` (V): both writers append to the
  same event list — both appends acknowledged, both survive in order.
  Appends never conflict.
- `same_field_write_is_silent_last_writer_wins` (A): writer A
  `set_candidate_revision('rev-a')` → Ok; writer B
  `set_candidate_revision('rev-b')` → Ok. Final: `'rev-b'`. Writer A's
  *acknowledged* write was silently lost — 2 acknowledged, 1 surviving,
  0 conflicts surfaced. The final state is indistinguishable from a
  world where writer A never wrote.
- `no_conflict_is_ever_surfaced` (A): after the silent loss, there is
  no conflict marker, no version field, and no field-deletion API at
  all — so the design's delete-vs-update adversarial case has no seam
  either.

The design's pass criteria are violated (a writer's update WAS silently
overwritten; a conflict WAS NOT surfaced), so the verdict is `fail` at
the seam — the absent merge function, documented with the observed
loss. This task is a sibling of task-31 (optimistic concurrency):
both observe the same last-writer-wins store from different angles
(no versioning there, no merge here). The verdicts agree, which is the
correct outcome for a genuinely absent capability.

## The fix — what changed and why

No product fix was made — and none should be made on gauntlet
authority (the design explicitly forbids building a merge to make it
pass). The gauntlet-side work was pinning the real behavior exactly:

- **Changed:** `crates/phlow-gauntlet/src/tasks/task_34.rs` (new) —
  drives the real `EvaluationRecord` with scripted adversarial write
  plans (disjoint fields, same event list, same field, conflict-marker
  inspection); the doc comment records the task-31 relationship so the
  duplicate-seeming finding is understood as two probes of one store.
- **Why:** a merge claim needs a merge function. The cases prove
  disjoint work coexists (trivially) but same-field concurrency loses
  acknowledged writes silently — so the honest verdict is seam-absent,
  not a faked pass on the harmless cases.
- **Source:** `crates/phlow-experiment/src/record.rs`
  (`EvaluationRecord`, its setters, its `Vec` appends — no merge;
  no field-deletion API anywhere in the crate).
- **Validation agents:** the 2 validation tests
  (`disjoint_fields_merge_cleanly`, `concurrent_appends_both_survive`)
  pin the non-lossy cases so the failure is scoped precisely.
- **Adversarial agents:** the 2 adversarial tests
  (`same_field_write_loses_silently`,
  `no_conflict_ever_surfaced_and_task_fails_at_seam`) demonstrate the
  silent acknowledged-write loss and the missing conflict primitive —
  and the folded-in task-level assertion pins the `fail`-at-`seam`
  verdict.

## Full technical depth

The record's mutation surface is independent setters
(`set_candidate_revision`, `set_candidate_digest`, …) plus
bounded-Vec appends (`record_event`, `record_observation`,
`add_security_result`). None of them consult any version, none of them
detect overlap, and none of them record intent — a write is a bare
overwrite or a bare append. Two scripted writers racing on one real
record therefore give exactly the case results above: disjoint work
commutes by construction, same-key work is last-writer-wins, and
there is no metadata (no version vector, no writer id, no intent log)
from which a merge could even be reconstructed after the fact.

Consequence for callers: any two writers to the same field must
serialize out-of-band (the task-31 finding, from the merge angle) or
accept silent loss. If Matt ever wants a merge here, the shape is
clear — intent-carrying writes (writer id + base version), a merge
function with field-level resolution, deterministic conflict rules,
and explicit deletion — but that is a product decision, banked, not a
gauntlet fix.

## Sources

- Primary: `crates/phlow-experiment/src/record.rs`
  (`EvaluationRecord` mutation surface; no merge function; no
  field-deletion API).
- Driver: `crates/phlow-gauntlet/src/tasks/task_34.rs` (real record,
  scripted adversarial write plans, four cases).
- Tests: `crates/phlow-gauntlet/tests/task_34.rs` (2V/2A).
