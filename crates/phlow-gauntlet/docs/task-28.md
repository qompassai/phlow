# task-28: idempotency keys

**Kind:** rust · **Status:** pass · **Wave:** 26–30 · **Commits:** pending (wave 26-30)

## ELI5

An idempotency key is a "do this exactly once" label. You hand the system
a request with a key like `payment-123`; if your network hiccups and you
send it again, the system recognizes the key and does NOT run the action
twice. The tricky parts: two different keys must both work (dedup is
per-key, not global); the same key with *different* content is a
conflict and must be rejected loudly, never silently executed; and keys
usually expire after a while (TTL) so old keys can be reused — or the
system documents that they never expire and accepts the growing key list.

## What this task attempts

- **Goal:** drive phlow's real submission-dedup point — the experiment
  `Scheduler`'s admission ledger (`crates/phlow-experiment/src/...`,
  `Scheduler::admit`) — with caller-supplied `NodeId`s as idempotency
  keys and show the design's idempotency holds: same key twice admits
  once; different keys admit twice; conflicting payloads are rejected;
  old keys behave per the documented expiry rule.
- **Mechanism:** the real `Scheduler` (no mocks), real `SchedulerNode`
  fixtures, `ExperimentError::DuplicateNode` as the conflict signal.
- **Success criterion:** exactly-once side effects per key; the conflict
  case returns an explicit error naming the key.
- **Non-goals:** adding a TTL or a result-handle return. The design's
  ideal is measured against what exists; deltas are documented, not
  hidden.

## What happened

Pass — on the first and only attempt. The seam exists: `Scheduler::admit`
keys nodes by caller-supplied `NodeId` and refuses re-admission with
`ExperimentError::DuplicateNode`, so the side effect (admission) runs
exactly once per key. The four cases:

- `same_key_twice_admits_once` (V): key `idem-1` submitted twice — first
  admitted, second rejected with `DuplicateNode { id: "idem-1" }`; the
  ledger holds the key exactly once.
- `different_keys_admit_twice` (V): keys `idem-a` and `idem-b` both
  admitted — dedup is per-key, not global.
- `same_key_different_payload_rejected` (A): key `idem-x` re-submitted
  with a different payload — rejected with `DuplicateNode { id:
  "idem-x" }`, an explicit conflict error naming the key; the stored
  payload (`payload-1`) is untouched — the conflicting submission was
  never silently executed or applied.
- `no_key_expiry_documented` (A): key `idem-old` re-submitted — rejected
  with `DuplicateNode`. There is no TTL: keys never expire and never
  become "new" again.

Two semantic deltas from the design's ideal, documented not hidden: (1)
the second submission gets `DuplicateNode` (an error), not the first
result handle — a client must catch-and-refetch; (2) no key TTL/expiry —
permanent rejection, with the cost that the admitted key set grows
without bound (no eviction).

## The fix — what changed and why

No product fix was needed — the seam exists and the safety properties
hold. The gauntlet-side work was pinning the real behavior exactly:

- **Changed:** `crates/phlow-gauntlet/src/tasks/task_28.rs` (new) —
  drives the real `Scheduler` through the real `NodeId`-keyed admission
  path; the doc comment names the two semantic deltas so no one reads
  "pass" as "matches the design's ideal exactly".
- **Why:** an idempotency claim is only as good as its conflict and
  expiry behavior. The cases prove the conflict is an explicit
  key-naming error (not silent execution) and that expiry does not exist
  (not "unobserved").
- **Source:** `crates/phlow-experiment/src/control_plane.rs`
  (`Scheduler::admit`, `NodeId`-keyed ledger),
  `crates/phlow-experiment/src/error.rs` (`DuplicateNode`).
- **Validation agents:** the 2 validation tests
  (`same_key_twice_admits_once`, `different_keys_admit_twice`) assert
  once-per-key admission and per-key (not global) dedup.
- **Adversarial agents:** the 2 adversarial tests
  (`conflicting_payload_is_rejected_not_executed`,
  `old_keys_never_expire_and_task_passes`) try to smuggle a conflicting
  payload through the same key and to resurrect an old key — both are
  refused — and the folded-in task-level assertion pins the `pass`.

## Full technical depth

`Scheduler::admit` validates the node's shape and inserts it into the
ledger keyed by `NodeId`; a second admission of the same id is refused
with `ExperimentError::DuplicateNode { id }` before any side effect
runs, so admission is exactly-once per key. Reads (`node()`) return the
stored node — the first submission's payload is what the ledger holds,
immutable to re-submission. There is no key-expiry field, no TTL sweep,
no eviction: the `node()` lookup that finds `idem-old` on re-submission
is the same lookup that rejects it, forever.

The two deltas matter for callers: (1) `DuplicateNode` is an error, not
a result handle — the client pattern is catch → `node()` re-fetch →
proceed; a design-ideal "return the original handle" would need the
admit path to return the stored node. (2) Permanent keys mean the
ledger's key set grows without bound across a long-lived scheduler —
acceptable for bounded experiments, a real cost for unbounded ones.

## Sources

- Primary: `crates/phlow-experiment/src/control_plane.rs`
  (`Scheduler::admit`, `NodeId`-keyed ledger, `node()` lookup).
- Primary: `crates/phlow-experiment/src/error.rs`
  (`ExperimentError::DuplicateNode`).
- Driver: `crates/phlow-gauntlet/src/tasks/task_28.rs` (real
  `Scheduler` + four cases).
- Tests: `crates/phlow-gauntlet/tests/task_28.rs` (2V/2A).
