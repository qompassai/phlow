# task-42: audit log append-only

**Kind:** rust · **Status:** fail (seam absent — no tamper-evident journal, no entry linking, no sealing key, no verifier; banked product decision) · **Wave:** 41–45 · **Commits:** pending (wave 41-45)

## ELI5

An audit log is a diary nobody can secretly rewrite: every entry is
chained to the one before it (like a daisy chain — break one link
and the break shows), the whole chain is sealed with a key, and
there is a checker program anyone can run that points at exactly
which entry was tampered with. If an attacker flips one byte in an
old entry, the checker must fail and name the entry number. If they
chop off the tail, the length check must catch it. If they rewrite
the whole diary consistently, they still need the sealing key —
without it, the forgery fails.

phlow has no such diary. No crate writes events to a verifiable
journal; no entry chaining, no sealing keys, no checker program
exist anywhere in the sources. Two grow-only structures do exist,
but neither is the seam: the approval replay store keeps
single-use approval ids as one-JSON-object-per-line JSONL (replay
protection, not a journal — entries *expire*, the opposite of a
diary), and the fusion decision log is an in-memory list that is
never persisted. The design's adversarial weapons — byte flip, tail
truncation, whole-log rewrite — have no target.

## What this task attempts

- **Goal:** locate the audit event writer in phlow's Rust crates and
  run the design's tamper scenarios against it: byte flip in an old
  entry (verification fails with the entry index), tail truncation
  (length + head-hash mismatch), whole-log rewrite without the
  sealing key (fails).
- **Mechanism:** the `task_42.rs` driver probes the live working
  tree — a runtime vocabulary scan over every
  `crates/*/src/**/*.rs` for integrity-mechanism tokens (assembled
  at runtime so the probe cannot self-match; the probe's own file is
  path-excluded because the task NAME contains the design
  vocabulary), plus classification of the three grow-only structures
  the scan does find.
- **Success criterion:** any post-hoc modification is *detectable*
  with the tampered entry identified; verification is a runnable
  check, not documentation.
- **Non-goals:** building the journal. Whether phlow should gain a
  sealed, entry-linked journal is banked for Matt as a product
  decision, not auto-implemented.

## What happened

Fail at `"seam"` — on the first and only attempt, honestly. There
is no journal to tamper with:

- `no_integrity_tokens_in_sources` (V): the vocabulary scan finds
  zero integrity-mechanism tokens across the workspace — no entry
  linking, no sealing keys, no tamper-evidence journaling in any
  phlow source.
- `grow_only_stores_lack_verification` (V): the three grow-only
  structures the scan does find are classified and rejected.
  `ConsumedApprovals` (`phlow-experiment/src/promotion.rs`)
  persists single-use approval ids as one-JSON-object-per-line
  JSONL for restart safety — replay protection, not an event
  journal: entries EXPIRE via `evict_expired` (the opposite of a
  journal), and there is no entry linking, no sealing key, no
  verifier. `FusionReceipt.log`
  (`phlow-tools/src/solpi/action_fusion.rs`) is an in-memory
  `Vec<String>` decision log — never persisted, no integrity at
  all. `ApprovalQueue.records`
  (`phlow-approval/src/queue.rs`) is a bounded in-memory
  `Vec<Record>` — "append-only" only as a doc note that a full
  queue rejects new requests instead of evicting old decisions;
  never persisted, no entry linking, no sealing key, no verifier.
  Zero integrity-mechanism tokens appear near any of the three
  structures.
- `byte_flip_has_no_verifier` (A): flipping a byte in an old entry
  would need a writer and a verifier; neither exists. No tamper
  detection was measured because there is no journal to tamper —
  a claimed detection would be invented, not sourced.
- `truncation_has_no_head_hash` (A): no head-hash or length
  tracking exists anywhere, and the replay store drops expired
  entries by design — so a shorter tail is normal operation there,
  not evidence of tampering. Truncation is undetectable *and*
  unremarkable.

## The fix — what changed and why

No product fix was made — banked for Matt as a product decision
(whether phlow should gain a sealed, entry-linked journal; the
approval/promotion path, which already persists consumed approval
ids, would be the natural first consumer). The gauntlet-side work
was an honest probe:

- **Changed:** `crates/phlow-gauntlet/src/tasks/task_42.rs` (new) —
  bounded live-workspace probe: integrity-vocabulary scan,
  grow-only-structure classification, and the two adversarial
  no-target cases; fail-closed (a case fails if the vocabulary ever
  appears, i.e. `where = "<case>"` instead of the task-level
  `"seam"`).
- **Why:** a tamper-evidence claim needs a journal. The probe proves
  the vocabulary is absent and the adjacent grow-only structures
  lack every integrity mechanism — so the honest verdict is
  seam-absent, not a faked pass on "the JSONL is append-only".
- **Source:** the live working tree (`crates/*/src/**/*.rs`),
  `crates/phlow-experiment/src/promotion.rs`
  (`ConsumedApprovals`, `evict_expired`),
  `crates/phlow-tools/src/solpi/action_fusion.rs`
  (`FusionReceipt.log`),
  `crates/phlow-approval/src/queue.rs`
  (`ApprovalQueue.records`).
- **Validation agents:** the 2 validation tests pin the
  `fail`-at-`seam` verdict and prove the scan ran over the real
  tree (zero integrity tokens; the three grow-only structures found
  and classified).
- **Adversarial agents:** the 2 adversarial tests pin the
  no-verifier / no-head-hash evidence and rule out a probe crash
  masquerading as the finding.

## Full technical depth

The probe's vocabulary scan assembles its tokens at runtime from
halves (`audit`+`_log`, `hash`+`_chain`, `tamper`+`-evident`,
`sealing`+` key`, and the `append`+`-only` pair handled
separately), so the probe's own source can never match its prose.
The walk covers every `.rs` file under a `src/` directory in
`crates/`, bounded by file size (1 MiB) and file count (50,000),
skipping only the probe's own file by exact path — necessary
because the task NAME itself ("audit log append-only") contains
the design vocabulary.

The mechanism scan returns zero hits: no phlow source names entry
linking between consecutive records, a sealing key, or
tamper-evidence journaling. The grow-only scan returns exactly the
known structures: `ConsumedApprovals` in promotion.rs (the JSONL
replay store — one `{\"id\",\"expires_ms\"}` object per line,
`evict_expired` dropping dead entries past the TTL),
`FusionReceipt.log` in action_fusion.rs (a bounded in-memory
`Vec<String>`), and `ApprovalQueue.records` in queue.rs (a bounded
in-memory `Vec<Record>` whose "append-only" doc note only describes
full-queue rejection instead of eviction), plus this task's own NAME
echoed in the task-36 doc. Each is classified against the design's
criteria and rejected:
the replay store's entries expire (anti-journal), neither links
entries, neither seals, neither verifies.

The adversarial cases then follow the design's weapons to their
absent targets. A byte flip needs a log file format with per-entry
integrity — none exists, so there is no entry index for a verifier
to name. A tail truncation needs length or head-hash tracking —
none exists, and at the closest store a shorter tail is routine
(expiry), so the scenario is not even anomalous there. A
whole-log rewrite needs a sealing key the attacker lacks — there
is no key because there is nothing to seal. In each case the probe
documents the missing target rather than measuring a detection
that has no mechanism behind it.

What tamper-evidence would need (banked for Matt, not implemented
here): an event writer that links each entry to the previous
(hash chain), seals checkpoints with a key, and ships a runnable
verifier that names tampered entry indexes — with the
approval/promotion path as the natural first consumer, since it
already persists security-relevant records.

## Sources

- Primary: the live working tree — `crates/*/src/**/*.rs`
  (vocabulary scan, zero integrity-mechanism hits).
- Primary: `crates/phlow-experiment/src/promotion.rs`
  (`ConsumedApprovals`: JSONL replay store, `evict_expired`).
- Primary: `crates/phlow-tools/src/solpi/action_fusion.rs`
  (`FusionReceipt.log`: in-memory decision log).
- Driver: `crates/phlow-gauntlet/src/tasks/task_42.rs` (bounded
  live-workspace probe).
- Tests: `crates/phlow-gauntlet/tests/task_42.rs` (2V/2A).
