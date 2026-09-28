# task-150: hostile scope-feed refusal

**Kind:** rust (adversarial) · **Status:** pass · **Wave:** 146–150 · **Commit:** pending (wave 26)

## ELI5

The scope feed is a list of places you're allowed to test, delivered regularly. Task 150 checks what happens when that delivery turns hostile: a new entry for a website you were never allowed to touch, a delivery with a forged signature, an entry that quietly widens "one small network" into "the entire internet", or an old delivery replayed to roll you back. In every case Phlow must refuse the whole delivery, write down what happened, and keep testing exactly the old list — never the attacker's.

## What this task attempts

- **Goal:** verify structural refusal of hostile scope snapshots: the snapshot is rejected with a typed error, the feed quarantined, the previous scope retained — for out-of-bounds targets, bad signatures, CIDR widening, and version replay.
- **Mechanism:** `src/tasks/task_150.rs` adds a driver-local ingestion layer over `ScriptedFeed` (a `SignedFeed` decorator carrying per-version signature tags — a toy keyed sha256, labeled MOCK, standing in for pinned-key verification). Ingestion enforces, in order: authenticity (tag check, before any target is parsed) → version monotonicity (TUF-style rollback protection) → enrollment bounds (allowed suffixes with dot-boundary matching; CIDR prefix floor; no widening of enrolled targets) → file. Four scenarios: `out_of_bounds_target_refused` (V1: `*.evil.com` vs an example.com-only enrollment, refused both on a self-reported hostile poll and on a snapshot carrying the target); `bad_signature_rejected_before_parse` (V2: wrong-key tag; the snapshot also carries a hostile target, and the refusal must be `BadSignature`, proving targets were never parsed); `cidr_widening_refused` (A1: `10.0.0.0/24 → 10.0.0.0/8` refused as a superset of the enrolled scope); `replayed_version_rejected` (A2: correctly signed v3 after v5 → `StaleVersion{got: 3, latest: 5}`).
- **Success criterion:** all four hostile snapshots rejected with typed errors; scope store unchanged (v5 throughout); quarantine log entries present.
- **Non-goals:** real signature cryptography (the toy tag is a stand-in, honestly labeled); the benign refresh path (task 131).

## What happened

PASS on the exact tree, all four scenarios:

- **V1:** the self-reported hostile poll → `FeedError::HostileTarget{value: "*.evil.com"}`; the v6 snapshot carrying `*.evil.com` → the same typed refusal from the ingestion layer (reason names the enrolled suffixes); scope stayed v5; two quarantine entries.
- **V2:** the wrong-key snapshot → `IngestError::BadSignature{version: 6}` — not `HostileTarget`, even though a hostile target sat in the snapshot. Authenticity is checked before parsing, so the refusal proves the targets were never inspected.
- **A1:** `10.0.0.0/8` → `HostileTarget` with reason "widens enrolled target 10.0.0.0/24"; the filed scope kept the /24.
- **A2:** the replayed v3 (valid targets, valid tag — the replay is the only offense) → `StaleVersion{got: 3, latest: 5}`; scope stayed v5.

## Full technical depth

The check order is the mechanism. Authenticity first: the version number inside a snapshot is only trustworthy once the snapshot is authenticated, so the tag is verified before the version or any target is read — this is also what makes V2's "rejected before parsing" guarantee structural rather than incidental. Version monotonicity second: a replayed snapshot is a rollback attack, and the TUF specification treats exactly this as an attack class ("check for a rollback attack… if the version… is not N+1, discard it, abort the update cycle, and report the rollback attack"). The enrollment bounds last: a target is in scope only if its host equals or is a dot-boundary subdomain of an enrolled suffix (`notexample.com` does not match `example.com` — the dot boundary is load-bearing), and a CIDR is allowed only at or below the enrolled prefix floor and only if it does not widen an already-enrolled target (the never-widen rule: a superset is not the enrolled scope).

Every refusal quarantines the feed *and* retains the previous scope — the two halves of fail-closed. Refusing the snapshot but clearing the scope would blind the workflow; keeping the scope but not quarantining would let the next poll retry the attack silently. The quarantine log records which check fired and why, so the refusal is auditable, not just silent.

Honest limitation: the signature tag is a keyed sha256, not a real signature scheme — it stands in for "the enrollment pins the feed key" at toy scale. Key rotation, threshold signing, and algorithm agility are all out of scope; what the task verifies is the *wiring* (a snapshot whose tag does not verify is rejected before parsing), not the cryptography.

Distinct from task 131 (benign refresh) and task 133 (benign revocation): this is the adversarial feed.

## Sources

- `crates/phlow-gauntlet/src/bounty/feed.rs` — `ScopeFeed`, `ScriptedFeed` (hostile/malformed injection fixtures)
- `crates/phlow-gauntlet/src/bounty/store.rs` — `ScopeStore::file` (monotonic filing; the backstop behind the ingestion pre-check)
- https://github.com/theupdateframework/specification/blob/master/tuf-spec.md — rollback-attack check: a new version that is not exactly N+1 is discarded and reported
- `~/workspace/gauntlet-design-tasks-131-150.md` — task-150 design (Wave 26)
