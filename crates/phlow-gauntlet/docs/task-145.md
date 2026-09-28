# task-145: exact submission payload preview

**Kind:** rust · **Status:** pass · **Wave:** 25 · **Commits:** pending (wave 141-145)

## ELI5

Before anything is sent to the bug-bounty platform, the operator gets
to see *exactly* what would go over the wire — every byte of it: the
envelope (which program, which submission, a fingerprint of the
content) plus the report itself. The preview is proven identical to
what the serializer produces, byte for byte. If anyone changes the
finding after the preview, the fingerprint won't match anymore — the
preview is marked stale and the send is blocked until the operator
looks at a fresh preview. And preview isn't a back door: you can only
ask for one when the finding is already `Approved`.

## What this task attempts

- **Goal:** prove the operator previews byte-exactly what would be
  sent; any post-preview mutation invalidates the preview and blocks
  submission until re-preview; preview is refused for non-`Approved`
  findings.
- **Mechanism:** `crates/phlow-gauntlet/src/tasks/task_145.rs`
  implements `preview_payload` (finding + report markdown → `Preview {
  bytes, sha256, content_hash }`, envelope serialized with
  `serde_json`), `check_preview_fresh` (re-derive the content hash,
  compare), driving the scaffold's `SubmissionGate` (hash-bound,
  single-use-nonce approval) and `FakePlatform` (scripted double).
- **Success criterion:** preview bytes == independent re-serialization
  byte-for-byte; envelope carries program id, submission id, content
  hash; mutated finding → `PreviewError::Stale` and the gate answers
  `HashMismatch` on the mutated bytes; re-preview re-authorizes;
  `Reportable`/`Candidate` → `PreviewError::NotApproved`; the bytes
  the platform receives equal the preview bytes exactly.
- **Non-goals:** the human-readable document (task 144), the operator
  approval UX itself (task 146). The envelope format is the driver's
  documented-ours format, not any real platform's API.

## What happened

All four cases passed on the first attempt against scripted fixtures
(MOCK) with `FakePlatform` as the platform double:

- **preview_equals_serializer_output (V1):** preview bytes equal an
  independently re-serialized envelope byte-for-byte; `preview.sha256`
  matches the bytes; the submission gate authorized the preview bytes
  under an operator approval bound to that hash; `FakePlatform`
  received the identical bytes (`last_payload() == preview.bytes`).
- **preview_carries_envelope (V2):** the preview bytes parse as JSON
  carrying `program_id`, `submission_id`, `content_hash` (== sha256 of
  the report bytes), and the full `report` body.
- **mutation_invalidates_preview (A1):** after retitling the finding,
  `check_preview_fresh` returned `Err(PreviewError::Stale)`; submitting
  the mutated envelope under the old approval hash got
  `GateError::HashMismatch`; the un-mutated report still verified
  fresh (no false stale); re-preview produced a fresh preview and the
  gate authorized the new bytes with a new nonce.
- **unapproved_preview_refused (A2):** `Reportable` and `Candidate`
  findings both got `Err(PreviewError::NotApproved { state })` naming
  the actual state.

Verdict: **replicates** — what you see is what you send, and any
deviation blocks the send.

## The fix — what changed and why

No fix iterations: the first attempt passed.

## Full technical depth

The preview is a `Preview { bytes, sha256, content_hash }` triple.
`preview_payload` refuses unless `finding.state == Approved` — the A2
gate, keeping preview inside the approval flow rather than beside it.
It then hashes the report markdown (`content_hash`), builds the
envelope as a `serde_json::json!` literal with fixed key order
(`program_id`, `submission_id`, `content_hash`, `report`), and
serializes once. `serde_json` preserves insertion order
(`preserve_order` is a default feature), so the serialization is
deterministic — the V1 case's independent re-serialization of the same
literal must and does match byte-for-byte, which is the whole point:
there is exactly one serialization path, and the preview *is* its
output, not a second implementation that could drift.

Staleness is content-addressed, not version-counter-addressed: the
preview carries no revision number because it doesn't need one.
`check_preview_fresh` re-derives `sha256(report_md)` and compares it
to `preview.content_hash`. Any mutation of the finding that changes
the report bytes changes the hash — the preview is stale, typed
`PreviewError::Stale { expected, got }`. The A1 case then shows the two
independent enforcement points agreeing: the driver's freshness check
*and* the scaffold's `SubmissionGate`, which binds the operator's
approval to the exact payload hash and answers `HashMismatch` when the
mutated bytes arrive under the old hash. The gate additionally spends
the approval nonce exactly once, so the re-previewed send needs a new
approval with a new nonce — replaying the old approval is refused by
the gate's spent-nonce set (the two submits in A1 use nonces 4243 and
4244 for exactly this reason).

The wire-integrity assertion closes the loop the task title promises:
`FakePlatform::submit(&preview.bytes)` records the payload, and the
case asserts `last_payload() == preview.bytes` as slices — byte
equality, not "equivalent JSON". There is no serialization, copy, or
transformation between the preview the operator reviewed and the bytes
the platform received in this path; the assertion would catch any
hidden mutation step smuggled in later.

## Sources

- The "what you see is what you send" (WYSIWYS) principle for
  security-critical confirmations — the operator authorizes the exact
  bytes, and the authorization is cryptographically bound to them.
  (Primary: the task's design rationale.)
- The scaffold itself: `crates/phlow-gauntlet/src/bounty/approve.rs`
  (`SubmissionGate`: liveness, scope binding, exact payload-hash
  binding, single-use nonces; `sha256_hex` — the same hash the
  custody chain in task 141 uses, so hashes are comparable across the
  wave), `crates/phlow-gauntlet/src/bounty/platform.rs`
  (`FakePlatform` — documented as modeling platform *mechanics*, not
  any real platform's API).
- `serde_json` with default features: `Map` preserves insertion order
  (`preserve_order`), making `to_vec` of a literal-constructed `Value`
  deterministic. (Primary: the byte-equality claim's serialization
  premise.)
- Design doc: `~/workspace/gauntlet-design-tasks-131-150.md`, Wave 25,
  task-145 (mirrors the task-92 approval machinery's hash-bound
  previews).
