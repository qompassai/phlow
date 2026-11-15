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
