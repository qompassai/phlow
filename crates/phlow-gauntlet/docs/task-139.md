# task-139: finding deduplication across cycles

**Kind:** rust · **Status:** pass (A2 tiebreak implemented) · **Wave:** 24

## ELI5

The same bug found in two different weekly cycles should be one entry
in the notebook with "seen twice" written on it — not two entries.
This task checks the notebook's dedup rule: each finding gets a
fingerprint (a hash of what makes it *that* finding), and the store
files by fingerprint. Same fingerprint twice → one record, observation
count 2. Different fingerprints → two records. Two findings with the
same title but different fingerprints are *not* merged — deciding two
similar-looking bugs are the same bug is the human operator's job, not
the machine's. And if two *different* findings ever got the same
fingerprint (a hash collision), the store keeps both: the dedup key is
the composite (fingerprint, content-hash), so the collision is a
deterministic tiebreak — two records, both contents preserved — while
a byte-identical re-observation still merges into one record.

## What this task attempts

- **Goal:** the same finding observed in two cycles is one record with two observations — never two records.
- **Mechanism:** the scaffold's `FindingStore::insert` in `crates/phlow-gauntlet/src/bounty/store.rs`, driven with synthetic findings in `src/tasks/task_139.rs` (MOCK).
- **Success criterion:** V1 one record, `observation_count == 2`; V2 two records; A1 near-duplicates (same title, different fingerprint) → two records, no fuzzy merge; A2 forced collision → deterministic tiebreak recorded, both findings preserved.
- **Non-goals:** the re-probe that *produces* the duplicate (task 138's job) — this is the store absorbing it.

## What happened

V1, V2, A1 pass on the first attempt: repeat observation yields one
record with `observation_count == 2`; distinct findings yield two
records; same-title/different-fingerprint yields two records (no fuzzy
merge — the deliberate precision choice). A2 was a documented NEGATIVE
(a pinned test held it) until the scaffold gained the composite-key
tiebreak: with the fingerprint string forced equal but title/body
different, the second `insert` now returns a fresh id
(`is_new=true`), the store holds two records under the fingerprint,
and both titles are recoverable via `findings_for`. A byte-identical
re-observation of A still merges (`is_new=false`, same id,
`observation_count` bumps). The integration test now pins
`verdict == "replicates"`, `records == 2`, `finding_b_preserved`,
and `duplicate_merged`.

## Where it went wrong

- **Stage:** A2 — fingerprint collision with differing content.
- **Symptom (historical, pre-fix):** `insert` of finding B (same
  fingerprint string as A, different title/body) returned
  `(id_of_A, false)`; `get_by_fingerprint` still showed A's title; B's
  title, body, and evidence hash were gone from the store.
  `observation_count` read 2, misattributing B as a re-observation of
  A. Fixed by the composite-key tiebreak above.
- **Evidence:** `insert A -> (f000001, is_new=true); insert B -> (f000001, is_new=false)`; stored title `"title-A"` ≠ `"title-B"`.
- **Root cause:** `FindingStore::insert` (`store.rs`) uses the fingerprint
  string as the sole `HashMap` key and performs no content comparison
  on a hit. A true SHA-256 collision is infeasible, but the design's
  threat model includes weaker/older fingerprint schemes and
  adversarially supplied fingerprints; under that model the store
  silently drops findings. Verified by reading the code path, not
  guessed.

## The fix — what changed and why

The scaffold bug is fixed on branch `fix/findingstore-collision`
(unpushed). `FindingStore` now keys on the composite (fingerprint,
content_hash):

- `by_fingerprint: HashMap<String, Vec<Finding>>` — one bucket per
  fingerprint; the bucket holds every distinct-content record.
- `content_hash(finding)`: deterministic sha256 (via the crate's
  `approve::sha256_hex` — never `DefaultHasher`, whose per-instance
  random seed would silently break cross-insert comparison) over the
  canonical field sequence `target_id, fingerprint, title,
  evidence.sha256`, each length-prefixed in fixed order. Excluded:
  `id` (store-assigned), `state` (lifecycle), `observation_count`
  (the accumulator), `reject_reason` (post-validation), custody
  entries (volatile `at` timestamps), `evidence.truncated`, and
  `evidence.raw` (covered by `evidence.sha256`).
- `insert`: same (fingerprint, content_hash) → bump
  `observation_count`, return `(existing_id, false)`; same fingerprint
  with different content hash → fresh `f{:06}` id, distinct record,
  return `(new_id, true)`.
- `get_by_fingerprint` (ambiguous under collision) is replaced by
  `findings_for(fp) -> &[Finding]` plus id-based `transition`, which
  returns a typed `StoreTransitionError::UnknownId` instead of
  panicking. `NonDuplicateCheck` now fails only on same
  (fingerprint, content-hash) with a different record id.
- No persistence exists (the store is in-memory only; no serde derives,
  no file writes), so no migration was needed.
- Task-143's A2 fixture now re-submits content-identical findings:
  under the composite key a retitled re-observation is different
  content (a new record), so "the same false positive" means identical
  content for the stickiness case. Task-143's design claim is updated
  to "sticky per (fingerprint, content-hash)".

## Full technical depth

Dedup-by-content-hash is the git object model: identical content →
identical key → one stored object (Pro Git, "Git Objects"). The
precision rule (no fuzzy merge) mirrors that model exactly — git never
merges two objects because their messages look similar. The collision
arm is the interesting one: git's model assumes a strong hash; the
gauntlet's design deliberately weakens the assumption ("forced via
fixture") to test the store's behavior when the key lies. The
scaffold's `insert` treated key-equality as content-equality, which is
only valid under the strong-hash assumption. The honest verdict was
therefore NEGATIVE with a bug report, not a faked pass — and the fix
above is the tiebreak the design demanded: the store now compares
content on a fingerprint hit instead of assuming it. The sticky
rejection rule (`Rejected` findings stay rejected, task 143's
territory) is preserved per (fingerprint, content-hash).

## Sources

- Primary: `crates/phlow-gauntlet/src/bounty/store.rs` (`FindingStore::insert`); Pro Git, chapter 10 "Git Internals — Git Objects" (content-addressed storage, the dedup-by-hash mental model).
- Secondary: the gauntlet design doc, task-139 A2 (the tiebreak requirement the scaffold does not meet).
