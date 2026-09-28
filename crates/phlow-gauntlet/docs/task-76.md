# task-76: tokenizer boundary mismatch

**Kind:** rust (adversarial) · **Status:** fail (open) · **Wave:** 76–80 · **Commits:** pending (wave 76-80)

## ELI5

Token budgets are "how much the model may read and write, counted in the model's own currency." The design wants those budgets counted with the SERVING model's tokenizer — because the model charges in its tokens, not our characters. Phlow has no tokenizer at all. It counts characters: the runtime ends a role with "Context budget exhausted" when the serialized message CHARACTERS exceed the cap, truncation cuts at character boundaries, and the only token-unit number phlow produces — `max_tokens = min(8192, context_length / 2)` — is an operator-configured number with no tokenizer behind it, sent to Ollama in token units with no "this is an estimate" flag. The serving model's tokenizer lives inside Ollama and is invisible to phlow: there is no tokenizer X, so there is no X/Y divergence to warn about and nothing to re-encode truncation through.

## What this task attempts

- **Goal:** verify budgets and truncation are computed with the serving model's tokenizer — X/Y divergence measured and surfaced, truncation re-encoded through the serving tokenizer, the ledger never presenting X-counts as Y-counts — or document the gap.
- **Mechanism:** `src/tasks/task_76.rs` probes the REAL seams directly: char_budget_is_the_ruler (char count trips the budget exactly at the cap), char_truncation_splits_tokens (two mock tokenizers diverge on the same fixture; the char cut is a token boundary for neither), max_tokens_is_configured_not_tokenized (`max_tokens_for` == the formula, no tokenizer), ledger_never_shows_token_units (no budget/usage/payload field presents token units). Plus an exact-token source scan for `tokenizer` (zero hits in product crates) and a Cargo.toml dependency scan (no tokenizer crate).
- **Success criterion:** a tokenizer binding behind counting/truncation, or the gap documented as the finding (and banked for Matt as a product decision).
- **Non-goals:** inventing the tokenizer binding on gauntlet authority (it is a product decision, not a bug fix).

## What happened

Honest FAIL at `where = "seam"`, first attempt — the absence IS the finding:

- **V1:** the char budget is the ruler — serialized chars exceed `max_context_chars` → "Context budget exhausted", exactly at the boundary, independent of any tokenization.
- **V2:** char-boundary truncation splits tokens — mock tokenizers X and Y count the same fixture differently and the char cut aligns with neither; the divergence is measured by the mocks, never detected or repaired by phlow.
- **A1:** `max_tokens_for(context_length) = min(8192, context_length / 2)` — an operator-configured number with no tokenizer behind it, presented in the token-unit payload field with no estimate flag.
- **A2:** the ledger never shows token units — no budget, usage, or payload field presents X-counts as Y-counts, because there is no X to present.

## Full technical depth

The seams are `crates/phlow-runtime/src/runtime.rs::role_turn` (`must_dumps(messages).chars().count() > budgets.max_context_chars` → "Context budget exhausted"), `crates/phlow-agent/src/memory.rs::truncate_chars` (char-boundary truncation of retrieved context), and `crates/phlow-llm/src/payload.rs::max_tokens_for` (the configured number, not re-exported at the crate root — reachable as `phlow_llm::payload::max_tokens_for`). The exact-token scan for `tokenizer` over every product crate's `src/**/*.rs` (the gauntlet crate excluded — its own probes use the design vocabulary) finds zero hits; no workspace Cargo.toml depends on `tiktoken`, `tokenizers`, `sentencepiece`, or `hf-hub`. The danger the design names is real in principle: a char budget and a token budget are different currencies, and phlow silently spends one while the model charges the other — but with no binding there is no divergence to measure, only the documented absence.

Banked for Matt (product decision, NOT auto-implemented on gauntlet authority): whether phlow should gain a tokenizer binding — budgeting with the serving model's tokenizer, re-encoding truncation through it, and warning on X/Y divergence. That is a new product feature, not a bug fix; the gauntlet documents the gap and stops.

## Sources

- `~/workspace/repos/phlow/crates/phlow-runtime/src/runtime.rs` — `role_turn` char-count budget
- `~/workspace/repos/phlow/crates/phlow-agent/src/memory.rs` — `truncate_chars`
- `~/workspace/repos/phlow/crates/phlow-llm/src/payload.rs` — `max_tokens_for`
- `~/workspace/gauntlet-design-tasks-71-100.md` — task-76 design (Wave 76–80)
