# Self-improvement safety case (PH-9)

Status: landed 2026-11-10. Rule of the case: **the loop may propose;
only the operator disposes.** Every self-improvement surface in
phlow is either read-only, append-only, or a typed denial. This
document states what the loop may never do, where each denial lives,
and which test proves it. The tests are the case: they live in the
owning crates' suites (`tests/safety_case.rs` in `phlow-self-improve`,
`phlow-checks`, and `phlow-runtime`, plus the unit tests beside each
denial), and the suite is itself a candidate approved check (PH-12) —
weakening it is a visible diff to a named check, not a silent edit.

## The denials

### 1. The loop may never edit its own prompts

`phlow-self-improve::PromptEvolver::evolve` always fails with
`SelfImproveError::EvolutionDisabled` and the exact denial text:
*"Automatic prompt evolution and implicit Git commits are disabled.
Review and edit operator-owned prompts manually."* Construction
succeeds; evolution never does. There is no other prompt-writing
path in the crate.

**PH-4 stays OFF.** Automatic prompt mutation does not exist and
this package does not add an offline candidate generator: under
D-1269, candidate generation — if it is ever built — is
proposal-only (diffs with evidence, evaluated offline, applied by a
person). Tonight's state is the stronger one: no generator at all.

Tests: `prompt_evolver::tests::evolve_always_fails_with_exact_python_denial`;
`safety_case::prompt_evolution_is_denied`.

### 2. The loop may never change its own checks

Checks are exact argv from the operator's TOML (`FlowConfig`),
pinned by canonical path + SHA-256 at admission and re-verified
before every spawn (`phlow-checks::pin`). No runtime API, and no
model-visible tool, adds, removes, or edits a check. The legacy
implicit-execution tools are fail-closed denials, preserved as typed
values in `phlow-checks::disabled`:

- `shell_exec` → *"Arbitrary commands and cwd overrides are
  disabled. Configure exact named check argv and use flow_check."*
- `lsp_check` → *"No implicit language command execution. Configure
  a named lint/typecheck in operator TOML or attach Rose's native
  editor tools."*

An unknown check name reports `unavailable` ("No such configured
check") — the loop cannot invent a check into existence.

Tests: `disabled::tests` (exact texts); `safety_case` in
phlow-checks (denials + unknown check never runs); runtime
`safety_case::model_tool_surface_has_no_self_mutation_tools`
(the removed tools stay absent from every role's schema list).

### 3. The loop may never widen its own budgets

Budgets (`max_iterations`, `max_cycles`, `max_tool_calls`,
`max_context_chars`, `max_task_chars`) exist only inside validated
operator config (`phlow-config::AgentConfig`, range-asserted at
construction). The runtime's one dynamic influence, the utility
pre-gate, can only **narrow**: `run_max_cycles` is applied as
`cap.min(config.max_cycles())`, and the run-scoped model override
never writes back to the operator config. Best-of-k is an embedder
setter with a hard `1..=8` assert — operator surface, not a model
tool (see the schema audit test).

Tests: `phlow-config` load tests (out-of-range budgets rejected);
`safety_case::model_tool_surface_has_no_self_mutation_tools`
(no budget-shaped tool is model-reachable).

### 4. The loop may never mutate its own skills

`phlow-self-improve::SkillStore` is read-only over a root-pinned
workspace. `save_skill` always fails with `SkillMutationDisabled`
(*"Automatic skill mutation is disabled; edit manually"*).

Tests: `skill_store::tests`; `safety_case::skill_mutation_is_denied`.

### 5. The loop may never touch history

Two histories, both closed:

- **Skill/Git history**: `SkillStore::revert_skill` always fails
  with `GitMutationDisabled` (*"Automatic Git mutation is disabled;
  revert manually"*), and `history()` returns an empty list instead
  of reading Git — the loop cannot even *see* history to act on it.
- **Run history**: the PH-1 outcome table is append-only at the
  SQLite level (UPDATE/DELETE triggers abort on any connection),
  and one run leaves exactly one row (`OutcomeDuplicate` refusal).

Tests: `skill_store::tests`;
`safety_case::history_is_neither_read_nor_rewritten`,
`outcome::tests::rows_are_append_only_against_update_and_delete`.

### 6. The loop may never rewrite its own lessons (PH-6)

Plan-versus-actual divergence records and the lesson cards built
from them are evidence, held to the same discipline as run history:

- **Append-only.** `plan_divergences` and `lesson_cards` carry the
  same UPDATE/DELETE triggers as `run_outcomes`. A card is
  superseded by appending its next version; rebuilding over an
  unchanged run set appends nothing. One run leaves exactly one
  divergence record (`DivergenceDuplicate` refusal).
- **Retrieved, never mutated into the prompt.** A card reaches the
  planner as run context (the user-side message) — the planner's
  system prompt is byte-identical with or without a card. Lessons
  are data the planner reads, not instructions the loop wrote into
  itself; this is denial 1's boundary applied to PH-6.
- **Lane-bound.** A card's content label is the most restrictive
  label among its source records (any non-`sfw` source makes the
  card `nsfw`), and an `nsfw` card is invisible to a retrieval
  scoped `sfw` — fail-closed, per the Bloch lane law.

Tests: `divergence::tests` (derivation, append-only, one-per-run);
`lessons::tests` (aggregation, versioning, lane labels, scope
visibility); `safety_case::divergence_records_and_lesson_cards_are_append_only`,
`safety_case::nsfw_lessons_are_invisible_to_sfw_retrieval`;
runtime `lesson_cards` suite (class recording, context injection,
prompt byte-identity, with/without status parity).

### 7. A proposed check never admits itself (PH-3)

Denial 2 says the loop may never change its own checks. PH-3's
check proposals are the proposal side of that denial, held to
three rules:

- **Escapes are evidence-checked and append-only.** An escape
  records only against a PH-1 outcome row whose recorded checks
  are non-empty and all `ok` — a defect a check caught is not an
  escape and the store refuses to call it one. The `escapes`
  table carries the same UPDATE/DELETE triggers as run history,
  and the record copies the escaped run's task class and lane
  labels from its outcome row; they cannot be re-labelled at
  record time.
- **Validation before queueing.** A proposal queues only when its
  exact argv fails on the escaped revision and passes on the
  fixed one, run directly (no shell) under a deadline by the
  operator-supplied revision pair. An unvalidated or
  gate-failing proposal is rendered as such and never queued.
- **Pending is the end of the pipeline.** Queueing admits the
  proposal to `phlow-approval` as a pending record (no permission
  change: admitting a check grants the loop nothing). No code
  path in the proposal pipeline decides a record, writes a
  config, or adds a check; admission takes an operator's verdict
  on the record plus the operator's own config edit.

Tests: `escape::tests` (clean-run requirement, append-only,
clustering, validation gate);
`safety_case::escapes_are_append_only_and_require_a_clean_run`,
`safety_case::a_check_proposal_never_admits_itself` (pending
state, non-operator and reserved-actor refusals, bystander
config byte-identical).

### 8. A lesson's lane comes from its trajectory, never its caller (PH-7)

PH-7 captures a finished run as a lesson record for bloch. The
lane law (labels from birth, fail-closed, nsfw invisible to sfw)
is enforced structurally, not by caller discipline:

- **Derived, never asserted.** The capture API takes no lane
  parameter. The lane is derived inside the capture from the
  run's flowguard trajectory label (`LabelSnapshot`): a public
  trajectory (`ReaderSet::All`) captures `general`/`sfw`;
  anything narrower — or no trajectory at all — fails closed to
  `restricted`/`nsfw`. The outcome row's own labels are never
  copied (runtime rows carry placeholders), and the snapshot's
  trust rank is recorded as evidence only: the audience decides
  the lane.
- **Evidence-checked and append-only.** A capture records only
  against a PH-1 outcome row, exactly once per run
  (`CaptureInvalid` / `CaptureDuplicate` refusals), in a table
  carrying the same UPDATE/DELETE triggers as run history.
  Trajectory evidence is recorded as shape — audience class,
  reader count, trust name — never reader identities.
- **Eligibility is verification, not outcome.** A capture is
  lesson-eligible exactly when the run's verification strength
  reaches `tool_checked` (BL-2); failed and weakly verified runs
  are still captured, marked ineligible — the capture log is the
  audit trail, not only the lesson feed. Visibility follows the
  lane law: an `nsfw` capture is invisible, and uncounted, to an
  `sfw`-scoped listing.

Tests: `capture::tests` (lane derivation per audience, refusal
and duplicate, scope visibility, append-only);
`safety_case::capture_lane_comes_from_the_trajectory_never_the_row`,
`safety_case::captures_are_append_only_and_refuse_unknown_runs`.

### 9. Skill trust is recomputed from rows, never asserted (PH-8)

PH-8 records the skills a run loaded on its outcome row and
recomputes per-skill standing from those rows — the same posture
as PH-5's reviewer calibration:

- **Rows are the evidence.** The recompute reads only outcome
  rows (status, verification strength, recorded skills). A run
  recorded before PH-8 carries no skills and counts for no skill;
  nothing is backfilled and no surface's self-report is consulted.
- **One factual failure demotes.** A failure is *factual* when the
  run failed **and** its verification strength reached
  `tool_checked` — the outcome was tool-checked, not merely
  asserted. A single factual failure demotes the skill regardless
  of successes; a self-reported failure never demotes, because an
  unverified run cannot indict a skill. `abandoned` runs are
  neither successes nor failures.
- **Standing is a proposal.** Trusted requires at least 5 runs
  and a Wilson 95% lower bound at or above 0.5 (named proposals:
  `MIN_SKILL_RUNS`, `TRUST_FLOOR`). The report renders evidence
  and edits nothing — no skill, config, or prompt changes path
  exists here, per denials 1 and 4.

Tests: `skill_trust::tests` (Wilson bounds, factual-failure
demotion, self-reported non-demotion, provisional floor,
pre-PH-8 loader); `outcome` skills round-trip and migration
tests; `safety_case::skill_trust_recomputes_from_rows_and_one_factual_failure_demotes`.

### 10. A cue cannot widen the approved check set (PH-K2)

PH-K2 wires the fleet cue primitives into the runtime's live run
path: at the head of every run, the runtime drains its own cue
directory — expired `run-this` cues reaped and listed, live ones
answered with `run-done`. The drain adds no authority:

- **Approved set only.** The handler runs named checks from the
  operator-approved config, executed by the same pinned
  `CheckRunner` the runtime itself uses. A cue naming a check
  outside that set is refused *whole* — verdict `refused`, the
  unapproved name in the evidence, and nothing runs, not even the
  approved names beside it. A cue can ask for a subset of what the
  operator approved; it can never add to it.
- **Addressed work only.** Only cues addressed to `phlow` are
  consumed; another server's cue is left pending, untouched. Only
  cues visible under the drain's lane scope are listed or claimed
  (the primitives' lane law), and every `run-done` carries the
  original cue's labels from birth.
- **Refused by name, never waited on.** A malformed cue file
  fails the drain with the file's name, surfaced on the run
  report as `cues_error`; the run proceeds — the fleet protocol
  cannot take the worker down with it. Expired cues are reaped
  and listed (`cues_expired`), never consumed and never waited
  on. Per-run bounds: at most 8 cues drained, at most 32 checks
  per cue, evidence bounded to 2,000 characters.

Tests: `cues::tests` (payload parsing, evidence bound);
`cue_wiring` (consume-and-answer in a real run, exactly-once;
expired reaped and listed);
`safety_case::cue_cannot_widen_the_approved_check_set`,
`safety_case::malformed_cue_file_is_refused_by_name_and_the_run_proceeds`,
`safety_case::cue_addressed_to_another_server_is_not_consumed`.

### 11. An export never defaults a lane and never duplicates (PH-13)

PH-13 exports outcome rows to bloch's BL-1 intake. The export is
a mapping with refusals, not a copy:

- **Lane from the capture, never the caller.** The exported
  `origin_lane`/`content_label` come from the run's PH-7 capture,
  where they were derived from the flowguard trajectory — the
  export API has no lane parameter, and the outcome row's
  placeholder labels are never read. A capture whose lane is
  outside bloch's vocabulary (`general`/`restricted`) is refused
  (`ExportInvalid`), never defaulted; a run with no capture has
  no derivable lane and is refused by id.
- **Lane law at export.** An `nsfw`-labelled record is never
  exported into an `sfw`-scoped intake (`ExportLaneRefused`,
  typed, reported per run). The scope is an explicit operator
  argument to `phlow-export` — there is no default scope.
- **Vocabulary is bloch's.** `result` is required in BL-3's
  vocabulary (`ok`/`failed`/`abandoned`); a capture carrying
  anything else is refused, never renamed. `asked` is the run's
  task text, joined from its run report; a run with no joined
  task text is refused, never fabricated from the plan.
  `verification` is the row's own strength label.
- **Append-only, never duplicated.** Bloch's store appends
  without dedupe, so the stop is phlow's: an append-only
  `outcome_exports` ledger (same UPDATE/DELETE triggers as run
  history) plus a scan of the target file itself, so a lost
  ledger cannot re-export what the target already holds. An
  oversized or corrupt target is a hard refusal, never a
  truncated guess.

Tests: `export::tests` (mapping shape, lane/label/result
refusals, scope refusal, batch dedupe, target-scan dedupe);
`bloch_link` (a mapped row accepted by bloch's own intake gate;
the exporter's output read back as a bloch store; refusal-vocabulary
parity); `safety_case::export_never_defaults_a_lane_and_never_fabricates_asked`,
`safety_case::export_refuses_nsfw_into_an_sfw_intake`,
`safety_case::export_never_duplicates_a_run`.

## What is *not* a denial (and must not become one silently)

- Recording outcomes and feedback is append-only writing by the
  **runtime**, from its own counters — the model has no tool for it.
- Proposals are allowed and encouraged (PH-2 budgets, PH-3 checks,
  PH-4 prompts): a proposal is a file an operator reads, queues
  through `phlow-approval`, and applies or rejects. Nothing in this
  case blocks proposals; it blocks *self-application*.
- The operator's own surfaces (CLI flags, `/model`, editing TOML,
  editing prompts by hand) are untouched by every denial above.
  Denials bind the loop, not the person.

## Amending this case

A new self-improvement capability lands only with: its denial or
append-only rule stated here, one test per denial in the owning
crate's `safety_case` suite, and the proposal path (if any) named.
Removing a denial requires removing its test in the same diff —
which is exactly the visibility this case exists to provide.
