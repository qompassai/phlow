# task-141: evidence preservation + chain of custody

**Kind:** rust · **Status:** pass · **Wave:** 25 · **Commits:** pending (wave 141-145)

## ELI5

When the prober finds something, it saves the tool's raw output exactly
as it arrived — byte for byte — and stamps it with a fingerprint (a
SHA-256 hash). Then, every time someone handles that evidence (the
validator checks it, the reporter writes it up, the operator approves
it, the submitter sends it), they sign a logbook saying who they are,
what they did, and when. If anyone changes even one byte afterwards,
the fingerprint won't match anymore, and the next check catches it —
the finding gets locked in quarantine for a human to look at, not
quietly thrown away. And if the tool spits out a gigantic blob (100
MB), we keep only the first 10 MB and mark it "truncated" instead of
running out of memory.

## What this task attempts

- **Goal:** prove the scaffold's `Evidence` type preserves tool output
  byte-exact, grows an append-only custody chain per handling step,
  detects tampering with a typed error before the finding can advance,
  and bounds oversize blobs at a named cap.
- **Mechanism:** `crates/phlow-gauntlet/src/tasks/task_141.rs` drives
  `crate::bounty::types::{Evidence, CustodyEntry, Finding,
  FindingState}` and `crate::bounty::approve::sha256_hex` (the same
  hash the submission gate binds approvals to). The driver's
  `seal_evidence` / `append_custody` / `verify_custody` /
  `Quarantine` helpers implement the custody discipline on top of the
  scaffold types.
- **Success criterion:** byte-exact round-trip with recorded sha256;
  chain length == handling steps with every entry binding the sealed
  hash; a flipped byte yields `EvidenceError::Tampered` and the finding
  is quarantined with evidence intact, never advancing past Candidate;
  100 MiB input stores exactly 10 MiB with the `truncated` marker and
  the seal entry naming the cap.
- **Non-goals:** judging the finding (task 142), rejecting it (task
  143), or rendering/submitting it (tasks 144/145). The scaffold's
  `Evidence` struct itself is taken as-is (see scaffold notes below).

## What happened

All four cases passed on the first attempt against scripted fixtures
(MOCK tool output):

- **byte_exact_roundtrip (V1):** sealed the synthetic tool output
  fixture; stored bytes == input bytes; `sha256` equals an independent
  recomputation; `truncated == false`; exactly one seal custody entry;
  fresh seal passes `verify_custody`.
- **custody_chain_grows (V2):** walked the legal state machine
  Candidate → Validated → Reportable → Approved → Submitted on the
  `ManualClock`, appending a custody entry per step. Chain length 5
  (1 seal + 4 steps); every entry's `evidence_sha256` equals the sealed
  hash; timestamps strictly increasing; verification green after every
  step.
- **tamper_detected (A1):** flipped one byte of `raw`; the next
  `verify_custody` returned `Err(EvidenceError::Tampered)` naming the
  expected vs recomputed hashes; the finding was quarantined with its
  (tampered) evidence intact and stayed `Candidate` — it cannot advance
  on tampered evidence.
- **oversize_truncated (A2):** 100 MiB synthetic input stored exactly
  10 MiB (`EVIDENCE_CAP_BYTES`); `truncated == true`; sha256 covers the
  stored bytes; the seal entry reads
  `sealed-truncated(cap=10485760B)` — the marker names the cap.

Verdict: **replicates** — the custody discipline holds on all four
scenarios.

## The fix — what changed and why

No fix iterations: the first attempt passed. The driver was written
against the scaffold's `Evidence`/`CustodyEntry` types directly, with
the seal/append/verify/quarantine logic living in the driver (disjoint
from sibling waves; the scaffold is shared and read-only for this
wave).

## Scaffold notes (reported, not fixed)

The design doc describes a richer scaffold than what landed: `types.rs`
has no typed error set (`EvidenceTampered` among others) and no
evidence seal/verify constructors, so the driver defines
`EvidenceError::Tampered` and the seal/verify helpers locally. This is
a gap the coordinator should know about, not a blocker — the driver's
local implementation is the honest test of the discipline.

## Full technical depth

The chain-of-custody discipline has three parts. **Sealing** happens
once, at evidence creation: `seal_evidence` takes the raw bytes by
borrow (no ownership games), truncates to `EVIDENCE_CAP_BYTES`
(10 MiB, a named constant with units) when oversized, hashes the
*stored* bytes with the scaffold's `sha256_hex`, and opens the custody
chain with a `sealed` (or `sealed-truncated(cap=…B)`) entry. Hashing the
stored bytes rather than the input is load-bearing: the hash must
verify what is actually kept, and the marker names the cap so a
consumer can see truncation happened without re-deriving it.

**Appending** happens per handling step: `append_custody` pushes a
`CustodyEntry { handler, action, at, evidence_sha256 }` where the hash
is the *current* sealed hash. Because every entry binds the same hash,
any mutation of `raw` breaks every entry at once — there is no
"re-seal to cover the tamper" path in the driver, and the scaffold
gives none either.

**Verification** recomputes `sha256(raw)` and compares it to the sealed
`evidence.sha256` and to each entry's `evidence_sha256`. The first
mismatch returns `EvidenceError::Tampered { expected, got }` — a typed
error carrying both hashes, so the quarantine record is auditable. The
finding is then *held*, not dropped: the driver's `Quarantine` clones
the finding's id, fingerprint, reason, and evidence into an
append-only, fingerprint-queryable list. The finding itself stays
`Candidate`; the state machine has no quarantine state, and none is
invented — quarantine is deliberately a driver-side holding area, kept
separate from the lifecycle states.

The oversize case is the DoS bound: a 100 MiB tool-output blob (a
realistic pathological case — a verbose scanner dumping a whole
response body) is cut to the cap *before* hashing and storage, so the
stored blob is exactly 10 MiB and the process never holds the full
input longer than the fixture requires. The `truncated` flag plus the
cap-naming marker keep the truncation visible downstream (task 144's
renderer can surface it; task 145's content hash covers the stored
bytes, so a truncated evidence blob still has a stable identity).

## Sources

- NIST SP 800-86, *Guide to Integrating Forensic Techniques into
  Incident Response* — the chain-of-custody model: preserve original
  media, document every handling step, verify integrity by hashing.
  (Primary: the discipline this task mechanizes.)
- The scaffold itself: `crates/phlow-gauntlet/src/bounty/types.rs`
  (`Evidence`, `CustodyEntry`, `Finding`, `FindingState`),
  `crates/phlow-gauntlet/src/bounty/approve.rs` (`sha256_hex` — the
  same hash the submission gate binds approvals to, so custody hashes
  and gate hashes are comparable values).
- Design doc: `~/workspace/gauntlet-design-tasks-131-150.md`, Wave 25,
  task-141 (the four scenarios and pass criteria this driver
  implements verbatim).
