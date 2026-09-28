# task-50: bounded event buffers

**Kind:** rust · **Status:** fail (seam absent — evict buffers exist but none counts drops; no producer/consumer event bus; banked product decision) · **Wave:** 46–50 · **Commits:** pending (wave 46-50)

## ELI5

Inside the agent, one part produces events ("tool finished",
"message arrived") and another part consumes them. Between them
sits a buffer — a waiting room with a fixed number of chairs.
When the room is full and a new event arrives, the oldest event
is shown the door (oldest-first eviction). The design demands
three things: the room's size is a named, explicit limit; the
doorman keeps an exact count of everyone turned away (a dropped
counter the consumer can read); and the eviction rule is a named,
documented choice. The adversarial scenarios are a sustained
flood (the counter must stay exact and memory flat) and critical
events (a priority rule must protect them — or its absence must
be documented).

phlow has waiting rooms but no doorman's count. The mailbox ring
(`phlow-tuios/src/mailbox.rs`) evicts oldest-first under an
explicit count bound (256 messages) and byte bound (512 KiB) —
memory is genuinely bounded. But the struct carries no dropped
counter and no method exposes one; 344 messages can vanish in a
flood and the only way to know is to notice the id numbers skip.
Worse, eviction is oldest-first *regardless of kind*: an early
`Ask` record (agent-coordination history — the closest thing to
a critical event) is evicted by a later flood of `Notice`s, with
no priority rule and no record of the loss. And there is no
producer/consumer event bus between orchestration stages at all
(task-35's finding, re-verified) — the design's buffer has no
bus to live on.

## What this task attempts

- **Goal:** locate the internal producer/consumer event bus
  buffer and run the design's scenarios: sustained overflow
  (memory bounded by an explicit constant, dropped counter
  exact, no silent loss) and critical event types (priority
  policy exists, or its absence is documented).
- **Mechanism:** the `task_50.rs` driver does two things. First,
  a runtime vocabulary scan over every non-gauntlet
  `crates/*/src/**/*.rs` for dropped-counter tokens
  (`drop_count`, `dropped_count`, `evicted_count`,
  `total_dropped`, `num_dropped`, `dropped_total`) and for an
  event bus (`event_bus`, `EventBus`, `event_channel`), plus a
  structural check that the real `Mailbox` struct/impl names no
  drop-counting field or method. Second, it drives the real
  `Mailbox` past its bound: 600 sends (ids 1–600), then a flood
  over an early `Ask` record.
- **Success criterion:** the design's triple — memory bounded by
  an explicit constant AND the dropped counter exact with no
  silent loss AND a named documented drop policy.
- **Non-goals:** building the bus or the counter. Whether the
  mailbox ring (and/or a real event bus) should gain exact drop
  accounting, and whether any event kind deserves eviction
  priority, is banked for Matt as a product decision, not
  auto-implemented.

## What happened

Fail at `"seam"` — on the first and only attempt, honestly.
The buffers bound memory but drop silently:

- `evict_buffers_drop_silently` (V): the dropped-counter
  vocabulary scan returns zero across the workspace, and the
  real `Mailbox` struct holds only
  `messages`/`text_bytes`/`next_id`/`senders` — no dropped
  counter, no method exposing one. The agent context window and
  the fusion decision log evict oldest-first the same way, with
  no accounting either.
- `no_producer_consumer_event_bus` (V): zero event-bus hits —
  no bus exists between orchestration stages, so the design's
  buffer has nowhere to live.
- `sustained_overflow_loss_is_silent` (A): 600 sends against the
  real mailbox; the ring holds 256, ids stay monotonic 1–600,
  and 344 messages vanish with no counter. The driver computes
  the loss from id gaps — proving the consumer must *infer*
  what the buffer will not report. Memory bounded ✓, honest
  loss accounting ✗.
- `critical_kinds_are_dropped_too` (A): an early `Ask` record
  is evicted by a later `Notice` flood. Documented, as the
  design requires: no priority policy exists; eviction is
  oldest-first regardless of `MessageKind`.

## The fix — what changed and why

Nothing changed: the seam is absent, so there is nothing to fix
without a product decision. The driver is new in this wave
(`src/tasks/task_50.rs`), plus four integration tests
(`tests/task_50.rs`). No production code was touched.

## Full technical depth

The honest credit first: the mailbox ring genuinely bounds
memory — two caps, count (`RING_MESSAGES_MAX` = 256) and bytes
(`RING_TEXT_BYTES_MAX` = 512 KiB), enforced in `evict_until_fits`
on every send. The byte cap means a single huge message cannot
wedge the ring; the count cap means the message list cannot
grow. What is missing is exactly the design's accounting
criterion: "the dropped counter is exact" and "the consumer
observes loss, not just infers it."

The A1 demonstration is the sharpest evidence: ids are
monotonic (1–600) while the ring holds 256, so exactly 344
messages are gone — but the `Mailbox` API surface has no way to
ask "how many did you drop?". A consumer polling with
`ReadFilter` sees a clean, gapless-looking window starting at
id 345; the gap is visible only if the consumer tracked ids
itself. That is the precise failure mode the design's "no
silent loss" criterion forbids.

The per-sender rate limiter in `send` is a separate mechanism
(the A1 sends use distinct senders so the flood tests the ring
bound, not the limiter) — worth naming so nobody confuses the
two.

What exact accounting would need (banked for Matt, not
implemented here): a monotonic `dropped` counter on the ring
(and any future event bus), incremented on every eviction,
exposed to consumers; and a documented decision on whether any
`MessageKind` (e.g. `Ask`) deserves eviction priority. The
current oldest-first-regardless-of-kind rule is at least
explicit and now documented — the silence was the problem, not
the policy.

## Sources

- Primary: `crates/phlow-tuios/src/mailbox.rs` (`Mailbox`,
  `RING_MESSAGES_MAX`, `RING_TEXT_BYTES_MAX`,
  `evict_until_fits`, `MessageKind`, `ReadFilter` — struct
  fields audited: no dropped counter).
- Primary: the live working tree — `crates/*/src/**/*.rs`
  excluding `phlow-gauntlet` (vocabulary scans: zero
  dropped-counter hits, zero event-bus hits).
- Driver: `crates/phlow-gauntlet/src/tasks/task_50.rs`
  (drives the real mailbox; behavioral overflow + priority
  demonstrations).
- Tests: `crates/phlow-gauntlet/tests/task_50.rs` (2V/2A).
