# task-81: provider auth failure modes

**Kind:** rust (adversarial) · **Status:** fail (open) · **Wave:** 81–85 · **Commits:** pending (wave 81-85)

## ELI5

When a provider rejects your API key, the client should know *why*: an expired key (get a new one), a revoked key (something is wrong, investigate), a malformed header (your code has a bug), a 403 with "insufficient scope" (the key is fine — widen what it's allowed to do). Each of those is a different typed error with a different fix. And a 401 must never be retried: retrying a dead key burns quota and looks like credential stuffing. Phlow's provider client has none of this — because it has no auth at all. The only provider client is the Ollama backend, and Ollama needs no API key. There is no Authorization header, no key to expire, no key to revoke, no 401 to classify — the whole auth-failure dimension has no seam to stand on.

## What this task attempts

- **Goal:** verify the provider client's auth surface — typed, distinct errors for expired vs revoked vs malformed vs wrong-project vs insufficient-scope, 401s never retried, no key material in error messages (task-43's lesson) — or document the absence with file evidence.
- **Mechanism:** `src/tasks/task_81.rs` runs exact-token source scans (task_48 pattern) for `authorization` / `api_key` / `bearer` / `401` / `403` over `phlow-llm/src` (only the credential scrubber's own test fixtures in `redact.rs`, classified — the scrubber removes credentials, it never sends them), inspects `OllamaConfig`'s fields (no key field), enumerates `LlmError`'s variants (no auth variant), and drives the REAL `OllamaBackend` over `FakeLlmTransport` with scripted 401/403 failures: a 401 arrives as the untyped `Transport(String)`; a 401 followed by a queued success leaves chat returning the 401 (exactly one attempt, zero retries); a 403 insufficient-scope arrives as `Transport` with the scope signal only in free-text detail.
- **Success criterion:** typed auth-failure classification with the no-retry-on-401 rule, or the absence documented with file evidence (and banked for Matt as a product decision).
- **Non-goals:** inventing API-key auth on gauntlet authority (it is a product decision, not a bug fix).

## What happened

Honest FAIL at `where = "seam"`, first attempt — the absence IS the finding:

- **V1:** no auth surface — `authorization` / `api_key` / `bearer` / `401` / `403`: 0 hits each over phlow-llm/src; `OllamaConfig` (model.rs:180-187) has no key field. No key material exists to classify, rotate, or leak.
- **V2:** auth failures untyped — `LlmError` = BadRequest, ResponseTooLarge, BadJson, BadShape, Transport (no auth variant); a scripted 401 arrives as `Transport("401 Unauthorized")`.
- **A1:** 401 never retried — one attempt, zero retries, measured by script consumption. The design's no-retry half holds vacuously (no retry loop exists in `OllamaBackend::chat`), but the error is untyped: no `auth_failed` class.
- **A2:** 403 insufficient-scope untyped — the scope signal lives only in free-text detail; the operator cannot be told "widen the scope" vs "rotate the key" by error type.

## Full technical depth

The probe scans every `phlow-llm/src/**/*.rs` with exact-token (case-insensitive) matching, bounded like task_48, excluding only the gauntlet crate itself (the harness's own probes use the design vocabulary). The classification is falsifiable: any hit fails the case with "the absence finding is refuted". The OpenAI references elsewhere in the workspace (`phlow-tools` function-calling schema shapes) are request-payload shapes, not an API client — verified as a different seam.

The no-retry measurement is behavioral, not textual: the fake is queued `[Err(401), Ok(success)]`; if the backend retried, the second reply would be consumed and `chat` would succeed. It returns the 401 error — exactly one attempt. This is the good half of the design (retrying auth failures mimics credential stuffing), achieved by absence of retry machinery rather than by a rule. The classification half — the typed errors that let the TUI/CLI render distinct actionable messages per failure class — cannot exist without an auth surface.

Banked for Matt (product decision, NOT auto-implemented on gauntlet authority): whether phlow should gain API-key auth (e.g. for future OpenAI/Anthropic providers) with typed auth-failure classification and the no-retry-on-401 rule.

## Sources

- `~/workspace/repos/phlow/crates/phlow-llm/src/transport.rs` — OllamaBackend, single post_chat call, no retry loop
- `~/workspace/repos/phlow/crates/phlow-llm/src/error.rs` — LlmError variants (no auth variant)
- `~/workspace/repos/phlow/crates/phlow-config/src/model.rs:180-187` — OllamaConfig fields (no key)
- OpenAI API error docs; Anthropic authentication error shapes — cited by the design as the pass-criteria source; phlow has no such client to apply them to
- `~/workspace/gauntlet-design-tasks-71-100.md` — task-81 design (Wave 81–85)
