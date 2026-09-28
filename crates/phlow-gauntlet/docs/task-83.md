# task-83: streaming vs non-streaming parity

**Kind:** rust (validation) · **Status:** fail (open) · **Wave:** 81–85 · **Commits:** pending (wave 81-85)

## ELI5

Providers can send a response two ways: all at once (one JSON blob) or as a stream of chunks (server-sent events, each `data: {...}` line a fragment, ending with `data: [DONE]`). The chunks have to be reassembled byte-exactly: a chunk boundary can land in the middle of a JSON escape (`"don\u005Cu..."`) or in the middle of a multi-byte UTF-8 character, and sloppy reassembly produces garbage or mojibake. A stream that ends without `[DONE]` is *truncated* — it must be reported as truncated, never treated as a complete answer. Phlow never streams: every chat payload carries `"stream": false`, and there is no SSE code at all. There is no second mode for the single-shot mode to be in parity with.

## What this task attempts

- **Goal:** verify streaming vs non-streaming parity — byte-identical reassembly across adversarial chunk splits (inside escapes, inside multi-byte UTF-8), truncated streams typed as truncated, tool-call argument fragments reassembled exactly, usage handled whether it arrives in the final chunk or not at all — or document the absence with file evidence.
- **Mechanism:** `src/tasks/task_83.rs` measures the REAL payload builder (`build_chat_payload` emits `stream=false`), runs exact-token source scans (task_48 pattern) for `event-stream` / `text/event-stream` / `[DONE]` / `reassemble` / `eventsource` / `event_source` over `phlow-llm/src` (all zero hits), and inspects the transport contract (`post_chat -> Result<Value, LlmError>`: one decoded JSON body per call — no chunk boundary concept) plus `BoundedBody` (a byte cap, not a terminator check) and `LlmError` (no `TruncatedStream` variant).
- **Success criterion:** the reassembled payload byte-identical to the non-streaming payload on fixtures with truncation typed, or the absence documented with file evidence (and banked for Matt as a product decision).
- **Non-goals:** inventing SSE streaming on gauntlet authority (it is a product decision, not a bug fix).

## What happened

Honest FAIL at `where = "seam"`, first attempt — the absence IS the finding:

- **V1:** streaming disabled by construction — the real builder emits `stream=false` (payload.rs:69 inserts it unconditionally).
- **V2:** no SSE vocabulary — 0 hits for every streaming marker.
- **A1:** chunk-boundary semantics unrepresentable — the contract has no chunks, so a boundary splitting a JSON escape or a multi-byte UTF-8 sequence cannot occur; there is no reassembly function for "byte-exact" to live in.
- **A2:** truncation semantics unrepresentable — `BoundedBody` caps bytes, it does not check terminators; with stream:false the provider's JSON body IS the completion, so "truncated streams never count as complete" has no error to arrive as.

## Full technical depth

The probe scans every `phlow-llm/src/**/*.rs` with exact-token (case-insensitive) matching, bounded like task_48, excluding only the gauntlet crate. Any hit fails the case loudly ("the absence finding is refuted"), keeping the finding falsifiable.

The design's adversarial chunk splits (inside escapes, inside multi-byte UTF-8, missing terminator) are transport-level concerns that presuppose a streaming transport. Phlow's transport contract returns the decoded body in one call: the only size discipline is `RESPONSE_BYTES_MAX` (2 MiB) enforced by `BoundedBody` *before* the chunk is kept — a hostile server cannot exhaust memory, but the "complete vs cut-off" distinction the design wants is unrepresentable because the client never asks for a stream.

Banked for Matt (product decision, NOT auto-implemented on gauntlet authority): whether phlow-llm should support SSE streaming with byte-exact reassembly and truncated-stream typing.

## Sources

- `~/workspace/repos/phlow/crates/phlow-llm/src/payload.rs:69` — `"stream": false` inserted unconditionally
- `~/workspace/repos/phlow/crates/phlow-llm/src/transport.rs` — `LlmTransport::post_chat` (one decoded body per call), `BoundedBody` (byte cap)
- `~/workspace/repos/phlow/crates/phlow-llm/src/error.rs` — `LlmError` (no TruncatedStream variant)
- OpenAI streaming/chat-completions docs; WHATWG SSE framing — cited by the design; phlow has no streaming client to apply them to
- `~/workspace/gauntlet-design-tasks-71-100.md` — task-83 design (Wave 81–85)
