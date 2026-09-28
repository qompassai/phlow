# task-82: provider rate-limit protocol compliance

**Kind:** rust (validation) · **Status:** fail (open) · **Wave:** 81–85 · **Commits:** pending (wave 81-85)

## ELI5

Providers rate-limit you: "slow down" comes back as HTTP 429 with a `Retry-After: 2` header meaning "wait 2 seconds, then try again". A well-behaved client reads that header, waits about 2 seconds, retries; distinguishes 429 (back off and retry) from 503 (longer backoff) from 401 (never retry — see task-81); clamps absurd waits (`Retry-After: 3600` shouldn't freeze the agent for an hour without saying so); gives up boundedly if the provider 429s forever; and keeps per-provider quota counters that don't leak between providers. Phlow's client speaks none of this protocol. It makes one attempt per call and hands the error to the caller. There is no 429 classification, no header parsing, no retry loop to clamp or bound, no quota state.

## What this task attempts

- **Goal:** verify the provider client speaks the rate-limit protocol — Retry-After honored within tolerance and clamped at a named max, 429/503/401 on distinct code paths, infinite-429 terminating bounded, per-provider counters never leaking — or document the absence with file evidence.
- **Mechanism:** `src/tasks/task_82.rs` runs exact-token source scans (task_48 pattern) for `429` / `retry-after` / `retry_after` / `quota` over `phlow-llm/src` (all zero hits), drives the REAL `OllamaBackend` over `FakeLlmTransport` with scripted 429 failures (a 429 arrives as untyped `Transport`; 5 calls against an always-429 fake consume exactly 5 replies — one attempt per call, zero internal retries), and inspects the `LlmTransport::post_chat` signature (no header slot — Retry-After has no wire-to-decision path).
- **Success criterion:** the rate-limit protocol spoken correctly, or the absence documented with file evidence (and banked for Matt as a product decision).
- **Non-goals:** inventing the 429/Retry-After protocol on gauntlet authority (it is a product decision, not a bug fix).

## What happened

Honest FAIL at `where = "seam"`, first attempt — the absence IS the finding:

- **V1:** no 429 classification — 0 hits; a scripted 429 arrives as `Transport("429 Too Many Requests")`, the same variant as a refused connection. No distinct 429 code path.
- **V2:** no Retry-After parsing — 0 hits; and the trait signature `post_chat(base_url, payload, timeout)` has no header parameter, so the header could not reach the backend even if a transport read it.
- **A1:** no bounded retry — 5 calls against an always-429 provider = 5 attempts, 0 internal retries. The design's clamped waits (`rate_limit_wait_exceeded`) and bounded give-up (`rate_limit_exhausted`) are unrepresentable: the retry loop they would live in does not exist. Termination-per-call is trivial, but backoff, clamping, and give-up are the caller's unbounded problem.
- **A2:** no per-provider quota state — 0 `quota` hits; the backend holds (cfg, base_url, transport) only. "Counters never leak across providers" is vacuous with no counters.

## Full technical depth

The probe scans every `phlow-llm/src/**/*.rs` with exact-token (case-insensitive) matching, bounded like task_48, excluding only the gauntlet crate. Any hit fails the case loudly ("the absence finding is refuted"), keeping the finding falsifiable as the codebase evolves.

Distinct from task-27 (which bounds *aggregate retry load* under a storm — the thundering herd): this task is *per-provider protocol* compliance — header parsing, status classification, quota state. Complementary layers, different properties. The design's pass criteria (Retry-After honored within tolerance, clamped at the max; infinite-429 terminates bounded; per-provider counters never leak) all presuppose a wait-and-retry state machine; phlow-llm has none.

Banked for Matt (product decision, NOT auto-implemented on gauntlet authority): whether phlow-llm should speak the 429/Retry-After protocol — honored-and-clamped waits, bounded give-up, per-provider quota — instead of the current single-attempt client.

## Sources

- `~/workspace/repos/phlow/crates/phlow-llm/src/transport.rs` — OllamaBackend (one post_chat per chat), LlmTransport signature (no header slot)
- `~/workspace/repos/phlow/crates/phlow-llm/src/error.rs` — LlmError (no rate-limit variant)
- OpenAI rate-limit docs; Anthropic rate-limit headers — cited by the design; phlow has no client speaking them
- `~/workspace/gauntlet-design-tasks-71-100.md` — task-82 design (Wave 81–85)
