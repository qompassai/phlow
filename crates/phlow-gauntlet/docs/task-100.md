# task-100: artifact swap between pipeline stages

**Kind:** rust · **Status:** fail (open) · **Wave:** 96–100 · **Commits:** pending (wave 96-100)

## ELI5

Imagine a relay race where the baton is never handed over — at each checkpoint the runner just *says* which baton they carried, and the judge writes it down without looking. The proposal checkpoint, the evidence checkpoint, and the approval checkpoint each verify their own thing and write their own note, but nobody checks that the three notes are about the same baton. Build good test evidence for the harmless plan, get the harmless plan approved, then promote the harmful plan while reusing the good evidence — every stage's own check passes, because no stage compares notes with any other stage.

## What this task attempts

- **Goal:** verify that the propose→validate→approve→apply pipeline binds the artifact across stages — hash-chained records, stage-to-stage re-hashing — or document the absence with mechanism evidence.
- **Mechanism:** `src/tasks/task_100.rs` drives the REAL `phlow-experiment` types with deterministic fixtures: the same benign artifact through all stages promotes (V1); the emitted record's fields are reconned for a proposal-content hash (V2); the validated-twin swap — evidence built for the benign twin, malicious twin promoted — succeeds (A1); the approve→apply transition is reconned for an artifact verifier (A2).
- **Success criterion:** cross-stage hash chaining exists, or its absence is documented with mechanism evidence.
- **Non-goals:** adding hash chaining on gauntlet authority (product decision — banked, never implemented here).

## What happened

Honest FAIL at `where = "swap_between_validate_and_approve"`, first attempt — the stage-binding seam is ABSENT:

- **V1:** the same (benign) artifact through propose → validate → approve succeeds — the unchained pipeline works when nothing is swapped.
- **V2:** the emitted `PromotionRecord` carries the approval's candidate digest (copied from the approval, never compared to the proposal) and no hash computed over the proposal's content — the structural reason swaps are undetectable in-band.
- **A1:** the gap — the validated-twin swap succeeds. Evidence (with artifact digests) is built for the benign twin; the promotion call takes the evidence and the approval as *independent arguments* alongside the malicious proposal; the gate checks each in isolation (evidence complete, approval valid, surface unprotected) and promotes. The record then shows the benign evidence digests, the copied approval digest, and the malicious proposal's identifiers — all from the same promotion, never cross-checked.
- **A2:** the gap — there is no apply stage in code at all: no `fn apply` exists anywhere in `crates/phlow-experiment/src` (apply is human-driven merge by design, per task-91), so nothing re-hashes the artifact after approval, and the record carries no content hash an apply step could verify against.

## Full technical depth

`PromotionGate::promote` takes `proposal`, `approval`, and `evidence` as three independent arguments — the signature itself is the absence of a stage contract. The A1 case builds the evidence bundle for the benign twin (`ArtifactDigest` entries over distinct hex fixtures), signs a genuine approval for it, then promotes the malicious twin with that evidence and approval. The promotion succeeds; the case asserts each stage checked its own input (evidence complete, approval verified, surface clean) while no comparison bound them, and fails the case because the swap went undetected.

The V2 case is the recon: the record's fields are inspected via its accessors and debug output — `candidate_digest` is present (copied from the approval at promote time) and there is no proposal-content hash. The A2 case scans `crates/phlow-experiment/src` for `fn apply` (zero hits) and confirms the record lacks the field an apply verifier would need.

The 50/50 split means the driver stops at A1: A2 is exercised by its own case and integration test, where the assertions live.

Product decision banked for Matt: whether the pipeline should gain cross-stage artifact hash chaining (with both expected and actual hashes on mismatch) across propose→validate→approve→apply.

## Sources

- `crates/phlow-experiment/src/promotion.rs` — `PromotionGate::promote` (three independent arguments; no cross-stage binding), `PromotionRecord` (no content hash)
- `crates/phlow-experiment/src/evidence.rs` — `EvidenceBundle`, `ArtifactDigest` (completeness-only checks)
- `~/workspace/gauntlet-design-tasks-71-100.md` — task-100 design (Wave 96–100)
