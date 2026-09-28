# task-184: bridge cross-talk isolation

**Kind:** rust · **Status:** pass · **Wave:** 29 · **Commits:** pending (wave 29)

## ELI5

Two temp workers, two jobs, two browsers — and neither may ever see the other's work. "Cross-talk" is when a result from browser A leaks into browser B's answers: imagine worker B confidently reporting worker A's findings as their own. This task runs two bridges side by side, plants a secret marker in browser A's results, then scans browser B's results for it (three evaluations, byte scans). It also tries to cheat: reaching browser A's tab using browser B's credentials — with a raw id, with B's own id format, and with a made-up id. Every cheat must be refused with a typed `UnknownTarget` error, and A's own id must still work. The namespaces are disjoint by construction (task-prefixed ids), and the test proves the construction holds, not just promises it.

## What this task attempts

- **Goal:** prove two concurrent bridges are fully isolated: no result cross-talk, foreign target ids refused, id namespaces disjoint.
- **Mechanism:** `crates/phlow-gauntlet/src/bridge.rs` (task-scoped target ids, `evaluate` target lookup, `BridgeError::UnknownTarget`) driven by `crates/phlow-gauntlet/src/tasks/task_184.rs`; assertions in `crates/phlow-gauntlet/tests/task_184.rs`.
- **Success criterion:** marker `EXFIL-MARKER-T1-9f2c` present in T1's stream, absent from all three of T2's scanned evaluations (`crosstalk=false`); three foreign id shapes (raw colliding id, T2's namespaced id, bogus T2 id) → `UnknownTarget` every time (`foreign_ids_refused=3`); T1's own id still evaluates; namespace overlap count is zero.
- **Non-goals:** network-level isolation (loopback-only binding is a separate invariant); more than two concurrent bridges (the mechanism is per-bridge state, so two is the representative case).

## What happened

Pass on the second iteration: 3/3, `test result: ok. 3 passed; 0 failed; finished in 0.02s`. The first iteration failed all three cases from the shared driver-reporting bug (task 179's fix). No isolation defect was found; the namespacing was correct from the start.

Wave-wide honesty note: the wave's first `cargo build -p phlow-gauntlet` failed with **23 errors and 1 warning** before any test ran; all were fixed before the first green gate.

## Where it went wrong

The only failure was the cross-task reporting bug documented in task-179: every `finish_case` computed `report.passed` but never assigned `report.failures`, so all three cases reported failure with empty evidence. The isolation logic itself passed on its first real run. Kept per the template's honesty requirement.

## The fix — what changed and why

- **Changed:** `src/tasks/task_178.rs`–`task_185.rs` — `finish_case` now assigns `report.failures = failures` alongside the verdict (shared with task 179's fix).
- **Why:** see task-179. Recorded here because this task's first run was its victim.
- **Source:** internal `CaseReport` contract.
- **Validation agents:** 3/3 green after the reporting fix; no isolation code changed.
- **Adversarial agents:** the three cases *are* the adversarial coverage — the fixture is deliberately sharp: the raw colliding id "collides with T1's raw id by design" (the evidence asserts this note is present), so the refusal is proven against an id that *looks* valid, not just against garbage.

## Full technical depth

Each `Bridge` owns a task-scoped id namespace: target ids are prefixed with the task id at creation (`launch_pair` returns both bridges and both namespaces), and `evaluate` resolves a target id strictly within the calling bridge's namespace. A lookup miss — raw id, other-bridge namespaced id, or fabricated id — returns the typed `BridgeError::UnknownTarget`, never a neighboring bridge's target. There is no global target registry; the absence of a shared lookup table *is* the isolation mechanism, which is why `namespaces_disjoint_by_construction` can assert `overlap=0` structurally rather than probabilistically.

The cross-talk scan is byte-level: T1 evaluates an expression whose canned result contains `EXFIL-MARKER-T1-9f2c`; T2 performs three evaluations over the same shapes; every byte of T2's results is scanned for the marker. `crosstalk=false` is therefore an observation over the full result bytes, not a spot check.

The cyclic-value policy (rejection, task 181) and newline-delimited framing (task 179) compose unchanged: each bridge frames and bounds its own stream independently, so neither the framing buffer nor the traversal state is shared.

Ghostex lineage: the isolated-bridge concept is adapted from maddada/Ghostex @ `c91146607205ac49303d1bcfe2fd6f9a86741500`. Ghostex's CEF/GPUI implementation was not ported; this is a Tiger Style Rust re-implementation of the concept.

## Sources

- https://chromedevtools.github.io/devtools-protocol/ — official CDP docs: Target domain, target id semantics, `Runtime.evaluate`
- `crates/phlow-gauntlet/src/bridge.rs` — task-scoped target ids, `evaluate` target resolution, `BridgeError::UnknownTarget`
- `crates/phlow-gauntlet/src/tasks/task_184.rs` — the three adversarial driver cases, `launch_pair`, `MARKER`
- `crates/phlow-gauntlet/tests/task_184.rs` — the integration assertions
- maddada/Ghostex @ `c91146607205ac49303d1bcfe2fd6f9a86741500` — adapted concept (CEF/GPUI implementation not ported)
