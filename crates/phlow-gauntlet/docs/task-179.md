# task-179: bridge stdio framing

**Kind:** rust · **Status:** pass · **Wave:** 29 · **Commits:** pending (wave 29)

## ELI5

The agent and the browser talk through a pipe, one message per line — like passing notes where each note ends with a newline so you know where one note stops and the next starts. This task checks the note-passing machinery itself: if a note arrives torn into five pieces, it must be taped back together into exactly one note (not zero, not two). If the other side hangs up mid-note, that must be reported as a specific "they hung up" error, not silence. And if someone tries to pass a note bigger than the agreed limit (4 MB), it must be refused with a typed error instead of blowing up memory.

## What this task attempts

- **Goal:** prove the newline-delimited stdio framing contract is exact: split writes reassemble once, mid-message EOF is typed, oversize frames are rejected.
- **Mechanism:** `crates/phlow-gauntlet/src/bridge.rs` (`StdioFramer`: `push_bytes`, `drain_messages`, `pending_bytes`) driven by `crates/phlow-gauntlet/src/tasks/task_179.rs`; assertions in `crates/phlow-gauntlet/tests/task_179.rs`.
- **Success criterion:** one JSON-RPC message split across 5 writes → exactly one delivery, handler invoked once, second drain empty; EOF with a partial message → `BridgeError::Eof`; a frame over `MAX_MESSAGE_BYTES` (4 MiB) → `BridgeError::FramingTooLarge`.
- **Non-goals:** the MCP protocol semantics on top of the framing (task 178 covers initialize); the DevTools HTTP surface (task 180).

## What happened

Pass on the second iteration: 3/3, `test result: ok. 3 passed; 0 failed; finished in 0.00s`. The first iteration failed all three cases for a single shared driver bug (below), not a framing defect — the framer itself was correct from the start.

Wave-wide honesty note: the wave's first `cargo build -p phlow-gauntlet` failed with **23 errors and 1 warning** before any test ran; all were fixed before the first green gate.

## Where it went wrong

- **Stage:** all three driver cases.
- **Symptom:** every assertion failed with an empty failure message — the reports showed `failures: []` while `passed` was false.
- **Evidence:** the test harness prints `report.failures.join("; ")`; it printed nothing, yet `report.passed` was false.
- **Root cause:** all eight task drivers set `report.passed = failures.is_empty()` but never copied the local `failures` vector into `report.failures`. The verdict was computed correctly and then thrown away before reporting. A reporting bug, not a product bug.

## The fix — what changed and why

- **Changed:** `src/tasks/task_178.rs` through `task_185.rs` — every `finish_case` now does `report.passed = failures.is_empty(); report.failures = failures;`.
- **Why:** the failure list is the evidence channel; computing a verdict without publishing its reasons makes failures undebuggable. No alternative was viable — the field exists precisely for this.
- **Source:** internal contract of `CaseReport` (`crates/phlow-gauntlet/src/skillopt/driver.rs`): `failures` carries the case's failure lines.
- **Validation agents:** re-ran all eight focused test binaries; the previously silent failures became legible and the real defects (tasks 178, 180, 181, 183) were then fixed on evidence.
- **Citations:** none external — internal reporting contract.

## Full technical depth

The framing contract is: **newline-delimited JSON**, one message per line, max `MAX_MESSAGE_BYTES` (4 MiB) per line. `StdioFramer` buffers raw bytes; `push_bytes` appends; `drain_messages` scans for `\n`, splits complete lines, parses each as JSON, and returns them — a trailing partial line stays buffered (`pending_bytes` exposes it). V1 writes one JSON-RPC `initialize` message in 5 uneven chunks, drains, and asserts exactly one message, one handler invocation, and that a second drain yields nothing (no duplication, no residue). V2 pushes a partial line then signals EOF and asserts the typed `BridgeError::Eof` (not a silent drop, not a parse error). V3 pushes a line exceeding 4 MiB and asserts `BridgeError::FramingTooLarge { bytes }` before any unbounded allocation — the bound is checked against the buffered length, so the rejection happens without materializing the giant string.

This framing layer is what `mcp_initialize` (task 178) and every scripted stdio exchange in the wave runs on. The cyclic-value policy for this wave is **rejection** (`CYCLIC_CONTRACT = "reject"` → `BridgeError::CyclicValue`, exercised in task 181); the framer itself is policy-agnostic — it frames bytes, and the JSON layer above enforces the policy.

Ghostex lineage: the ephemeral stdio bridge concept is adapted from maddada/Ghostex @ `c91146607205ac49303d1bcfe2fd6f9a86741500`. Ghostex's CEF/GPUI implementation was not ported; this is a Tiger Style Rust re-implementation of the concept.

## Sources

- `crates/phlow-gauntlet/src/bridge.rs` — `StdioFramer`, `MAX_MESSAGE_BYTES`, `BridgeError::{Eof, FramingTooLarge, FramingNotJson}`
- `crates/phlow-gauntlet/src/tasks/task_179.rs` — the three driver cases
- `crates/phlow-gauntlet/tests/task_179.rs` — the integration assertions
- maddada/Ghostex @ `c91146607205ac49303d1bcfe2fd6f9a86741500` — adapted concept (CEF/GPUI implementation not ported)
