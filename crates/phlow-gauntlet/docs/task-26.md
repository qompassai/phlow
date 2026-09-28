# task-26: circuit breaker

**Kind:** rust · **Status:** fail (open) · **Wave:** 26–30 · **Commits:** pending (wave 26-30)

## ELI5

A circuit breaker is the electrical panel of a networked program. When the
downstream service starts failing, the breaker "trips" after a few failures
and immediately rejects new calls (fast-fail) instead of hammering the dead
service. After a cooldown it lets one call through as a probe (half-open):
if that succeeds the breaker closes and traffic resumes; if it fails the
breaker stays open. "Correct" for this task means the design's numbers hold:
a configurable failure threshold trips the breaker, calls during the open
state fast-fail without touching the downstream, and recovery happens
through the half-open probe.

## What this task attempts

- **Goal:** drive phlow's real downstream-call wrapper,
  `phlow_llm::transport::OllamaBackend::chat`
  (`crates/phlow-llm/src/transport.rs`), through a scripted implementation
  of the real `LlmTransport` trait and show the circuit-breaker design
  holds: trip threshold, fast-fail while open, half-open recovery.
- **Mechanism:** the real `OllamaBackend::chat` → `LlmTransport::post_chat`
  path, with a `Probe` transport that scripts failures/successes and
  counts every downstream arrival.
- **Success criterion:** the design's breaker pass criteria (threshold
  trip, fast-fail, half-open probe recovery).
- **Non-goals:** inventing a breaker. The design says an evidenced
  `where = "seam"` failure is the correct result when the design says an
  absent seam is valid.

## What happened

Fail, open seam — on the first and only attempt. Phlow has no circuit
breaker on its downstream-call path. `OllamaBackend::chat` calls
`self.transport.post_chat(...)` directly (source scan: zero hits for
`breaker`, `circuit`, `half_open`, `fast_fail`, `open_circuit` in
`crates/phlow-llm/src/transport.rs`); every call reaches the downstream,
failures included. The four cases pin that honest finding:

- `consecutive_failures_all_reach_downstream` (V): 5 consecutive
  downstream failures — all 5 reach the downstream, none fast-fails.
- `recovery_is_per_call` (V): 3 failures then a scripted success — the
  4th call succeeds immediately. There is no half-open probe because
  there is no breaker state to probe; "recovery" is just the next call
  succeeding.
- `failure_storm_absorbed_one_to_one` (A): a 50-call failure storm — all
  50 reach the downstream, 0 fast-failed. The honest cost of the missing
  breaker, measured: a breaker would have capped this at the open
  threshold; without one the downstream absorbs the full storm.
- `calls_are_stateless` (A): 6 interleaved failures/successes — every
  outcome matches its scripted reply exactly, proving the path keeps no
  cross-call state (no failure counter, no breaker state machine).

The task-level `run` reports `fail` with `where = "seam"`: the breaker's
pass criteria cannot be evaluated.

## The fix — what changed and why

No fix — this is a documented design gap, never fixed under gauntlet
authority. The gauntlet-side work was refusing to pretend:

- **Changed:** `crates/phlow-gauntlet/src/tasks/task_26.rs` (new) —
  the doc comment states plainly "phlow has no circuit breaker"; the
  cases prove 1:1 forwarding, per-call recovery, storm absorption, and
  statelessness without claiming any of that is a breaker.
- **Why:** asserting a pass from "calls succeed after failures" would
  launder a plain pass-through into a circuit breaker. The honest verdict
  is the seam failure with the recon evidence attached.
- **Source:** `crates/phlow-llm/src/transport.rs`
  (`OllamaBackend::chat` → `self.transport.post_chat`, no admission
  control).
- **Validation agents:** the 2 validation tests
  (`consecutive_failures_all_reach_downstream`,
  `recovery_is_per_call`) assert every failure reaches the downstream
  and recovery is per-call.
- **Adversarial agents:** the 2 adversarial tests
  (`failure_storm_absorbed_one_to_one`,
  `calls_are_stateless_and_task_reports_seam`) absorb a 50-call storm
  and prove cross-call statelessness — and the folded-in task-level
  assertion pins `where = "seam"`.

## Full technical depth

`OllamaBackend` holds an `Arc<dyn LlmTransport>` plus config (base URL,
model, timeout). `chat()` builds the request payload and calls
`self.transport.post_chat(&self.base_url, &payload, timeout)` exactly
once per invocation; the returned `LlmError` (including
`LlmError::Transport`) propagates to the caller unmodified. There is no
failure counter, no threshold, no cooldown timer, no half-open probe
slot, no breaker-typed error variant — a source scan of the whole
`transport.rs` for `breaker`/`circuit`/`half_open`/`fast_fail`/
`open_circuit` finds nothing.

What is missing for a circuit breaker: a failure counter with a
configurable trip threshold, an open state that fast-fails calls without
touching the downstream (with a distinct error type so callers can tell
"breaker open" from "downstream failed"), a cooldown timer, and a
half-open probe that admits exactly one call to test recovery. The
design gap: either the LLM transport grows breaker state or the
pass-through is documented as the intended behavior under downstream
outage (with the storm-absorption cost accepted explicitly).

## Sources

- Primary: `crates/phlow-llm/src/transport.rs` (`OllamaBackend::chat`
  → `LlmTransport::post_chat`; needle scan documented in the driver).
- Driver: `crates/phlow-gauntlet/src/tasks/task_26.rs` (probe +
  four cases + `seam_finding`).
- Tests: `crates/phlow-gauntlet/tests/task_26.rs` (2V/2A).
