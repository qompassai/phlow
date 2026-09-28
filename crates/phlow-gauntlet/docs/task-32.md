# task-32: event-sourced replay

**Kind:** rust · **Status:** fail (seam half-absent — append path exists, no fold/replay) · **Wave:** 31–35 · **Commits:** pending (wave 31-35)

## ELI5

Event sourcing is the "the log IS the truth" pattern. Instead of
storing the current state, the system stores every event that ever
happened: `run.started`, `check.completed`, `run.finished`. To get the
current state you *replay* the log from the beginning, folding each
event into the state one by one — like re-reading a diary to remember
where you left off. The tricky parts: replaying a finished run's log
must produce byte-identical state to what the run had live; if the log
contains a schema change mid-stream, replay must handle it or reject it
explicitly (never misread it); and a truncated log must fail closed
with "incomplete" — never return half-rebuilt state as if it were
final.

## What this task attempts

- **Goal:** locate phlow's run event log (the append path) and its
  state-fold function, then rebuild run state purely by replaying the
  log: run executes, replay yields byte-identical state; adversarial:
  mid-stream schema bump (handled or explicitly rejected); truncated log
  (fails closed with "incomplete").
- **Mechanism:** the real `EvaluationRecord` — no mocks for the fold
  logic, scripted event logs.
- **Success criterion:** replayed state == live state for completed
  runs; every event type has a fold case (exhaustiveness asserted).
- **Non-goals:** hand-rolling a fold. The design says "none for the fold
  logic" as mocks — the fold must be real, or the verdict is honest
  about its absence.

## What happened

Fail at `"seam"` — on the first and only attempt, honestly. The seam is
half-present:

- `events_append_and_are_retained` (V): `record_event('run.started')`,
  `('check.completed')`, `('run.finished')` → all Ok, `event_count ==
  3`, order retained. The append half exists, bounded at `EVENTS_MAX =
  1024`.
- `events_are_opaque_strings_no_fold_cases` (V): the event log is
  `Vec<String>` — each event is an opaque string, not a typed variant.
  Zero event types, zero fold cases. The design's "every event type has
  a fold case" is vacuous: there are no types to be exhaustive over.
- `no_replay_entry_point` (A): `to_json` serializes the record
  one-way — no `from_json`, no replay constructor, no fold function
  anywhere in `crates/phlow-experiment/src` (source scan). A replay
  cannot be constructed against any real API; hand-rolling one would
  invent the seam, not drive it. Fail closed.
- `schema_bump_has_no_replay_handler` (A): `schema_version()` returns
  the compile-time `SCHEMA_VERSION` constant; no code path reads a
  schema version off a log and interprets it — the design's bump
  scenario has no replay to run against.

A note on the near-miss: `Lifecycle::transition` (promotion.rs) *is* a
real fold — it applies typed `LifecycleEvent`s to the promotion state
machine — but no event log is ever appended behind it; transitions
happen imperatively. A fold without a log is not event sourcing, and
claiming it as the seam would be a reskin of task-15 (lifecycle illegal
transitions), which already drove `transition`.

## The fix — what changed and why

No product fix was made — and none should be made on gauntlet
authority. The gauntlet-side work was pinning the real behavior exactly:

- **Changed:** `crates/phlow-gauntlet/src/tasks/task_32.rs` (new) —
  drives the real `EvaluationRecord` append path with scripted logs and
  documents the missing fold; the doc comment names the near-miss
  (`Lifecycle::transition`) and why it does not count.
- **Why:** an event-sourcing claim needs a real fold over a real log.
  The cases prove the log appends fine but nothing folds it — so the
  honest verdict is seam-half-absent, not a faked pass on `to_json`
  (serialization is not replay).
- **Source:** `crates/phlow-experiment/src/record.rs`
  (`record_event`, `EVENTS_MAX`, `to_json` one-way, `schema_version`
  constant),
  `crates/phlow-experiment/src/promotion.rs`
  (`Lifecycle::transition` — fold without a log).
- **Validation agents:** the 2 validation tests
  (`events_append_and_are_retained`,
  `events_are_opaque_strings_no_fold_cases`) pin the working append
  path and the typeless events.
- **Adversarial agents:** the 2 adversarial tests
  (`no_replay_entry_point`,
  `schema_bump_has_no_handler_and_task_fails_at_seam`) try to replay a
  scripted log and to exercise a schema-bump handler — neither exists —
  and the folded-in task-level assertion pins the `fail`-at-`seam`
  verdict.

## Full technical depth

`EvaluationRecord::record_event(&mut self, event: &str)` validates the
text shape (`check_record_text`), rejects past `EVENTS_MAX` (1024) with
`TooManyItems`, and pushes the string. The `events: Vec<String>` field
serializes inside `to_json` (pretty JSON of the whole record). There is
no inverse: no `from_json`, no `TryFrom<&str>`, no builder from an
event slice. A source scan of the crate's public API
(`record.rs`, `control_plane.rs`, `promotion.rs`, `registry.rs`,
`manifest.rs`, `evaluator.rs`, `error.rs`, `lib.rs`) finds no function
that takes an event log and returns state.

The typed-event near-miss: `LifecycleEvent` is a real enum
(`ContractInvalid`, `WorkspaceCreated`, `ChecksComplete`, …) and
`Lifecycle::transition(self, event)` is a real total fold to the next
`Lifecycle` (illegal pairs → `BadTransition`). But lifecycle events are
never logged — there is no `Vec<LifecycleEvent>` anywhere, no append
call, no replay. The state machine advances by direct `transition`
calls in the promotion flow. So the crate has a log without a fold
(`EvaluationRecord.events`) and a fold without a log
(`Lifecycle::transition`) — neither is event sourcing.

Consequence: run state cannot be rebuilt from history; the record's
JSON is a snapshot export, not a replayable log. If Matt ever wants
event sourcing here, the shape is clear — a typed event enum with a
total fold, an append-only log of those events, and a replay entry
point with schema-version dispatch — but that is a product decision,
banked, not a gauntlet fix.

## Sources

- Primary: `crates/phlow-experiment/src/record.rs`
  (`record_event`, `EVENTS_MAX = 1024`, `events: Vec<String>`,
  `to_json` one-way, `SCHEMA_VERSION`).
- Primary: `crates/phlow-experiment/src/promotion.rs`
  (`Lifecycle::transition`, `LifecycleEvent` — fold without a log).
- Driver: `crates/phlow-gauntlet/src/tasks/task_32.rs` (real record,
  scripted logs, four cases).
- Tests: `crates/phlow-gauntlet/tests/task_32.rs` (2V/2A).
