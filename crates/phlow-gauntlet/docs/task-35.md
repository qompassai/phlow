# task-35: duplicate delivery dedup

**Kind:** rust · **Status:** fail (seam absent — no event bus, no delivery dispatcher) · **Wave:** 31–35 · **Commits:** pending (wave 31-35)

## ELI5

"At least once" delivery means the postman might deliver the same
letter twice — the receiver's job is to not act on it twice. The
standard defense: every event gets a delivery ID, and the consumer
keeps a durable set of IDs it has already processed. Deliver the same
event 10 times, the side effect happens exactly once (idempotency).
Restart the consumer in the middle of the redeliveries, and it must
still deliver exactly-once: the "already seen" set survives the
restart, because if that set lived only in memory the restart would
wipe it and the side effect would happen again.

## What this task attempts

- **Goal:** locate phlow's event consumer (delivering `run.finished`
  with a side effect) and drive the design's scenarios against the real
  dispatcher: event delivered 10 times with the same ID → side effect
  counted exactly once; adversarial: consumer restarts mid-redelivery
  → still exactly once, dedup state survives.
- **Mechanism:** the real runtime crates (transports, scheduler) — no
  mocks for the dispatcher; a real subscription to `run.finished` and a
  real consumer counting side effects.
- **Success criterion:** exactly-once observable effect across all
  scenarios; durable seen-set survives restart.
- **Non-goals:** inventing an event bus. There is no bus to subscribe
  to; a documented failure with evidence is a success.

## What happened

Fail at `"seam"` — on the first and only attempt, honestly. There is no
internal event bus or delivery dispatcher in phlow:

- `runtime_transports_are_request_response_only` (V): the real
  transports construct — `MsgpackTransport::default()` (Neovim
  msgpack-RPC) and `ReqwestTransport::default()` (Ollama HTTP). Both
  are request/response RPC: zero event buses, zero subscribe APIs.
  Events move one direction per call; nothing broadcasts.
- `keyed_dedup_is_submission_side_only` (V): the keyed dedup that
  exists is caller-keyed at *submission* — `Scheduler::admit` rejects
  a re-admission of the same `NodeId` with `DuplicateNode`. Zero
  delivery-keyed dedups. The key is supplied by the caller, never
  assigned by a bus at delivery.
- `no_event_consumer_to_deliver_to` (A): there is no `run.finished`
  event type, no subscription API, and no counted side effect —
  `Vec::<()>::new()` stands in for the consumer: the design's default
  scenario (deliver 10×, count the side effect) cannot be staged.
- `restart_dedup_state_is_vacuous` (A): zero delivery-dedup states
  exist, so the restart scenario is vacuous — nothing persists, and
  there is nothing whose durability could be asserted.

No event system was reconned elsewhere either (source scan of
`crates/phlow-runtime/src`, `crates/phlow-llm/src`,
`crates/phlow-experiment/src`, plus the nvim-lua bridge layer): no
`subscribe`, no `broadcast`, no delivery ID type, no seen-set. The
existing dedup primitives are all distinct seams — submission-side
`DuplicateNode`, approval replay IDs — and none of them is a delivery
dispatcher.

## The fix — what changed and why

No product fix was made — and none should be made on gauntlet
authority. The gauntlet-side work was pinning the real behavior exactly:

- **Changed:** `crates/phlow-gauntlet/src/tasks/task_35.rs` (new) —
  constructs the real transports, exercises the real submission-side
  dedup, and documents the absent bus; the doc comment names the
  distinct-but-unrelated dedup primitives so no one confuses
  `DuplicateNode` with delivery dedup.
- **Why:** a dedup claim needs a delivery event to duplicate. The
  cases prove delivery is request/response only and the only keyed
  dedup lives at submission with caller-supplied keys — so the honest
  verdict is seam-absent, not a faked pass on `DuplicateNode`.
- **Source:** `crates/phlow-runtime/src/transport.rs`
  (`MsgpackTransport`, `ReqwestTransport` — request/response),
  `crates/phlow-experiment/src/control_plane.rs`
  (`Scheduler::admit`, `DuplicateNode` — submission-side).
- **Validation agents:** the 2 validation tests
  (`runtime_transports_are_request_response_only`,
  `keyed_dedup_is_submission_side_only`) pin the real transports and
  the submission-side dedup, scoping the failure precisely.
- **Adversarial agents:** the 2 adversarial tests
  (`no_event_consumer_to_deliver_to`,
  `restart_dedup_state_vacuous_and_task_fails_at_seam`) try to stage
  the duplicate-delivery scenario and the restart — neither can be
  staged — and the folded-in task-level assertion pins the
  `fail`-at-`seam` verdict.

## Full technical depth

phlow's delivery surface is two transports and a scheduler. The
transports (`MsgpackTransport` for Neovim msgpack-RPC,
`ReqwestTransport` for Ollama HTTP) issue calls and await replies —
each delivery is its own request; there is no fan-out, no replay, no
redelivery. The scheduler's idempotency surface is `NodeId`-keyed
admission: a second `admit` with the same id returns
`DuplicateNode { node }`, and approval replay IDs guard approval
reuse. These are submission gates, not delivery dedup: they dedup
*intent* the caller repeats, not *deliveries* the infrastructure
repeats.

Consequence: if a downstream ever wraps phlow in an at-least-once bus
(e.g. an agent loop re-delivering `run.finished`), there is no
built-in defense — the consumer must keep its own seen-set. If Matt
ever wants one here, the shape is clear — bus-assigned delivery IDs, a
subscribe/dispatch API, and a durable seen-set consulted before any
side effect — but that is a product decision, banked, not a gauntlet
fix.

## Sources

- Primary: `crates/phlow-runtime/src/transport.rs`
  (`MsgpackTransport`, `ReqwestTransport` — request/response RPC).
- Primary: `crates/phlow-experiment/src/control_plane.rs`
  (`Scheduler::admit`, `DuplicateNode` — submission-side, caller-keyed).
- Driver: `crates/phlow-gauntlet/src/tasks/task_35.rs` (real
  transports + scheduler, four cases).
- Tests: `crates/phlow-gauntlet/tests/task_35.rs` (2V/2A).
