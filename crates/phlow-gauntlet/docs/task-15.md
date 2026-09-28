# task-15: lifecycle illegal transitions

**Kind:** rust · **Status:** pass · **Wave:** gauntlet wave 1 ·
**Commits:** none (no repo changes made)

## ELI5

Phlow's self-improvement pipeline moves a *candidate* (a proposed change to
Phlow itself) through a strict sequence of stages: proposed, isolated,
tested, reviewed, awaiting a human, promoted, monitored. Two dead-end
stages exist: rejected and rolled back. A state machine in the Rust code —
`Lifecycle::transition` — is the only legal way to move between stages: it
accepts exactly 13 legal moves, rejects every other combination with a
typed error, and refuses *any* event once a candidate is in a terminal
stage. This task attacked that machine directly: it fired all 13 legal
moves (all succeeded), walked the full happy path to each terminal stage,
then fired 8 illegal moves — skipping stages, deploying without approval,
regressing a promoted candidate, and poking both terminal states. Every
illegal move was rejected with the correct error, and the candidate's stage
was provably unchanged after each rejection. The machine held.

## What this task attempts

- **Goal:** attempt illegal transitions against phlow's real Rust
  lifecycle state machine and prove every one is rejected with state
  unchanged.
- **Mechanism:** `phlow_experiment::promotion::Lifecycle::transition(self,
  event: LifecycleEvent) -> Result<Lifecycle, ExperimentError>` in
  `crates/phlow-experiment/src/promotion.rs` (lines 80–157), driven
  directly from `crates/phlow-gauntlet/src/tasks/task_15.rs` via the
  `phlow-experiment` path dependency.
- **Success criterion:** all 13 legal (state, event) pairs return the
  expected next state; all 8 illegal pairs return
  `ExperimentError::BadTransition` (or `LifecycleTerminal` on terminal
  states); the caller's state is unchanged after each rejection.
- **Non-goals:** the Lua harness lifecycle (a separate machine in diver's
  `lua/ai/harness`, explicitly out of scope for this Rust-kind task); the
  coarser scheduler-node machine (`NodeState` in
  `crates/phlow-experiment/src/control_plane.rs`), which has no
  fine-grained transition table by design.

## What happened

Pass, on iteration 2. Iteration 1 was honestly blocked (see below); once
the `phlow-experiment` dependency appeared in the working tree, the driver
was rewritten to drive the real machine and the full battery ran green:

- **Phase 1 (validation):** all 13 legal transitions succeeded with the
  exact expected next state (evidence lines `ok: <from> + <event> ->
  <next>`).
- **Phase 2 (validation):** `Proposed + ContractInvalid` reached terminal
  `Rejected`; the 7-event happy path `Proposed → Isolated → Tested →
  Reviewed → AwaitingHuman → Promoted → Monitored → RolledBack` reached
  terminal `RolledBack`.
- **Phase 3 (adversarial):** `Promoted + RegressionDetected` (the
  "completed → running" analog) rejected with `BadTransition`; state still
  `Promoted`.
- **Phase 4 (adversarial):** 8 illegal moves rejected — 6 with
  `BadTransition`, 2 terminal-state pokes (`Rejected + WorkspaceCreated`,
  `RolledBack + DeployedToCanary`) with `LifecycleTerminal` — each with
  the pre-call state asserted unchanged afterward.
- `cargo test -p phlow-gauntlet --test task_15`: 5/5 green (4 spec tests
  + metadata check).

## Where it went wrong

- **Stage:** driver wiring, iteration 1 (before any transition ran).
- **Symptom:** the first driver could not name the real types:
  `phlow-gauntlet`'s `Cargo.toml` declared only `serde_json` — no
  `phlow-experiment` dependency — and `Cargo.toml` was outside this
  task's file ownership. The driver verified the machine's presence from
  source text and reported an honest `Fail { where_:
  "dependency-wiring" }` rather than vendoring the transition table (a
  faked pass).
- **Evidence:** the iteration-1 driver is superseded; its honest-fail
  behavior is preserved in git history of the task file (not committed).
- **Root cause:** the gauntlet design doc (`docs/00-design.md`) assumes
  Rust-kind tasks are "driven through phlow's Rust crates (primarily
  `phlow-experiment`)", but no dependency entry existed when the wave
  started. During the wave a sibling worker (task-14, whose target is the
  same crate's evaluator) added `phlow-experiment = { path =
  "../phlow-experiment" }` to `crates/phlow-gauntlet/Cargo.toml` —
  visible as an uncommitted working-tree change with the comment "the real
  evaluator under test". That unblocked this task legitimately: the
  dependency is real workspace state, not something this task smuggled
  in.

## The fix — what changed and why

- **Changed:** `crates/phlow-gauntlet/src/tasks/task_15.rs` — replaced
  the honest-fail probe driver with the real battery: a `LEGAL` table of
  all 13 (state, event, next) triples, a terminal-sequence walk, and an
  `ILLEGAL` battery of 8 (state, event, terminal?) probes, each asserting
  the rejection variant and post-rejection state equality.
  `crates/phlow-gauntlet/tests/task_15.rs` — 2V+2A integration tests
  (see below). `crates/phlow-gauntlet/docs/task-15.md` — this doc.
- **Commit:** none (per task instructions, no commits).
- **Why:** with the dependency present, driving `Lifecycle::transition`
  directly is exactly what the design doc prescribes, and it tests the
  shipped code rather than a copy. The `ILLEGAL` table deliberately
  includes the task brief's cases: terminal → anything (both terminal
  states), skipped stages (`Proposed + RegressionDetected`, `Tested +
  HumanApproved`), and the completed → running analog (`Promoted +
  RegressionDetected`). State-unchanged is asserted with `assert_eq!`
  after each rejection even though the by-value `self` signature makes
  mutation structurally impossible — so a future signature change fails
  loudly instead of silently.
- **Source:** `crates/phlow-experiment/src/promotion.rs:128-157`
  (`transition`); `crates/phlow-experiment/src/error.rs:101-110`
  (error taxonomy); `crates/phlow-gauntlet/docs/00-design.md` (task-15
  row: "out-of-order `LifecycleEvent`s rejected; terminal states reject
  every event").
- **Validation agents:** `cargo test -p phlow-gauntlet --test task_15`
  5/5 green; `cargo test -p phlow-gauntlet --lib` 15/15 green;
  `rustfmt --check` clean on both task files; clippy reports zero
  warnings in this task's files (the single package-level clippy error
  is an unused import in task_11.rs — another worker's file, untouched).
- **Adversarial agents:** the 8-case illegal battery plus the 7-case
  independent battery in the integration tests (which probe the machine
  directly, not through the driver's tables — defense in depth against
  a wrong table). Attempted and found nothing: no illegal pair was
  accepted, no rejection mutated state, no wrong error variant.
- **New convention (if any):** proposed for the coordinator — Rust-kind
  gauntlet tasks need their target crate declared in `phlow-gauntlet`'s
  `[dependencies]` *before* the wave starts; "driven through phlow's Rust
  crates" is not satisfiable on per-task file ownership alone.
- **Citations:** `crates/phlow-gauntlet/Cargo.toml` (`[dependencies]`
  table, uncommitted sibling change); `crates/phlow-gauntlet/docs/
  00-design.md` task-15 row; `crates/phlow-gauntlet/src/lib.rs:52-57`
  (`TaskKind::Rust`: "Driven directly against phlow's Rust crates.").

## Full technical depth

The machine under test is `Lifecycle`, an exhaustive 9-variant enum in
`crates/phlow-experiment/src/promotion.rs:80-106`: `Proposed`, `Isolated`,
`Tested`, `Reviewed`, `AwaitingHuman`, `Promoted`, `Monitored`, plus
terminal `Rejected` and `RolledBack` (`is_terminal`, lines 117–122).
Twelve `LifecycleEvent`s (lines 159–181) drive it. `transition`
(lines 128–157) first rejects any event on a terminal state with
`ExperimentError::LifecycleTerminal { state }`, then matches exactly 13
legal `(state, event)` pairs:

| from | event | to |
|---|---|---|
| Proposed | ContractInvalid | Rejected |
| Proposed | WorkspaceCreated | Isolated |
| Isolated | ChecksComplete | Tested |
| Isolated | ChecksFailed | Rejected |
| Tested | EvaluationComplete | Reviewed |
| Tested | RegressionFound | Rejected |
| Reviewed | GatesSatisfied | AwaitingHuman |
| Reviewed | RegressionFound | Rejected |
| AwaitingHuman | HumanApproved | Promoted |
| AwaitingHuman | ApprovalDenied | Rejected |
| AwaitingHuman | ApprovalExpired | Rejected |
| Promoted | DeployedToCanary | Monitored |
| Monitored | RegressionDetected | RolledBack |

Everything else falls into the `_` arm and returns
`ExperimentError::BadTransition { from, event }` (lines 146–150). The
error taxonomy lives in `crates/phlow-experiment/src/error.rs:101-110`.

The task's key property — "rejection leaves state unchanged, no partial
mutation" — is enforced by the signature itself: `transition(self, …)`
consumes the state by value, and `Lifecycle` is `Copy`. There is no
`&mut self`, no interior mutability, no two-phase validate-then-commit
where validation could pass and mutation half-run. A rejection returns
`Err` and produces no new value; the caller's binding is untouched by
construction. This is the direct structural answer to the task-05 finding
(the Lua supervisor mutated before validating): in this Rust machine,
that bug class cannot be expressed. The battery still asserts the
post-rejection value explicitly, because a future refactor could change
the signature.

A second, coarser machine exists in the same crate and is deliberately
*not* this task's target: `NodeState` in
`crates/phlow-experiment/src/control_plane.rs` (13 variants:
`Proposed → Admitted → Preparing → Executing → Verifying → Reviewing`,
plus 7 terminal states). Its transitions are enforced by `Scheduler`
methods (`admit`, `publish_result`, `cancel_run`) rather than one
transition table — notably, `publish_result` moves any non-terminal node
straight to a terminal state by design (no mandated intermediate
`Executing`), so "skipping running" is legal *there* and would be a false
positive for this task. The design doc's task-15 row names
`LifecycleEvent` ordering, which is only `promotion.rs`.

Test split (2V+2A, in `crates/phlow-gauntlet/tests/task_15.rs`):

- V1 `v_all_legal_transitions_succeed`: `run()` returns `Pass`; exactly
  13 legal-transition evidence lines present.
- V2 `v_sequences_reach_each_terminal_state`: evidence shows both
  terminal `Rejected` and terminal `RolledBack` reached via legal
  sequences.
- A1 `a_completed_to_running_rejected_state_unchanged`: `Promoted +
  RegressionDetected` → `Err(BadTransition { from: "promoted", event:
  "regression_detected" })`; `assert_eq!(state, Promoted)` after.
- A2 `a_illegal_transition_battery_rejected_state_unchanged`: 7 illegal
  pairs probed directly against the real machine (independent of the
  driver's tables), each asserting the expected variant and unchanged
  state.

## Sources

- Primary: `crates/phlow-experiment/src/promotion.rs` (Lifecycle enum
  lines 80–106; `is_terminal` 117–122; `transition` 128–157;
  `LifecycleEvent` 159–181); `crates/phlow-experiment/src/error.rs`
  (`BadTransition`/`LifecycleTerminal` lines 101–110);
  `crates/phlow-gauntlet/Cargo.toml` (`[dependencies]`, uncommitted
  sibling change adding `phlow-experiment`);
  `crates/phlow-gauntlet/docs/00-design.md` (task-15 row);
  `crates/phlow-gauntlet/src/lib.rs:52-57` (`TaskKind::Rust`).
- Related (explicitly out of scope): `crates/phlow-experiment/src/
  control_plane.rs` (`NodeState`, `Scheduler::admit/publish_result/
  cancel_run`); diver `lua/ai/harness` (Lua-side lifecycle).
- Secondary: none.
