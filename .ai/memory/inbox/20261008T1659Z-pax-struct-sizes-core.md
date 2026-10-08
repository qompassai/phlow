# Struct-size audit: phlow-autoresearch + core records (2026-10-08)

phlow-autoresearch: audited and deliberately unchanged. The loop's
hard budget is ITERATIONS_MAX = 32, so every record is a tens-count
struct (LedgerEntry 224 B, ChangeSet 104 B, Evaluation 80 B), each
already at its layout floor — and LedgerEntry's field order is the
ledger hash contract besides. Nothing to shrink, nothing pinned.

phlow-agent (core): the genuinely numerous records are also already
at their floors, so the audit landed compile-time layout pins:

- ScoredCandidate 16 B (best_of_n.rs) — up to CANDIDATES_MAX
  (100_000) per selection; the most numerous record in the core
- Trajectory 56 B (best_of_n.rs) — up to 100_000 per verify call
- RankEntry 40 B (system1.rs) — up to CANDIDATES_MAX (4096) per
  ranked answer

Audited and rejected as tens-count: ChatMessage (context cap 40),
CompactStep (compaction cap 64), MemoryEntry (retrieval pages, not a
stored collection), Verification and SelectionReport (one per call).

Gates: phlow-agent tests 75 passed / 0 failed (1 pre-existing
ignore); phlow-autoresearch 28/28, untouched by this change; clippy
--all-targets -D warnings clean; rustfmt clean on touched files.
