# task-12: poisoned context compaction

**Kind:** rust · **Status:** pass · **Wave:** gauntlet 20-task (single-task worker) ·
**Commits:** none (working tree only; do not push without Matt's word)

## ELI5

Imagine an agent that reads long tool logs and, to save memory, squeezes them
into short "receipts": a summary plus a few exact quotes from the original.
The danger: what if someone slips a fake quote into the squeeze — a line the
original never said? Then the short receipt *looks* trustworthy but carries a
lie, and the original (which would expose the lie) is thrown away. This task
attacks the piece of code that is supposed to prevent exactly that: phlow's
`EvidenceReducer`. The attacker feeds it fake quotes, sneaky one-word edits,
and receipts tampered with after the fact. "Correct" means the reducer says
**no** every time — it refuses to compact and keeps the original untouched —
while still compacting honest, exact quotes normally.

## What this task attempts

- **Goal:** prove the shipped `EvidenceReducer` refuses to launder poisoned
  evidence into a clean-looking compact receipt.
- **Mechanism:** drives the real `phlow_agent::solpi::EvidenceReducer`
  (`crates/phlow-agent/src/solpi/reducer.rs`) — compiled verbatim into the
  gauntlet task module via `#[path]`, not reimplemented — through
  `crates/phlow-gauntlet/src/tasks/task_12.rs` (`run()`), with 2V+2A
  integration tests in `crates/phlow-gauntlet/tests/task_12.rs`.
- **Success criterion:** poisoned input → `ReductionOutcome::Unchanged`
  (originals preserved, verifiable); clean input → `Compacted` (control).
- **Non-goals:** the reducer's input-bound errors (`SourceTooLarge` etc.)
  are the reducer's own unit tests' job; this task covers the
  verification-vs-poison behavior only. No Lua, no Neovim, no network.

## What happened

Passed on the first attempt — the defense held. `cargo test -p
phlow-gauntlet --test task_12`: 5/5 green. The driver `run()` executed all
five checks and reported `Pass` with one evidence line per check:

- `control-compacts`: clean proposal compacted; 3/3 quotes verified
  byte-for-byte against 148 source bytes; every retained quote
  independently re-checked with `str::contains`.
- `fabricated-quote-refused`: invented quote `"tls: disabled; backdoor on
  :9090"` attributed to the real source → `Unchanged`, source byte-identical
  before/after, reason names `quotation 0`.
- `altered-quote-refused`: `"42 passed, 1 failed"` (one digit flipped from
  the true `"42 passed, 0 failed"`) → `Unchanged`.
- `tampered-receipt-refused`: a legitimately compacted receipt edited
  post-hoc (`"tls: enabled"` → `"tls: disabled"`) and re-proposed →
  `Unchanged`. There is no acceptance path for a receipt struct except fresh
  re-verification through `reduce()`.
- `requires-opt-in`: a non-opted-in reducer returns `Err(NotEnabled)` and
  verifies nothing.

## The fix — what changed and why

No fix iterations: the task passed on the first attempt, so there is no fix
entry. (Per the template, this section records one entry per fix iteration;
zero iterations occurred.) The one deliberate design decision, made before
any code: **include the shipped reducer source verbatim** (`#[path =
"../../../phlow-agent/src/solpi/reducer.rs"]`) instead of reimplementing it,
because reimplementing the unit under test would prove nothing about the
code that actually ships. Rationale: the gauntlet crate deliberately has no
dependency on phlow-agent (disjoint crate boundaries), and the task brief
forbids touching anything outside the three task files — including
`Cargo.toml` — so a path dependency was not an option. The `#[path]` include
compiles the exact shipped bytes into the test harness; the doc and the
driver's evidence both state this openly.

## Threat model

**What the wolf can do.** The wolf controls the *proposal*, not the source:
it is the component (a model, a plugin, a compromised tool) that suggests
"here is the compact version of that long log." It can put any strings it
likes in `summary` and `quotes` — including text the source never contained,
near-copies with one word changed, or a receipt struct it edited after a
legitimate verification.

**How it gets in.** Context compaction is where long evidence is *replaced*
by short evidence. If the reducer blessed a poisoned proposal, the poison
would be laundered: the original source gets discarded, the receipt survives
as "verified," and every downstream decision cites a quote nobody can check
anymore. The three concrete entries: (1) fabricated quote — invented text
pinned on a real source, e.g. turning `"tls: enabled"` into `"tls:
disabled; backdoor on :9090"`; (2) subtle alteration — `"42 passed, 0
failed"` → `"42 passed, 1 failed"`, one digit flipping a test result;
(3) tampered receipt — take a genuinely verified receipt and edit one
retained quote afterwards.

**What stops it.** Three layers, all in the shipped code
(`crates/phlow-agent/src/solpi/reducer.rs`):

1. **Opt-in gate** (lines 165–167): `reduce()` on a non-opted-in reducer
   returns `Err(ReducerError::NotEnabled)` before any verification. The
   reducer is inert unless explicitly enabled — no silent default-on.
2. **Byte-for-byte verification, fail-closed** (lines 170–182): bounds are
   checked first (line 168), then every quotation must be non-empty and
   occur in the source via `source.contains(quote)`. The *first* mismatch
   returns `ReductionOutcome::Unchanged` with a bounded reason naming the
   offending quotation index. No partial receipt is ever produced — the
   `Compacted` variant is constructed only after the loop completes.
3. **No trust in receipt structs**: `CompactReceipt` is an output-only
   value; nothing in the API accepts one as input. A tampered receipt must
   come back through `reduce()` as a fresh proposal, where layer 2 kills it.

**Residual risk (stated, not fixed — out of scope).** `contains()` is
substring matching: a quote that appears *somewhere* in the source verifies
even if the proposal's framing misattributes its context. Byte-for-byte
matching proves the words exist, not that the summary interprets them
honestly. The summary text itself is never verified — by design, it is the
caller's claim, and the quotes are the checkable evidence. A wolf that keeps
quotes exact but writes a lying summary is outside this reducer's contract.

## Full technical depth

Data flow: caller holds `source: &str` (borrowed, never mutated — "unchanged
on failure" is structural, enforced by the borrow checker, not by a restore
step) and builds a `ReductionProposal { summary, quotes }`. `reduce()`
validates input bounds (`SOURCE_BYTES_MAX` = 1 MiB, `SUMMARY_BYTES_MAX` = 8
KiB, `QUOTES_MAX` = 64, `QUOTE_BYTES_MAX` = 8 KiB — lines 24–39) returning
typed `ReducerError`s on violation; these are caller bugs, not verification
outcomes. Verification then iterates the quotes in order: empty quotes are
rejected explicitly (an empty quote matches everything and proves nothing —
lines 171–176), and each non-empty quote must satisfy
`source.contains(quote)` (line 177). First failure short-circuits to
`Unchanged { reason }` with the reason truncated to `REASON_CHARS_MAX` = 512
chars (lines 200–205). Only a full pass builds `CompactReceipt { summary,
quotes, source_bytes, verified_quotes }` (lines 184–189), with the invariant
`verified_quotes == quotes.len()` made observable as a field.

The gauntlet side: `src/tasks/task_12.rs` includes that file verbatim and
runs five checks (one control, three attacks, one opt-in gate), each a small
function returning `Result<String, String>`; `run()` aggregates them into
`TaskOutcome::Pass`/`Fail` with bounded evidence via
`crate::bound_evidence`. A side effect worth knowing: because the include
carries the reducer's own `#[cfg(test)] mod tests`, `cargo test -p
phlow-gauntlet --lib` also runs the reducer's 10 shipped unit tests — they
are the real module's tests, not duplicates, and they pass.

One stub inconsistency found and corrected: the `task-12` stub declared
`KIND = TaskKind::NvimLua` (the default for unimplemented stubs), while the
task brief specifies a Rust-kind task (no Lua driver, no nvim). The module
now declares `KIND = TaskKind::Rust`; ID (`task-12`) and NAME (`poisoned
context compaction`) are unchanged.

## Sources

- Primary: `crates/phlow-agent/src/solpi/reducer.rs` — opt-in gate
  (lines 165–167), bounds check ordering (line 168), empty-quote rejection
  (lines 171–176), byte-for-byte check (line 177), first-mismatch rejection
  (lines 178–181), bounded reason (lines 200–205), bounds constants (lines
  24–39). Repo `~/workspace/repos/phlow` @ `87c182d`.
- Secondary: SoL-Pi paper arXiv 2609.20519 and the NVlabs SoL-Pi blog/REAMDE
  (the "Evidence-Preserving Reducer" concept this code ports); the phlow
  memory record of the 2026-09-26 SoL-Pi port direction ("compact receipts
  only when every retained quote matches the source; opt-in/disabled by
  default; failures leave results unchanged").
- This task's files: `crates/phlow-gauntlet/src/tasks/task_12.rs`,
  `crates/phlow-gauntlet/tests/task_12.rs`.
