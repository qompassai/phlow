# task-14: evaluator budget exhaustion

**Kind:** rust · **Status:** pass · **Wave:** (not assigned in brief) ·
**Commits:** none (no-commit rule for this program)

## ELI5

An evaluator is the part of phlow that runs a staged experiment — validate,
prepare, execute, verify, review, promote — and it gets an allowance: a fixed
number of tool calls, a fixed number of output bytes, and a hard deadline.
This task hands the evaluator an allowance too small for the work and checks
that it fails closed: it stops the moment the allowance is gone, says
exactly which allowance ran out ("tool calls budget exhausted" or "output
bytes budget exhausted"), changes nothing as part of refusing, and can never
be coaxed into reporting success afterwards. A zero allowance is rejected
before anything is built, so there is nothing to spend. Correct looks like a
named exhaustion error, frozen state, and a terminal stage — never a hang,
never a generic error, never a quiet partial result.

## What this task attempts

- **Goal:** prove phlow's real evaluator fails closed on budget exhaustion —
  explicit, bounded, and terminal.
- **Mechanism:** the real code paths, no fakes in the budget logic —
  `crates/phlow-experiment/src/evaluator.rs` (`BudgetTracker::new`,
  `BudgetTracker::consume`, `Evaluator::{validate, prepare, execute,
  verify, review, promote}`) and the typed errors in
  `crates/phlow-experiment/src/error.rs` (`ExperimentError::BudgetExhausted`,
  `::InvalidBudget`, `::DeadlineExceeded`, `::BadStageOrder`). Driven from
  `crates/phlow-gauntlet/src/tasks/task_14.rs` (`run`, Kind::Rust, no Lua,
  no nvim); asserted in `crates/phlow-gauntlet/tests/task_14.rs`
  (2 validation + 2 adversarial + 1 driver test).
- **Success criterion:** tiny budget → the exact `BudgetExhausted{what}`
  variant with the fail-closed message, observable state byte-identical
  before/after the refusal, and `verify` refusing success on the exhausted
  evaluator — terminal, no resurrection.
- **Non-goals:** wall-clock (`DeadlineExceeded`) exhaustion paths; the
  control-plane scheduler budgets; diver's Lua harness (that's task-02, the
  sibling that proved the same property on the Lua side).

## What happened

Passed on the first behavioral attempt: `cargo test -p phlow-gauntlet
--test task_14` → 5/5 green in 0.00 s; `cargo clippy -p
phlow-gauntlet --all-targets -- -D warnings` → zero warnings; `cargo fmt -p
phlow-gauntlet -- --check` → clean; full-crate regression `cargo test -p
phlow-gauntlet` → all suites green (58 tests, 0 failures), so the new
`phlow-experiment` path dependency broke nothing. The evaluator has real
budget enforcement — no "document the absence" fallback was needed. Evidence
from the driver run:

- `overspend execute(1,200) -> BudgetExhausted{what="output bytes"}:
  "output bytes budget exhausted; failing closed"`
- `state unchanged after refusal: stage=execute remaining_calls=1
  used_bytes=0`
- `verify(complete evidence) -> BudgetExhausted{what="tool calls"}:
  "tool calls budget exhausted; failing closed" — refusing success on
  exhausted work`
- `second verify refused identically; no resurrection`
- `terminal: stage=verify remaining_calls=0 used_bytes=64 —
  review/promote unreachable, no success reported`

## Where it went wrong

One compile-time iteration before any test ran — not a behavioral failure.

- **Stage:** first `cargo clippy` after wiring the dependency.
- **Symptom:** `error[E0603]: struct 'VerificationOutcome' is private ...
  the module 'evaluator' is defined here --> 
  crates/phlow-experiment/src/lib.rs:47: mod evaluator;` (8 errors, one per
  imported item).
- **Evidence:** `crates/phlow-experiment/src/lib.rs:47` declares `mod
  evaluator;` (private) while lines 61–65 re-export every item at the crate
  root via `pub use evaluator::{...}`.
- **Root cause:** I imported from `phlow_experiment::evaluator::{...}`,
  assuming the module path was public. It isn't — the crate's public
  contract is the root re-export. Verified by reading `lib.rs`, not guessed.

## The fix — what changed and why

- **Changed:** `crates/phlow-gauntlet/src/tasks/task_14.rs` and
  `crates/phlow-gauntlet/tests/task_14.rs` — both import blocks changed
  from `phlow_experiment::error::ExperimentError` +
  `phlow_experiment::evaluator::{...}` to a single
  `phlow_experiment::{ArtifactDigest, BudgetTracker, CheckRun, EvalStage,
  Evaluator, EvidenceBundle, ExperimentError, VerificationOutcome}`.
- **Commit:** none (no-commit rule).
- **Why:** the fix follows the crate's actual public contract (the root
  re-exports) instead of reaching through a private module path. The
  alternative — asking to make `mod evaluator` public — would widen the
  crate's API surface for no reason; rejected.
- **Source:** `crates/phlow-experiment/src/lib.rs:47` (`mod evaluator;`)
  and `:61–65` (`pub use evaluator::{...}`) — primary source, read directly.
- **Validation agents:** this worker — `cargo clippy -p phlow-gauntlet
  --all-targets -- -D warnings` zero warnings, `cargo test -p
  phlow-gauntlet --test task_14` 5/5, full-crate `cargo test -p
  phlow-gauntlet` all green.
- **Adversarial agents:** this worker — the 2 adversarial tests below plus a
  `u64::MAX` overflow probe against `checked_add` (documented under full
  depth); no holes found.
- **New convention (if any):** none.
- **Citations:** `crates/phlow-experiment/src/lib.rs:47,61–65`;
  `crates/phlow-experiment/src/evaluator.rs` (full file, read before coding).

## Full technical depth

The budget is `BudgetTracker { tool_calls_remaining, output_bytes_max,
output_bytes_used, deadline_ms, now_ms }`
(`crates/phlow-experiment/src/evaluator.rs`, "Budget tracker" section). The
clock is injected (`set_now_ms`), so every path is deterministic. Three
properties make exhaustion fail closed:

1. **Construction rejects zero.** `BudgetTracker::new` returns
   `Err(InvalidBudget{field})` for any zero dimension — there is no tracker
   to spend from, so a zero budget can never execute anything. The driver
   never reaches this path because the tiny budget (1 call, 128 bytes,
   600_000 ms deadline) is valid-but-insufficient; the adversarial test
   covers all three zero dimensions.

2. **Consume checks before mutating.** `BudgetTracker::consume` orders its
   checks as deadline → tool-call bound → checked-add output bytes → output
   bound, and only then does `tool_calls_remaining -= tool_calls;
   output_bytes_used = used`. A refusal therefore leaves every field
   untouched — the driver snapshots `stage/remaining/used` before and after
   the overspend and requires byte-identical equality, which is the
   "no half-applied state" proof. The `checked_add` maps overflow to
   `BudgetExhausted{what: "output bytes"}` instead of wrapping; the
   adversarial test spends exactly `u64::MAX` output bytes then attempts one
   more byte and asserts the closed failure and an unchanged `used` counter.

3. **Exhaustion is terminal at the evaluator level.** `Evaluator::execute`
   calls `self.budget.consume(..)?` *before* `self.advance(..)`, so a
   refused execute never advances the stage machine. `Evaluator::verify`
   checks deadline, then requires `tool_calls_remaining > 0` — "verification
   itself costs a tool call" — *before* looking at evidence, so even a
   complete `EvidenceBundle` yields `Err(BudgetExhausted{what: "tool
   calls"})` rather than `Ok(true)`. The evaluator is then stuck at
   `EvalStage::Verify` with no budget: `review`/`promote` are unreachable
   (stage order), and a second `verify` fails identically — no resurrection,
   no path to success.

The error text is explicit, not generic: `Display` for `BudgetExhausted`
renders `"{what} budget exhausted; failing closed"`, and the tests assert
both the exact enum variant (`assert_eq!` against
`ExperimentError::BudgetExhausted{what: "output bytes"}`) and the message
string. This is the Rust-side sibling of task-02, which proved the same
fail-closed property for diver's Lua harness (`budget.exhausted` event,
terminal `failed` state); the two together cover both runtimes.

One scaffolding note: `phlow-gauntlet` had no dependency on
`phlow-experiment`, so driving the real evaluator required adding one line
to `crates/phlow-gauntlet/Cargo.toml`
(`phlow-experiment = { path = "../phlow-experiment" }`) — build
configuration, not code, and the only file touched outside the task's three.
It is flagged here so the orchestrator can confirm it doesn't collide with
other workers' edits.

## Sources

- Primary: `crates/phlow-experiment/src/evaluator.rs` (BudgetTracker,
  Evaluator, EvalStage, evidence types — full file read);
  `crates/phlow-experiment/src/error.rs` (`BudgetExhausted`,
  `InvalidBudget`, `DeadlineExceeded`, `BadStageOrder` variants and their
  `Display` impls); `crates/phlow-experiment/src/lib.rs:47,61–65`
  (private module, public re-exports).
- Secondary: `crates/phlow-gauntlet/docs/task-02.md` (the Lua-side sibling
  result this task mirrors on the Rust side).
