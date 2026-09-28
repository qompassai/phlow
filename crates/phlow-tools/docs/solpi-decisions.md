# SoL-Pi decisions — `phlow-tools`

Concept re-expressions of two NVlabs/SoL-Pi mechanisms for the tool
layer. Nothing here is a port: the ideas are re-expressed in Tiger Style
Rust against phlow's own tool model. Upstream is TypeScript for the Pi
coding agent; this crate has no Pi dependency.

## Verified upstream sources

- Repository: <https://github.com/NVlabs/SoL-Pi> (MIT License; verified
  2026-09-27). README defines the four mechanisms, states "every
  mechanism is opt-in and disabled by default" and "a missing
  configuration leaves every mechanism disabled", and lists the shared
  rules: no Pi patches, explicit opt-in, preserve evidence ("original
  observations remain available locally, and reducer failures leave the
  original result unchanged").
- Blog: <https://nvlabs.github.io/SoL-Pi/> — technical details, design
  rationale, and core insights behind the four mechanisms.
- Paper: arXiv 2609.20519, "SoL-Pi: Recursively Scaling Auto-Research
  Loops for Efficient Agent Harnesses".

## 1. Action Fusion (`solpi/action_fusion.rs`)

**Upstream:** "An edit or write can run its follow-up validation command
in the same tool call."

**Re-expression:** `fuse()` takes an action closure and a validation
closure. The action runs exactly once; on success the validation runs
with the action's value; the `FusionReceipt` records both halves plus a
bounded decision log. Validation output is capped at
`VALIDATION_OUTPUT_BYTES_MAX` (8 KiB); overflow discards the output and
marks the validation failed — flagged on the receipt, never silently
truncated. A validation failure never rolls back or re-runs the action;
the caller sees the failure and retries explicitly.

**Deliberate differences from upstream:** upstream fuses a shell
validation command; phlow has no shell tool (fail-closed by design), so
validation is a caller-supplied Rust closure. There is no command
string to inject into — the fusion surface cannot become a shell.

**Opt-in:** `FusionPolicy::disabled()` is the default; `fuse()` returns
`FusionError::NotEnabled` before running anything when disabled.

## 2. ObservationPack (`solpi/observation_pack.rs`)

**Upstream:** "Repeated large text results become stable handles with
exact paged recall." A result over 10 KiB is archived locally; later the
model sees a stable handle, the original size, and head/tail lines, and
can pull exact pages back on demand. Nothing is lost.

**Re-expression:** `PackStore::insert()` archives observations at or
above `PACK_BYTES_THRESHOLD` (10 KiB, mirroring the upstream value) and
returns an `ObservationProjection` (opaque handle, exact original byte
size, page count, byte head/tail samples). `page()` serves
`PAGE_BYTES_MAX` (4 KiB) pages verbatim; `original()` always returns the
full archived bytes. Small observations are rejected with
`BelowThreshold` so callers serve them inline. The store never evicts
silently: a full store returns `StoreFull` and the caller decides.

**Deliberate differences:** upstream replays the full result for the
next two requests before switching to the handle (Pi request plumbing);
this port packs on insert — the paging contract is identical, the
replay schedule is Pi-specific and not re-expressed.

**Opt-in:** `PackStore::new()` is disabled; every operation returns
`PackError::NotEnabled` until `PackStore::opt_in()`.

## Bounds (named, with units)

| Bound | Value | Rationale |
|---|---|---|
| `PACK_BYTES_THRESHOLD` | 10 KiB | Upstream eligibility value |
| `PAGE_BYTES_MAX` | 4 KiB | One page stays small enough to re-send cheaply |
| `HEAD_TAIL_BYTES` | 512 B | Projection sample, not a content channel |
| `OBSERVATION_BYTES_MAX` | 4 MiB | One observation cannot exhaust the store |
| `STORE_BYTES_MAX` | 16 MiB | Aggregate cap across `OBSERVATIONS_MAX` (256) |
| `VALIDATION_OUTPUT_BYTES_MAX` | 8 KiB | Untrusted validation text stays bounded |
| `FUSION_LOG_ENTRIES_MAX` | 32 | Decision log is observability, not storage |

## MITRE ATLAS mapping (v2026.05/2026.08 identifiers)

Covered, each with a mitigation and an adversarial test in-module:

- **AML.T0051 LLM Prompt Injection (indirect):** observations and
  validation output are inert bytes — stored verbatim, served verbatim,
  never executed or interpreted as instructions. Fusion never re-runs
  the action no matter what the validation output demands. Tests:
  `pack_stores_injected_content_verbatim_without_interpreting`,
  `fusion_never_reruns_action_on_injected_validation_output`.
- **AML.T0046 Spamming ML System with Chaff Data:** hard per-object and
  aggregate bounds; oversized input is rejected, a full store refuses
  instead of evicting. Tests: `pack_rejects_oversized_observation`,
  `pack_store_full_evicts_nothing_silently`,
  `fusion_caps_oversized_validation_output`.
- **AML.T0057 LLM Data Leakage:** handles are opaque and store-scoped —
  a handle from another store, an unknown handle, or an out-of-range
  page is rejected; projections expose only 512-byte head/tail samples.
  Test: `pack_handles_are_store_scoped_and_pages_bounded`.
- **AML.T0081 Modify AI Agent Configuration:** default-off; enabling
  requires an explicit `enable()`/`opt_in()` call — a default-constructed
  config or policy refuses. Tests: `fusion_default_policy_is_disabled`,
  `pack_refuses_without_opt_in`, config `default_config_disables_everything`.

Out of scope, with rationale (not covered, not claimed):

- **AML.T0010 AI Supply Chain Compromise:** no new dependencies added
  (std only); supply-chain review is a repo-level concern, and this
  change adds no new supply-chain surface.
- **AML.T0050 Command and Scripting Interpreter:** fusion validation is
  a caller-supplied Rust closure, not a shell string — no interpreter is
  introduced by this feature.
- **AML.T0054 LLM Jailbreak / AML.T0056 Meta Prompt Extraction:** no
  LLM, prompts, or system-prompt store exist in these modules.
- **AML.T0025 Exfiltration via Cyber Means / AML.T0055 Unsecured
  Credentials:** pure in-memory modules — no I/O, no network, no
  credential handling.

## What was left out

- Upstream's two-request full-replay schedule (Pi-specific).
- Any network/model calls: this port is fully local.
- Cross-store handle portability: handles are deliberately store-scoped.
