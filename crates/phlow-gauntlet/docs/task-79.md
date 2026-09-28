# task-79: offline fallback

**Kind:** rust (adversarial) · **Status:** fail (open) · **Wave:** 76–80 · **Commits:** pending (wave 76-80)

## ELI5

Offline fallback is "what the agent does when the model server is unreachable." The design wants offline to mean one of two precise things: the pinned model revision is cached locally, so run with the cached model and stamp the run record `offline: true` plus the exact revision — or the pinned model is NOT cached, so fail closed with a typed `model_unavailable_offline` naming the missing revision. Never silently substitute a different model. Phlow's reality: model resolution is a config string passed verbatim to Ollama — there is no cache, no revision pinning, no content-hash verification, no offline mode, no `offline: true` label. Offline detection is a boolean: `is_available()` returns false when the hub refuses, and `chat` fails with the untyped `LlmError::Transport`. That IS fail-closed (no fallback, no substitution) — but untyped and revision-blind. And because the model string is opaque, a provider-side tag move (same tag, different bytes) is undetectable by phlow.

## What this task attempts

- **Goal:** verify offline means pinned-cache-or-fail-closed — cached revision runs with `offline: true` labeling, missing revision fails with typed `model_unavailable_offline`, never a silent substitution — or document the gap.
- **Mechanism:** `src/tasks/task_79.rs` drives the REAL config/transport seam: online_resolution (real `load_config` → `model_for` → fake `/api/tags` → `is_available()` → real payload carries the model verbatim), offline_pinned_revision_absent (`set_model` accepts a fake `@sha256:` suffix as mere characters; no cache dir/revision field/offline flag), offline_fails_closed_untyped (fake hub refuses → `is_available()` false → `chat` → `LlmError::Transport`, not `model_unavailable_offline`), no_silent_substitution_no_verification (distinct revision strings reach the payload verbatim — never swapped, never verified).
- **Success criterion:** a pinned, hash-verified offline cache with typed offline errors, or the gap documented as the finding (and banked for Matt as a product decision).
- **Non-goals:** inventing revision pinning on gauntlet authority (it is a product decision, not a bug fix).

## What happened

Honest FAIL at `where = "seam"`, first attempt — offline means transport-error, not a mode:

- **V1:** online resolution works — config string → `model_for(Coder)` → `/api/tags` lists it → `is_available()` true → the real chat payload's `model` field carries it verbatim. No revision, no hash, no cache involved.
- **V2:** the pinned revision is unrepresentable — `OllamaConfig` exposes model/base_url/timeout_secs/context_length only; `FlowConfig::model_for` returns `&str` (no revision type); `set_model` validates non-emptiness only.
- **A1:** offline fails closed but UNTYPED — hub refused → `is_available()` false → `chat` → `Err(LlmError::Transport("connection refused…"))`. No fallback, no substitution — but not `model_unavailable_offline`, and it names no revision because there is no revision to name.
- **A2:** no silent substitution, no verification — `task-79-fixture@rev-a` and `@rev-b` reach the payload verbatim, never swapped; but a provider-side tag move under the same tag is invisible to phlow.

## Full technical depth

The seams are `crates/phlow-config/src/model.rs` (`model_for`, `OllamaConfig::set_model` validating non-empty only), `crates/phlow-llm/src/payload.rs::build_chat_payload` (the `model` field is the configured string, unchanged), and `crates/phlow-llm/src/transport.rs` (`OllamaBackend::is_available` collapsing every transport failure to `false`; `chat` propagating `LlmError::Transport`). The FakeLlmTransport queue proves both directions: a healthy `/api/tags` makes the backend available, a refused one makes it unavailable with no recovery path. The design's "cached-revision verification by content hash, not directory name" does not exist — there is no directory, no hash, no revision. The failure direction is safe (fail-closed), which is why the finding is the *missing precision*, not a wrong behavior.

Banked for Matt (product decision, NOT auto-implemented on gauntlet authority): whether phlow should pin model revisions — a content-hash-verified local cache, `offline: true` run-record labeling, and a typed `model_unavailable_offline`. That is a new product feature, not a bug fix; the gauntlet documents the gap and stops.

## Sources

- `~/workspace/repos/phlow/crates/phlow-config/src/model.rs` — `model_for`, `OllamaConfig::set_model`
- `~/workspace/repos/phlow/crates/phlow-llm/src/payload.rs` — `build_chat_payload` model field
- `~/workspace/repos/phlow/crates/phlow-llm/src/transport.rs` — `OllamaBackend::is_available`, `FakeLlmTransport`
- `~/workspace/gauntlet-design-tasks-71-100.md` — task-79 design (Wave 76–80)
