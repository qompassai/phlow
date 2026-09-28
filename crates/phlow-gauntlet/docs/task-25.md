# Learning doc template — copy to `task-NN.md` and fill in

> Every section is required. ELI5 first, then full depth. Cite primary
> sources for every protocol/API claim. Document failures with evidence,
> not adjectives.

# task-25: backpressure propagation

**Kind:** rust · **Status:** pass · **Wave:** 21–25 · **Commits:** pending (wave 21-25)

## ELI5

A producer makes work and a consumer does it, with a channel between them.
If the producer is faster than the consumer, the channel fills up — and
then something has to give. Backpressure is the healthy answer: instead
of piling up an infinite backlog (which eats all memory) or throwing work
away (which loses data), the producer *slows down to the consumer's
pace* — it waits. "Correct" for this task means: when the consumer is
slow, the producer blocks instead of buffering; when the consumer stops
entirely, the producer fails loudly with a timeout instead of hanging
forever; and when the consumer recovers, nothing was lost.

## What this task attempts

- **Goal:** drive phlow's real producer/consumer channel and prove
  backpressure: slow consumer ⇒ producer parks; stalled consumer ⇒
  bounded memory, zero loss; dead consumer ⇒ explicit timeout + recovery.
- **Mechanism:** the real `phlow_runtime::transport::MsgpackTransport`
  (`crates/phlow-runtime/src/transport/msgpack.rs`): one current-thread
  tokio worker owns the Neovim socket; `exec` calls cross
  `mpsc::channel(WORKER_QUEUE_CAPACITY)` with `WORKER_QUEUE_CAPACITY = 1`;
  `exec` takes `&mut self`, so at most one request is ever in flight per
  transport. The driver (`crates/phlow-gauntlet/src/tasks/task_25.rs`)
  scripts a fake msgpack-RPC peer with rmpv — the transport's own codec —
  over a Unix socket. Real transport code, real channel, controllable
  slow consumer.
- **Success criterion:** 4/4 cases pass — matched rates serve fast; a 2s
  consumer delay parks the producer ≥2s; a 30s full stall with 4 parked
  producers keeps RSS growth under 128 MiB with zero lost messages; a
  black hole fails with `TransportError::Timeout` near the 2s deadline and
  the same transport then recovers via reconnect.
- **Non-goals:** a general orchestration event bus — none exists. The
  recon documents the scope honestly: the experiment Scheduler's queue is
  admission-only (no consumer), the tuios permit pool throttles TCP
  connections rather than carrying stage messages. This channel is the
  editor-transport seam, and it is the only producer/consumer channel in
  the codebase.

## What happened

Passed on the first test run. Evidence from the four cases:

- `matched_rates` (V): 5 sequential execs against an immediate peer —
  all `Ok` with the exact canned `{"ok": true}` reply; worst latency
  well under the 5s local bound.
- `slow_consumer_parks` (V): peer delayed each reply 2s; the producer
  parked ~2s (elapsed in [2, 25)s) then received the intact reply —
  "producer throughput tracks the consumer: it blocked instead of failing
  or buffering".
- `stall_30s_resumes_cleanly` (A): 4 producers parked behind a 30s full
  stall; all 4 received intact replies when the peer resumed; total wall
  ~30s; RSS growth ≈ 0 MiB (bound 128 MiB); "no messages lost".
- `timeout_is_explicit` (A): black-hole peer — `exec` failed with
  `TransportError::Timeout` at ~2s (in [2, 10)s, "no hang"); the same
  transport then served a healthy peer on the same socket path in
  well under 10s ("the worker reconnected fresh after dropping the
  stalled stream").

The task-level `run` passes end-to-end (all four cases in one driver run).

## The fix — what changed and why

No fix iteration was needed — the design passed against the real
channel on the first attempt. One scope decision is recorded here because
it is easy to state wrong:

- **Changed:** the driver doc comment and recon state the scope plainly:
  this is the editor-transport channel, not an orchestration event bus.
- **Why:** the design's "producer/consumer channel" language could be
  read as demanding a stage-to-stage event bus, which phlow does not
  have. Claiming the msgpack channel *is* that bus would be dishonest;
  claiming the task is untestable would be lazy. The channel is a genuine
  producer/consumer seam with real backpressure semantics (bounded
  capacity + parking producer + explicit timeout), and the driver says
  exactly which seam it is.
- **Source:** `crates/phlow-runtime/src/transport/msgpack.rs`
  (`WORKER_QUEUE_CAPACITY: usize = 1`, `mpsc::channel`, `tx.send(request).await`,
  the `exec` doc comment "the channel cannot back up").
- **Validation agents:** the 2 validation tests
  (`matched_rates_serve_exact_replies`, `slow_consumer_parks_the_producer`)
  assert exact replies, latency bounds, and the parking behavior.
- **Adversarial agents:** the 2 adversarial tests
  (`full_stall_30s_resumes_with_flat_memory`,
  `black_hole_times_out_explicitly_then_recovers`) run the 30s stall with
  4 parked producers and the black-hole-then-recover sequence.

## Full technical depth

`MsgpackTransport::new` builds a single-threaded tokio runtime and
spawns one worker task owning the Unix socket; the producer side keeps
the `mpsc::Sender` and a oneshot-free reply path (the worker holds the
response waiter). `exec(expression, args, timeout)`:

1. Encodes the msgpack-RPC request (`[0, id, "nvim_exec_lua", [expr, args]]`).
2. `tx.send(request).await` — parks here if the worker is busy (capacity 1).
3. Awaits the worker's reply with `tokio::time::timeout(timeout + WORKER_GRACE)`.
4. The worker writes the frame, reads the response with its own per-read
   timeouts, matches the msgpack-RPC id, and on mismatch drops the
   stream as desynchronized.

Backpressure is therefore two-layered: the capacity-1 channel bounds
queueing to a single request, and the reply-await parks the producer for
the consumer's full service time. Because `exec` takes `&mut self`, one
transport can never have two requests in flight — the channel physically
cannot back up beyond its bound. The outer timeout converts a dead
consumer into `TransportError::Timeout` instead of a hang, and the worker
drops the half-dead stream, so the next `exec` reconnects fresh.

The scripted peer speaks the same wire protocol: it decodes request
frames with rmpv (keeping a 1 MiB buffer cap that fails closed), extracts
the msgpack-RPC id, and per behavior replies immediately (`[1, id, nil,
{ok: true, seq}]`), replies after N seconds, or never replies. Each case
runs against its own socket path in its own directory, so parallel tests
never share a peer.

## Sources

- Primary: `crates/phlow-runtime/src/transport/msgpack.rs`
  (`MsgpackTransport`, `WORKER_QUEUE_CAPACITY`, worker loop,
  `exec`, `TransportError::Timeout`, `WORKER_GRACE`).
- Primary: `crates/phlow-editor/src/bridge.rs` (`EditorTransport::exec`
  — `&mut self`).
- Primary: `crates/phlow-editor/src/contract.rs` (`SCHEMAS_LUA`,
  `CALL_LUA` — the two contract expressions).
- Driver: `crates/phlow-gauntlet/src/tasks/task_25.rs` (scripted peer,
  four cases, recon).
- Tests: `crates/phlow-gauntlet/tests/task_25.rs` (2V/2A).
