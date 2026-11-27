# SoL-Pi decisions — `phlow-agent`

Concept re-expressions of two NVlabs/SoL-Pi mechanisms for the agent
layer. Nothing here is a port: the ideas are re-expressed in Tiger Style
Rust against phlow's own agent model. Upstream is TypeScript for the Pi
coding agent; this crate has no Pi dependency.

## Verified upstream sources

- Repository: <https://github.com/NVlabs/SoL-Pi> (MIT License; verified
  2026-09-27). README defines the four mechanisms, states "every
  mechanism is opt-in and disabled by default" and "a missing
  configuration leaves every mechanism disabled", and lists the shared
  rules: no Pi patches, explicit opt-in, preserve evidence ("original
  observations remain available locally, and reducer failures leave the
  original result unchanged").
- Blog: <https://nvlabs.github.io/SoL-Pi/> — technical details, design
  rationale, and core insights behind the four mechanisms.
- Paper: arXiv 2609.20519, "SoL-Pi: Recursively Scaling Auto-Research
  Loops for Efficient Agent Harnesses".

## 1. Evidence-Preserving Reducer (`solpi/reducer.rs`)

**Upstream:** "Long diagnostic logs become compact receipts only when
every retained quotation matches the archived source." Reducer failures
leave the original result unchanged.

**Re-expression:** the caller proposes a reduction (`ReductionProposal`:
summary + retained quotations). `EvidenceReducer::reduce()` checks input
bounds, then verifies every quotation occurs in the source byte-for-byte
and is non-empty. The first failure yields
`ReductionOutcome::Unchanged` with a bounded reason — no partial
receipt, no edited receipt. The source is borrowed, never mutated, so
"unchanged" is structural, not a promise.

**Deliberate scope cut:** upstream may send eligible log content to a
configured reducer model. This port makes no model or network calls —
the proposal is caller-supplied and all verification is local, so no log
content can leave the process through this module. Remote reduction is
an exfiltration surface; cutting it is the point.

**Why empty quotations are rejected:** an empty string is contained in
every source, so accepting it would let a proposal claim evidence it
does not have.

**Opt-in:** `EvidenceReducer::new()` is disabled; `reduce()` returns
`ReducerError::NotEnabled` until `EvidenceReducer::opt_in()`.

## 2. Online Context Compact (`solpi/context_compact.rs`)

**Upstream:** "Completed plan steps become candidate points for Pi's
native compaction, subject to economic and window-pressure checks; after
a successful compaction, Pi continues the task in a new turn."

**Re-expression:** `evaluate_compaction()` measures a step snapshot
against an explicit cost model (`CompactionPolicy`) and returns a
`CompactionPlan` — the decision plus the reason, fully observable.
Compaction requires all three checks: window pressure ≥
`pressure_threshold` (default 0.75 of `window_bytes`), reclaimable bytes
≥ `min_savings_bytes` (default 4096), candidate count ≥
`min_candidate_steps` (default 2). Candidates are completed, unpinned
steps, first-seen order, capped at `CANDIDATES_MAX` (64). Pinned steps
(system prompts, safety context) are never candidates. The module only
decides; performing the compaction stays with the caller (upstream's
"new turn" continuation is Pi-specific and not re-expressed).

**Sizes are measured, not trusted:** `evaluate_compaction()` takes step
content and measures `content.len()` itself. There is no size field for
a caller to inflate, so adversarial pressure signals cannot force a
compaction.

**Opt-in:** `CompactionPolicy::disabled()` is the default;
`evaluate_compaction()` returns `PolicyError::NotEnabled` before
measuring anything when disabled. `with_thresholds()` rejects a zero
window and any non-finite or out-of-range pressure threshold at
construction.

## 3. Context API (`solpi/context_api.rs`) — Manuscript Phase 1 (2026-11-27)

**Authority:** the Manuscript Phase 0 record, section 6 (Owner GO
2026-11-27). Phase 0 replicated Devoto et al. (arXiv 2611.34147) in
miniature: the harness arm lost by *destruction* (compaction discarded
figures the model still needed; on the injection task it answered the
injected 0 after its real data was compacted away), and retrieval was
never used in 48 runs. This module is the model-facing half section 6
prescribed, built on section 2's engine — the engine still decides;
the API holds the state and performs.

**Re-expression:** `ContextLedger` holds live events, an archive, and
a **tally**. `collapse(start, end, summary, tag)` replaces an event
range with the model's summary and archives the originals under the
tag — compaction never discards, on either the model path or the
harness fallback (`harness_compact`, which runs `evaluate_compaction`
over the live snapshot and archives the oldest group-safe candidate
run). `retrieve(tag)` returns originals byte-for-byte. The task event
(id 0) and instruction events are pinned *by kind* — pinning is not a
caller flag, so tool traffic cannot acquire it — and no collapse range
may include a pinned event or split a tool group. The tally lives
outside the event stream: entries arrive only through explicit
`tally_write` calls carrying an author (`Model`/`Harness`), so an
instruction-shaped tool result has no path into the tally or the task.
The usage indicator is fed by true token counts recorded from the
serving stack (`record_true_prompt_tokens`) and surfaces the archive
index (tags + sizes) so retrieval is discoverable at need.
`admit_tool_results` budgets tool results per turn: whole, in order,
until the turn budget would break; the rest defer — never truncated.

**Honest scope:** the API buys answer survival under pressure, not
token economy. Phase 0 measured the managed arm at 5–10x tokens with
zero savings; no savings are claimed here.

**Opt-in:** `SolpiFeature::ContextApi` on `SolpiAgentConfig`, disabled
by default like every other mechanism.

**Witness:** gauntlet task-319 (collapse/retrieve round-trip; a
summary insufficient by construction, completable only through the
archive; the injection shape under a compaction storm; storm bounds),
plus the Phase 1 protocol re-run against Phase 0's numbers, recorded
in the Phase 1 record.

## Bounds (named, with units)

| Bound | Value | Rationale |
|---|---|---|
| `SOURCE_BYTES_MAX` | 1 MiB | One reduction cannot retain an unbounded log |
| `SUMMARY_BYTES_MAX` | 8 KiB | Receipts stay compact by construction |
| `QUOTES_MAX` | 64 | Quotation list stays reviewable |
| `QUOTE_BYTES_MAX` | 8 KiB | One quotation cannot smuggle a log |
| `WINDOW_BYTES_DEFAULT` | 256 KiB | Cost-model window for pressure |
| `PRESSURE_THRESHOLD_DEFAULT` | 0.75 | Compact only under real pressure |
| `MIN_SAVINGS_BYTES_DEFAULT` | 4096 B | Compaction must pay for itself |
| `MIN_CANDIDATE_STEPS_DEFAULT` | 2 | One step is not a compaction |
| `CANDIDATES_MAX` | 64 | Plan stays bounded |
| `EVENTS_MAX` (context_api) | 4096 events | Live list cannot grow without bound |
| `EVENT_BYTES_MAX` (context_api) | 256 KiB | One event cannot carry a corpus |
| `TALLY_ENTRIES_MAX` | 128 | The pinned tally stays reviewable at a glance |
| `TALLY_VALUE_BYTES_MAX` | 1024 B | A tally entry is a figure, not a document |
| `ARCHIVE_TAGS_MAX` | 64 | Archive index stays surfacable in the indicator |
| `COLLAPSE_SUMMARY_BYTES_MAX` | 64 KiB | A collapse summary cannot smuggle the range back in |
| `LIVE_TAIL_EVENTS` | 10 events | The working set is never a harness candidate |
| `TURN_RESULT_BUDGET_BYTES_DEFAULT` | 54,000 B | Half a 32K-token window per turn (Phase 0 density) |

## MITRE ATLAS mapping (v2026.05/2026.08 identifiers)

Covered, each with a mitigation and an adversarial test in-module:

- **AML.T0043 Craft Adversarial Data:** byte-for-byte quotation
  verification; a single changed byte aborts the reduction. Test:
  `reducer_rejects_single_byte_tampered_quote`.
- **AML.T0020 Poison Training Data / Training Data Poisoning:**
  fabricated or near-match quotations cannot enter a compact receipt —
  only exact source bytes are retained, and any mismatch leaves the
  original unchanged. Tests:
  `reducer_rejects_near_match_case_changed_quote`,
  `reducer_rejects_absent_quote`.
- **AML.T0046 Spamming ML System with Chaff Data:** input bounds on
  source, summary, quotation count and size; oversized input is a typed
  error before verification. Test: bound checks inside
  `reducer_rejects_near_match_case_changed_quote`; candidate cap in
  `compact_caps_candidate_list`.
- **AML.T0051 LLM Prompt Injection (indirect):** adversarial pressure
  signals (a full window of incomplete steps, inflated content) cannot
  force compaction — sizes are measured from content, candidates require
  completion, and pinned steps are exempt. Tests:
  `compact_ignores_full_window_of_incomplete_steps`,
  `compact_never_candidates_pinned_steps`.
- **AML.T0081 Modify AI Agent Configuration:** default-off; enabling
  requires an explicit `enable()`/`opt_in()` call; nonsense thresholds
  (NaN, infinite, ≤0, >1, zero window) fail at construction. Tests:
  `reducer_refuses_without_opt_in`, `compact_refuses_without_opt_in`,
  `compact_rejects_nonsense_thresholds_at_construction`, config
  `default_config_disables_everything`.

Out of scope, with rationale (not covered, not claimed):

- **AML.T0010 AI Supply Chain Compromise:** no new dependencies added
  (std only); supply-chain review is a repo-level concern.
- **AML.T0015 Evade AI Model / AML.T0018 Backdoor ML Model:** no model
  training, weights, or inference exist in these modules.
- **AML.T0054 LLM Jailbreak:** no LLM is in the reduction or compaction
  path by design (the remote-reducer scope cut).
- **AML.T0057 LLM Data Leakage / AML.T0025 Exfiltration via Cyber
  Means:** pure in-memory modules — no I/O, no network, no exfiltration
  channel; the remote-reducer cut removes the one upstream network
  surface.

## What was left out

- Upstream's remote reducer-model call (deliberate exfiltration-surface
  cut; documented above).
- Upstream's post-compaction "new turn" continuation (Pi-specific).
- Cross-run persistence of compaction state: plans are per-evaluation
  and stateless.
