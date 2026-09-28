# task-187: ranked fuzzy queries

**Kind:** rust · **Status:** pass · **Wave:** 30 · **Commits:** pending (wave 30)

## ELI5

Once sessions are in the database, the user types a few words and
expects the best match on top — like a search engine, but for past
agent sessions. The engine first asks SQLite for sessions whose
title or transcript contains every word (a fast, dumb filter), then
re-ranks those candidates with a small fuzzy scorer written in Rust:
a word match in the title beats a word match in the transcript, an
exact word beats a fuzzy subsequence, and ties always break by
session id so the order never shuffles between runs. This task proves
the ranking works on a 200-session corpus and that repeating the
same query ten times gives byte-identical results.

## What this task attempts

- **Goal:** prove ranked search returns the best match first and is
  deterministic across repeated queries.
- **Mechanism:** `session_find.rs` (`SessionIndex::search`,
  `fuzzy_score`, `term_tier`, `search_sql`);
  driver `src/tasks/task_187.rs`.
- **Success criterion:** query "pairing brute force" → the session
  titled with exactly those words is rank 1 (with distractors
  present); 10 identical queries → byte-identical rankings.
- **Non-goals:** incremental rescan (task 186), literal query handling
  (task 191), concurrency (task 192).

## What happened

Pass on the first executable gate run (the wave's shared compile blockers were fixed before any test executed). Integration tests 2/2:
`target_ranks_first` (rank 1 = `target-pairing-brute-force`, ≥2
distractor hits so the ranking is genuinely exercised) and
`rankings_byte_identical` (10/10 runs byte-identical). ~0.03s on
primo.

## Where it went wrong

One real design bug was caught and fixed before gating:

- **Stage:** driver corpus construction (pre-gate, local reasoning).
- **Symptom:** the initial corpus put the target's words only in its
  title and gave distractors *different* words. The SQL prefilter is
  a conjunction — every term must appear in title OR transcript —
  so distractors containing none of the terms never reached the
  scorer. The "ranking" was never actually exercised.
- **Root cause:** the prefilter (`search_sql`) narrows candidates
  before `fuzzy_score` ever runs; a corpus where only the target
  survives the prefilter proves nothing about ordering.

(The pre-gate E0521 lifetime fix is documented in task-186.md and
applied identically here.)

## The fix — what changed and why

- **Changed:** `src/tasks/task_187.rs` (`pairing_corpus`): every
  distractor transcript now contains all three query words
  ("pairing", "brute", "force") in prose, while the target keeps
  the exact title "pairing brute force notes".
- **Commit:** pending (wave 30).
- **Why:** with all three words present in every distractor's
  transcript, the prefilter returns the target plus distractors and
  the scorer must actually order them. The target still wins because
  title matches outscore transcript matches by design (title exact
  300 > transcript exact 100 per term). The alternative — weakening
  the prefilter — would have made the test easier while testing
  less.
- **Source:** `session_find.rs` `search_sql` (conjunction prefilter)
  and `fuzzy_score` tier weights; the driver asserts the distractor
  count explicitly (`hits >= 2`).
- **Validation agents:** `cargo test -p phlow-gauntlet --test task_187`
  on primo → 2/2 pass; rank-1 assertion plus distractor-presence
  assertion both green.
- **Adversarial agents:** the distractor corpus itself is the
  adversarial half — 199 sessions all containing the query words,
  forcing the scorer to distinguish title-exact from
  transcript-only.
- **Citations:** none beyond the engine source; the scoring contract
  is defined in `session_find.rs` module docs.

## Full technical depth

`search` splits the query into at most `MAX_QUERY_TERMS` (16)
whitespace-separated terms (queries over `MAX_QUERY_LEN` = 512
bytes are rejected). `search_sql` builds a conjunction: for each
term, `(title LIKE '%<escaped>%' ESCAPE '\' OR transcript LIKE
'%<escaped>%' ESCAPE '\')`, ANDed across terms. LIKE metacharacters
(`\`, `%`, `_`) are escaped by `escape_like`, so terms are always
literal. Up to `MAX_RESULTS` (100) candidate rows come back; each
is scored by `fuzzy_score` in Rust: per term, the best tier across
title-then-transcript (title exact-word 300, title substring 200,
title subsequence 100; transcript exact 100, substring 60,
subsequence 30), summed, ties broken by `id` ascending. The
prefilter keeps SQLite doing the coarse work; the scorer only ever
sees ≤100 rows, so ranking cost is bounded.

Determinism: scoring is pure (no hash maps iterated, no random
state), the sort key is `(score DESC, id ASC)` — a total order —
and the corpus is fixed. The driver serializes all 10 rankings to
strings and requires byte equality, which would catch any
nondeterministic tie-break.

## Sources

- Primary: `crates/phlow-gauntlet/src/session_find.rs`
  (`SessionIndex::search`, `search_sql`, `escape_like`,
  `fuzzy_score`, `term_tier`, `query_terms`);
  `crates/phlow-gauntlet/src/tasks/task_187.rs`;
  `crates/phlow-gauntlet/tests/task_187.rs`.
- Secondary: Ghostex `packages/find/src/lib.rs` (ranked-query
  shape); Ghostex `fuzzy.rs` (subsequence tier idea — re-implemented,
  not ported).
