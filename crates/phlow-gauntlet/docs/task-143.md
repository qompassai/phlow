# task-143: false-positive rejection with reason

**Kind:** rust · **Status:** pass · **Wave:** 25 · **Commits:** pending (wave 141-145)

## ELI5

Task 142 is the judge; this task is what happens to the condemned. A
finding that looks plausible but fails validation isn't just thrown
away — it's rejected *with the reason written down*, and parked in a
quarantine locker where the operator can audit it later, evidence and
all. The rejection sticks to the finding's fingerprint (its identity),
so if the same false positive shows up again next cycle, it gets
rejected again instead of sneaking back into the pipeline — no
resurrection loops.

## What this task attempts

- **Goal:** prove the rejection path records its reason verbatim,
  quarantines rejects with evidence intact, keeps the quarantine
  queryable for operator audit, attributes rejection to exactly the
  failed check, and keeps rejection sticky across cycles by
  fingerprint.
- **Mechanism:** `crates/phlow-gauntlet/src/tasks/task_143.rs` drives
  the task-142-style pipeline (four checks) plus
  `crate::bounty::store::FindingStore` (fingerprint-keyed, sticky) and
  the driver's `Quarantine` (append-only, fingerprint-queryable). The
  `reject_and_quarantine` helper walks Candidate → Rejected through
  the legal state machine, stamps `reject_reason`, and holds the record
  in quarantine.
- **Success criterion:** reason recorded verbatim on record and
  quarantine entry; quarantine evidence == finding evidence;
  quarantine lists every rejection in order; an FP differing from a TP
  only in `evidence` is rejected on exactly `evidence-present`;
  re-inserting the fingerprint next cycle creates no new record, keeps
  state `Rejected` and the reason intact.
- **Non-goals:** the pipeline mechanics themselves (task 142), report
  rendering (task 144). Quarantine is deliberately driver-side: the
  scaffold's state machine has no quarantine state, and none is
  invented.

## What happened

All four cases passed on the first attempt against scripted fixtures
(MOCK):

- **rejection_records_reason (V1):** a single-observation finding
  failed `reproducible`; rejected with reason `"not-reproducible"`;
  the stored record is `Rejected` with `reject_reason ==
  Some("not-reproducible")` verbatim; the quarantine entry carries the
  same reason and byte-identical evidence.
- **quarantine_queryable (V2):** two rejections
  (`not-reproducible`, `evidence-empty`) list in rejection order with
  verbatim reasons; lookup of an unknown fingerprint returns `None`
  (no phantom entries).
- **empty_evidence_rejected (A1):** the TP (full evidence) validates
  `Ok`; the FP — identical except `evidence.raw` empty — fails on
  exactly `evidence-present`. The fixtures were asserted to differ in
  no other field, so the attribution is precise.
- **rejection_sticky_across_cycles (A2):** cycle 1 rejected
  `fp-143-sticky`; cycle 2 re-inserted the same fingerprint
  (re-titled, as a re-observation would be) → `(same_id, is_new =
  false)`, record count unchanged, state still `Rejected`, reason
  intact, `observation_count` bumped to 2 (the re-observation is
  counted, the verdict is not resurrected).

Verdict: **replicates** — rejection is attributed, auditable, and
sticky.

## The fix — what changed and why

No fix iterations: the first attempt passed.

## Full technical depth

The rejection path has three moves, and the order matters.
**Classify**: the pipeline's `Err((check_name, Fail { reason }))`
names the failed dimension — task 142's attribution work is what makes
the reason *true*, this task makes it *durable*. **Record**: the driver
stamps `reject_reason` on the `Finding` *before* `FindingStore::insert`
(the store takes the finding by value and assigns the id; the reason
rides along), then transitions the stored record Candidate → Rejected
through the legal machine — a terminal state with no exits, so a
rejected finding cannot be transitioned back into the pipeline by
mistake. **Hold**: the quarantine clones the record's id,
fingerprint, reason, and evidence into an append-only list.

Stickiness falls out of the store's fingerprint keying, and the A2 case
verifies the exact property that matters: `insert` on a known
fingerprint bumps `observation_count` on the *existing* record and
returns `(existing_id, false)` — it never resets state, never clears
the reason, never creates a second record. The re-observation is still
counted (observation_count 1 → 2), which is correct: the system
remembers it saw the thing again, and remembers it already decided.
This is what prevents FP resurrection loops across cycles: without
stickiness, every cycle would re-spend validation budget re-rejecting
the same false positive, and a flaky check could eventually admit it.

The quarantine's queryability is the operator's audit surface: `list()`
returns rejections in order with verbatim reasons, `get(fingerprint)`
answers "what happened to this one". The A1 case closes the loop on
attribution precision: because the TP and FP fixtures differ in
exactly one field, the `evidence-present` verdict is provably about
that field — the rejection reason is evidence, not a guess.

One deliberate non-invention: quarantine lives in the driver, not in
the finding state machine. The scaffold's lifecycle has no quarantine
state, and adding one would change the machine every other task
depends on. The holding area is a separate structure with its own
query API; the finding's terminal `Rejected` state is the machine's
answer, quarantine is the operator's.

## Sources

- HackerOne triage practice: "not-applicable" reports are closed *with
  a reason*, kept visible to the reporter — rejection without a reason
  is indistinguishable from loss. (Primary: the verbatim-reason rule;
  cited by the design doc.)
- The scaffold itself: `crates/phlow-gauntlet/src/bounty/store.rs`
  (`FindingStore`: fingerprint-keyed insert, sticky rejection,
  `transition` through the legal machine),
  `crates/phlow-gauntlet/src/bounty/types.rs` (`FindingState::Rejected`
  is terminal — no exits).
- Design doc: `~/workspace/gauntlet-design-tasks-131-150.md`, Wave 25,
  task-143.
