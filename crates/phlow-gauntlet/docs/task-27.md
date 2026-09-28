# task-27: retry backoff under storm

**Kind:** rust · **Status:** fail (open) · **Wave:** 26–30 · **Commits:** pending (wave 26-30)

## ELI5

When a call fails transiently (the downstream hiccuped), a retry policy
tries again — but not immediately and not forever. "Backoff" means each
retry waits longer than the last (exponential backoff: 1s, 2s, 4s…);
"jitter" randomizes those waits so a fleet of callers doesn't retry in
lockstep and re-hammer the downstream (the "thundering herd"). An
"attempt cap" bounds the total tries, and a typed "exhaustion" error tells
the caller "we tried N times, all failed" instead of one raw failure.
"Correct" for this task means the design's numbers hold: bounded attempts,
exponential-with-jitter delays, and a storm of callers that gets
throttled rather than forwarded 1:1.

## What this task attempts

- **Goal:** drive phlow's real downstream-call wrapper,
  `phlow_llm::transport::OllamaBackend::chat`
  (`crates/phlow-llm/src/transport.rs`), through a scripted implementation
  of the real `LlmTransport` trait and show the retry design holds:
  transient failures retried with backoff+jitter, attempt cap with an
  exhaustion type, storms throttled.
- **Mechanism:** the real `OllamaBackend::chat` → `LlmTransport::post_chat`
  path, with a `Probe` transport that scripts failures/successes,
  timestamps every downstream arrival, and counts arrivals.
- **Success criterion:** the design's retry pass criteria (backoff
  schedule, jitter, attempt cap, exhaustion error).
- **Non-goals:** inventing a retry policy. The design says an evidenced
  `where = "seam"` failure is the correct result when the design says an
  absent seam is valid.

## What happened

Fail, open seam — on the first and only attempt. Phlow has no
retry/backoff machinery on its Rust downstream-call paths
(`OllamaBackend::chat`, `MsgpackTransport::exec`, the HTTP transport are
all single-shot). One transient failure is returned to the caller; a
storm reaches the downstream unthrottled; no delay schedule is ever
injected. The four cases pin that honest finding:

- `transient_failure_is_not_retried` (V): one transient failure reaches
  the downstream exactly once — the queued success reply is still waiting
  for an explicit second call. The path did not retry.
- `no_attempt_cap_or_exhaustion_error` (V): 20 consecutive failures
  under 20 explicit calls — every error is the raw
  `LlmError::Transport`. No attempt cap engages; no typed exhaustion
  error exists.
- `storm_reaches_downstream_unthrottled` (A): a 100-caller storm — all
  100 reach the downstream 1:1. No backoff, no jitter, no cap.
- `no_backoff_delays_injected` (A): per-call wall-clock latency across
  10 consecutive failures is flat, not exponential (worst per-call well
  under the 1000 ms bound) — no delay schedule is injected anywhere in
  the path.

The task-level `run` reports `fail` with `where = "seam"`: the retry
pass criteria cannot be evaluated.

Diver-owned finding (flagged, never fixed on gauntlet authority):
diver's `lua/ai/harness/supervisor.lua` `retry_run` DOES have
exponential backoff with jitter (`RETRY_ATTEMPTS_MAX = 4`) — but that is
the diver repo's Lua harness seam, not phlow's Rust transport path, and
this task's kind is rust. Fixing or claiming it is out of scope.

## The fix — what changed and why

No fix — this is a documented design gap, never fixed under gauntlet
authority. The gauntlet-side work was refusing to pretend:

- **Changed:** `crates/phlow-gauntlet/src/tasks/task_27.rs` (new) —
  the doc comment states plainly "phlow has no retry wrapper"; the cases
  prove no-retry, no-cap, unthrottled storm, and flat latency without
  claiming any of that is a retry policy.
- **Why:** asserting a pass from "the second explicit call succeeds"
  would launder caller-driven retries into a transport retry policy. The
  honest verdict is the seam failure with the recon evidence attached.
- **Source:** `crates/phlow-llm/src/transport.rs`
  (`OllamaBackend::chat` is single-shot; error propagates as
  `LlmError::Transport`).
- **Validation agents:** the 2 validation tests
  (`transient_failure_is_not_retried`,
  `no_attempt_cap_or_exhaustion_error`) assert the failure is not
  retried and no cap/exhaustion type exists.
- **Adversarial agents:** the 2 adversarial tests
  (`storm_reaches_downstream_unthrottled`,
  `no_backoff_delays_injected_and_task_reports_seam`) forward a
  100-call storm and measure flat per-call latency — and the folded-in
  task-level assertion pins `where = "seam"`.

## Full technical depth

The retry decision lives wherever the caller is: `OllamaBackend::chat`
makes exactly one `post_chat` attempt per call and returns whatever
`LlmError` comes back. There is no retry loop, no sleep between
attempts, no jitter source, no attempt counter, no `RetryExhausted`
error variant. The `Probe` timestamps every arrival in
`call_times_ms`; across 10 consecutive failures the worst per-call
latency stays far under 1000 ms and the arrival-time series is flat —
the behavioral signature of "no delay schedule". Under a 100-call
storm all 100 arrivals land with raw `LlmError::Transport` errors
returned 1:1 to the callers.

What is missing for the design's retry policy: a bounded retry loop
around the single-shot call with exponential backoff and jitter, an
attempt cap, a typed exhaustion error distinguishing "downstream
failed" from "we gave up after N tries", and — under storm — a
throttle (concurrency limit or queue) so the downstream is not hit
1:1. The design gap: either the transport grows a retry policy or
callers own retries explicitly and the single-shot contract is
documented as the intended behavior (with the unthrottled-storm cost
accepted explicitly).

## Sources

- Primary: `crates/phlow-llm/src/transport.rs` (`OllamaBackend::chat`
  single-shot; `LlmError::Transport` propagation).
- Diver-owned (flagged, not modified):
  `~/workspace/repos/diver/lua/ai/harness/supervisor.lua` (`retry_run`,
  `RETRY_ATTEMPTS_MAX = 4`, exponential backoff with jitter).
- Driver: `crates/phlow-gauntlet/src/tasks/task_27.rs` (probe +
  four cases + `seam_finding`).
- Tests: `crates/phlow-gauntlet/tests/task_27.rs` (2V/2A).
