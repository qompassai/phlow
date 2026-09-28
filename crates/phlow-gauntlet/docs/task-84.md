# task-84: provider outage failover

**Kind:** rust (adversarial) · **Status:** fail (open) · **Wave:** 81–85 · **Commits:** pending (wave 81-85)

## ELI5

When your main provider goes down, a router can fail over to a backup — but only to *pre-approved* backups, never to a provider it discovers or invents on the spot. And there's a hard data boundary: a workload marked local-only (your Ollama on your own machine) must NEVER fail over to a cloud provider, because that would send your private data to someone else's computer just because the local one hiccuped. Phlow has no router and no backups: one Ollama backend, one URL. An outage fails closed — the error comes back to you, `is_available()` says false. And since there's exactly one configured URL (default: your own loopback), no cloud request can ever be emitted. But the *policy* the design asks to verify — pre-approved secondaries, logged decisions, local-only enforcement — has no implementation.

## What this task attempts

- **Goal:** verify the provider router / failover policy — failover only to pre-approved secondaries, local-only workloads never leaving the machine (asserted at the HTTP layer), every failover decision logged with cause, down targets bounded and typed — or document the absence with file evidence.
- **Mechanism:** `src/tasks/task_84.rs` runs exact-token source scans (task_48 pattern) for `failover` / `fail_over` over every product crate's `src/**/*.rs` (zero hits), inspects `OllamaBackend::base_url()` (exactly one URL) and `OllamaConfig`'s fields (no secondary field), and drives the REAL `OllamaBackend` over a recording `LlmTransport` (URL log + scripted failure): a scripted connection-refused outage makes `chat` return the error after exactly 1 recorded request (no reroute, `is_available()` false); across the outage and availability check, 0 cloud requests were recorded — the only URL ever requested is the loopback base URL.
- **Success criterion:** the failover policy verified, or the absence documented with file evidence (and banked for Matt as a product decision).
- **Non-goals:** inventing multi-provider routing on gauntlet authority (it is a product decision, not a bug fix).

## What happened

Honest FAIL at `where = "seam"`, first attempt — the absence IS the finding:

- **V1:** single-provider architecture — `base_url()` returns exactly one URL; `OllamaConfig` has no `secondary_url` or failover target list.
- **V2:** no failover vocabulary — 0 `failover` / `fail_over` hits across the workspace.
- **A1:** outage not rerouted — 1 recorded request, error propagates, `is_available()` false. Fail-closed holds by construction; no failover decision is logged because there is no failover machinery to log.
- **A2:** local-only invariant by architecture — 0 cloud requests at the HTTP layer; the design's "never leave the machine" holds vacuously (one loopback URL, nothing to discover or improvise), but the routing POLICY (workload marked local-only, router refuses cloud failover) has no implementation: no workload marking, no router, no cloud secondary to refuse.

## Full technical depth

The probe scans every `crates/*/src/**/*.rs` with exact-token (case-insensitive) matching, bounded like task_48, excluding only the gauntlet crate. Any hit fails the case loudly ("the absence finding is refuted"), keeping the finding falsifiable.

Distinct from task-26 (circuit breaker: per-downstream *admission* state machine): this is *routing policy* with a data-boundary invariant — a different seam, a different property. The recording transport is the HTTP-layer assertion harness the design asks for: it logs every URL the backend actually requests, so "no cloud request is emitted" is measured, not asserted. The partial truth is recorded honestly: fail-closed and no-cloud-exfiltration hold by architecture today, so a future router must preserve them — but the design's failover dimension (pre-approved secondaries, logged decisions, bounded cascade) cannot be exercised against a client with no router.

Banked for Matt (product decision, NOT auto-implemented on gauntlet authority): whether phlow should gain multi-provider routing with a failover policy — pre-approved secondaries, logged decisions, local-only data-boundary enforcement.

## Sources

- `~/workspace/repos/phlow/crates/phlow-llm/src/transport.rs` — OllamaBackend (one base_url), is_available fail-closed
- `~/workspace/repos/phlow/crates/phlow-config/src/model.rs:180-187` — OllamaConfig fields (no secondary)
- `~/workspace/gauntlet-design-tasks-71-100.md` — task-84 design (Wave 81–85)
