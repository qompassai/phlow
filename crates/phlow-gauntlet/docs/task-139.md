# task-139: finding deduplication across cycles

**Kind:** rust · **Status:** pass with documented NEGATIVE (A2) · **Wave:** 24

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
fingerprint (a hash collision), the design demands both be kept with a
recorded tiebreak — the scaffold can't do that yet, and this task
documents the gap honestly instead of pretending.

## What this task attempts

- **Goal:** the same finding observed in two cycles is one record with two observations — never two records.
- **Mechanism:** the scaffold's `FindingStore::insert` in `crates/phlow-gauntlet/src/bounty/store.rs`, driven with synthetic findings in `src/tasks/task_139.rs` (MOCK).
- **Success criterion:** V1 one record, `observation_count == 2`; V2 two records; A1 near-duplicates (same title, different fingerprint) → two records, no fuzzy merge; A2 forced collision → deterministic tiebreak recorded, both findings preserved.
- **Non-goals:** the re-probe that *produces* the duplicate (task 138's job) — this is the store absorbing it.

## What happened

V1, V2, A1 pass on the first attempt: repeat observation yields one
record with `observation_count == 2`; distinct findings yield two
records; same-title/different-fingerprint yields two records (no fuzzy
merge — the deliberate precision choice). A2 is a documented NEGATIVE:
with the fingerprint string forced equal but title/body different, the
second `insert` returns `(first_id, is_new=false)`, the stored title
stays the first finding's, and the second finding's content is
unrecoverable from the store. The scaffold keys *solely* on the
fingerprint string and never compares content, so a colliding
fingerprint with different content is silently absorbed — data loss.
The case measures this exactly, classifies it `negative` (task-110
precedent: a classified verdict is a successful measurement), and the
integration test pins `verdict == "negative"` so a future scaffold
fix trips it loudly.

## Where it went wrong

- **Stage:** A2 — fingerprint collision with differing content.
- **Symptom:** `insert` of finding B (same fingerprint string as A,
  different title/body) returns `(id_of_A, false)`; `get_by_fingerprint`
  still shows A's title; B's title, body, and evidence hash are gone
  from the store. `observation_count` reads 2, misattributing B as a
  re-observation of A.
- **Evidence:** `insert A -> (f000001, is_new=true); insert B -> (f000001, is_new=false)`; stored title `"title-A"` ≠ `"title-B"`.
- **Root cause:** `FindingStore::insert` (`store.rs`) uses the fingerprint
  string as the sole `HashMap` key and performs no content comparison
  on a hit. A true SHA-256 collision is infeasible, but the design's
  threat model includes weaker/older fingerprint schemes and
  adversarially supplied fingerprints; under that model the store
  silently drops findings. Verified by reading the code path, not
  guessed.

## The fix — what changed and why

No fix applied: the scaffold is shared by four waves and the brief
forbids worker-side fixes. Reported as a scaffold bug to the
coordinator (see below). Suggested direction, not implemented: on a
fingerprint hit with differing canonical content, keep both records
(e.g. a collision chain keyed by content hash) and record which won
the deterministic tiebreak — never silently absorb.

## Full technical depth

Dedup-by-content-hash is the git object model: identical content →
identical key → one stored object (Pro Git, "Git Objects"). The
precision rule (no fuzzy merge) mirrors that model exactly — git never
merges two objects because their messages look similar. The collision
arm is the interesting one: git's model assumes a strong hash; the
gauntlet's design deliberately weakens the assumption ("forced via
fixture") to test the store's behavior when the key lies. The
scaffold's `insert` treats key-equality as content-equality, which is
only valid under the strong-hash assumption. The honest verdict is
therefore NEGATIVE with a bug report, not a faked pass. The sticky
rejection rule (`Rejected` fingerprints stay rejected, task 143's
territory) is unaffected.

## Sources

- Primary: `crates/phlow-gauntlet/src/bounty/store.rs` (`FindingStore::insert`); Pro Git, chapter 10 "Git Internals — Git Objects" (content-addressed storage, the dedup-by-hash mental model).
- Secondary: the gauntlet design doc, task-139 A2 (the tiebreak requirement the scaffold does not meet).
