# task-29: lease fencing

**Kind:** rust · **Status:** fail (open) · **Wave:** 26–30 · **Commits:** pending (wave 26-30)

## ELI5

A lease is a timed "I am the writer" badge: a worker acquires it, does
writes while it holds it, and renews it before it expires. "Fencing"
stops the scary failure: worker A holds the lease, pauses (long garbage
collection, network stall), its lease expires, worker B acquires it and
starts writing — then A wakes up and writes too, with stale data. A
fencing token (a number that only increases with each new lease) lets the
storage reject A's stale writes: "your token is old, the lease moved on."
"Correct" for this task means: at most one writer's effects are visible
per fencing epoch, and a stale holder's write is rejected with an
explicit fencing error.

## What this task attempts

- **Goal:** locate phlow's distributed lock / leadership lease ("if none,
  document; the worker then tests the *absence* as the finding" — the
  design's own instruction for this case) and show the fencing design
  holds between two contending workers.
- **Mechanism:** a token scan over the coordination-relevant crates
  (`phlow-experiment`, `phlow-runtime`, `phlow-agent`) for lease/fencing
  machinery, plus behavioral probes against the real experiment
  `Scheduler` to show the closest existing mechanism is not fencing.
- **Success criterion:** the design's fencing pass criteria (one writer
  per epoch; stale writes rejected with a fencing error).
- **Non-goals:** inventing a lease. The design says an evidenced
  `where = "seam"` failure is the correct result when the design says an
  absent seam is valid.

## What happened

Fail, open seam — on the first and only attempt. Phlow has no
distributed lock, lease, or fencing primitive. The token scans of
`phlow-experiment`, `phlow-runtime`, and `phlow-agent` find no
lease/fencing machinery (fail-closed: the driver errors with
`where = "recon"` if such tokens ever appear), and the behavioral
probes show the closest existing mechanism is not fencing:

- `no_lease_primitive_in_experiment` (V): token scan of
  `phlow-experiment/src` — no lease/fencing tokens.
- `no_lease_primitive_in_runtime_or_agent` (V): token scans of
  `phlow-runtime/src` and `phlow-agent/src` — no lease/fencing tokens.
- `generation_check_is_not_fencing` (A): the closest mechanism —
  generation-checked publication — is at-most-once publication, not
  mutual exclusion. A wrong-generation publish is rejected
  (`StaleGeneration`), a second publish for the same generation is
  refused (`DuplicateResult`), but no lease is acquired, no holder
  identity exists, and nothing expires: anyone presenting the right
  generation may publish.
- `two_workers_cannot_contend` (A): two independent "workers"
  (separate scheduler instances) both admit the same key — both
  succeed, because there is no shared lease store to contend on. The
  design's "two workers contend for a lease" scenario is
  unrepresentable; the loser is never fenced because there is nothing
  to hold.

The task-level `run` reports `fail` with `where = "seam"`: the fencing
pass criteria cannot be evaluated.

## The fix — what changed and why

No fix — this is a documented design gap, never fixed under gauntlet
authority. The gauntlet-side work was proving the absence two ways so it
cannot be mistaken for an untested claim:

- **Changed:** `crates/phlow-gauntlet/src/tasks/task_29.rs` (new) —
  the token scan is fail-closed (new machinery flips the verdict to
  `where = "recon"`, never to a silent pass), and the behavioral probes
  demonstrate the nearest mechanism answering a different question.
- **Why:** "no lease" as a text search alone is weak; pairing it with
  the generation-check and two-worker probes shows the absence is
  behavioral, not just textual — the design's contention scenario cannot
  even be set up.
- **Source:** `crates/phlow-experiment/src/control_plane.rs`
  (`publish_result`, generation checks), `src/error.rs`
  (`StaleGeneration`, `DuplicateResult`).
- **Validation agents:** the 2 validation tests
  (`no_lease_primitive_in_experiment`,
  `no_lease_primitive_in_runtime_or_agent`) assert the scans cover all
  three coordination crates with zero hits.
- **Adversarial agents:** the 2 adversarial tests
  (`generation_check_is_not_fencing`,
  `two_workers_cannot_contend_and_task_reports_seam`) show the nearest
  mechanism is at-most-once publication and that contention is
  unrepresentable — and the folded-in task-level assertion pins
  `where = "seam"`.

## Full technical depth

`Scheduler::publish_result` takes `(node_id, generation, digest, state)`:
a wrong generation is refused with `StaleGeneration { node, expected,
got }`; a second publish for the same node+generation is refused with
`DuplicateResult`. This looks fencing-adjacent — a stale writer's write
is rejected — but every fencing property is missing: there is no
acquire/release/renew API, no lease record, no holder identity (anyone
presenting the generation may publish), no expiry or clock, and no
shared store two workers could contend on. It is at-most-once
publication scoped to one in-process scheduler, not mutual exclusion.

What is missing for lease fencing: a lease-acquisition primitive with
expiry, a monotonically increasing fencing token issued per lease
grant, a shared lease store (or consensus) so two workers observe the
same lease state, and write paths that present the token and are
rejected when it is stale. The design gap: either phlow grows a lease
primitive for its multi-worker coordination or the single-scheduler
scope is documented as the intended boundary (with the two-scheduler
contention case explicitly out of scope).

## Sources

- Primary: `crates/phlow-experiment/src/control_plane.rs`
  (`publish_result`, generation checks).
- Primary: `crates/phlow-experiment/src/error.rs` (`StaleGeneration`,
  `DuplicateResult`).
- Driver: `crates/phlow-gauntlet/src/tasks/task_29.rs` (token scans +
  behavioral probes + `seam_finding`).
- Tests: `crates/phlow-gauntlet/tests/task_29.rs` (2V/2A).
