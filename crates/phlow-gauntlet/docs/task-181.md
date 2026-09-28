# task-181: hostile page payload bounds

**Kind:** rust · **Status:** pass · **Wave:** 29 · **Commits:** pending (wave 29)

## ELI5

A web page can lie about how much data it's handing you: "here's a string" that is actually 50 megabytes, or an object nested thousands of layers deep, or an object that refers to itself in a circle so naive code walks it forever. This task is the bouncer: it feeds the bridge exactly those hostile payloads and checks that each is defanged in a specific, typed way — the 50 MB string is cut to 64 KB with a `truncated: true` marker, the deep object stops at 32 levels with a depth marker, the cyclic one is refused outright with a `CyclicValue` error, and an allocation meter proves the agent-visible output stayed bounded.

## What this task attempts

- **Goal:** prove every hostile `Runtime.evaluate` result shape is bounded: giant strings truncated, deep objects depth-capped, cyclic objects rejected, output bytes metered.
- **Mechanism:** `crates/phlow-gauntlet/src/bridge.rs` (`bound_value` + helpers, `AllocationMeter`, `evaluate_limited`, `BridgeError::CyclicValue`) driven by `crates/phlow-gauntlet/src/tasks/task_181.rs`; assertions in `crates/phlow-gauntlet/tests/task_181.rs`.
- **Success criterion:** 50 MiB string → text ≤ 65,536 bytes with `truncated=true`; 512-deep object → depth marker `__bridge_depth_exceeded_at=32`; cyclic object → typed `CyclicValue` refusal; meter reports `bytes_copied ≤ MAX_RESULT_BYTES`.
- **Non-goals:** real-browser hostile payloads (scripted fixtures cover the protocol shapes per the CDP docs; the traversal is transport-independent).

## What happened

Pass on the fourth iteration: 4/4, `test result: ok. 4 passed; 0 failed; finished in 0.08s`. Three distinct failures preceded it. This task was the wave's hardest — it found a real stack-overflow-class defect in the test fixture itself and one in the traversal.

Wave-wide honesty note: the wave's first `cargo build -p phlow-gauntlet` failed with **23 errors and 1 warning** before any test ran; all were fixed before the first green gate.

## Where it went wrong

**Failure 1 — fixture SIGABRT.** *Stage:* deep-object case with a 10,000-deep `serde_json::Value`. *Symptom:* the test binary aborted — `thread 'main' panicked` was not even reached; the process died with SIGABRT in `serde_json`'s recursive `Value` drop. *Root cause:* the *fixture construction* (not the bridge) built a 10,000-deep nested `serde_json::Value`; dropping it recursed 10,000 frames and overflowed the stack. The bridge's iterative traversal never saw it — the test harness died first.

**Failure 2 — depth marker shape.** *Stage:* depth-bound case after the fixture fix. *Symptom:* assertion failed — expected the marker `__bridge_depth_exceeded_at`, got the raw tail of the value instead. *Root cause:* the iterative traversal emitted the depth marker but the depth check was off-by-one relative to where the marker was expected; the walk descended one level past the cap before planting the marker.

**Failure 3 — cycle blindness.** *Stage:* cyclic-object case. *Symptom:* no `CyclicValue` error — the cyclic fixture traversed as if acyclic. *Root cause:* the traversal tracked visited nodes by value pointer of the *walker's* re-pushed frames, not by the identity of the original JSON nodes; re-visits of the same `serde_json::Map` allocation were never recognized as cycles.

## The fix — what changed and why

**Fix 1 — 512-deep fixture.** The fixture depth went from 10,000 to `FIXTURE_DEPTH = 512` — still 16× `MAX_RESULT_DEPTH` (32), so it proves the cap with margin, but shallow enough that `serde_json::Value`'s recursive drop stays in its stack budget. The lesson is recorded in the code comment: fixture construction is itself hostile-input handling; keep fixtures inside the drop recursion budget.

**Fix 2 — exact depth planting.** `bound_value`'s iterative loop now checks `depth >= MAX_RESULT_DEPTH` before descending and plants `__bridge_depth_exceeded_at=32` as the leaf at exactly the cap. Verified by the assertion on the marker and on the output depth.

**Fix 3 — pointer-identity cycle tracking.** The traversal now records the raw pointer of each `serde_json::Map`/`Vec` it opens (in a bounded `seen` set capped alongside the depth budget) and returns `BridgeError::CyclicValue` on re-entry. The wave's cyclic-value policy is **rejection**, declared in the contract (`CYCLIC_CONTRACT = "reject"`): cycles are never serialized, never truncated — refused with a typed error. The alternative (reference-preserving serialization) was rejected: it would leak internal memory addresses into agent-visible output.

**Fix 4 (known, documented honestly) — fixture storage still clones the 50 MB canned result.** `ScriptedPort::evaluate` currently returns `Ok(result.clone())` on the matching fixture, which copies the full 50 MB allocation once before `evaluate_limited` truncates it. The truncation guarantee holds (agent-visible bytes ≤ 65,536), but the intermediate clone is wasteful and misrepresents the spirit of the bound. The meter claim is therefore scoped narrowly and honestly: it counts bytes copied into agent-visible output, not all process allocations. Eliminating the clone (move-out fixture storage) is flagged as follow-up work in the wave report.

- **Source:** CDP `Runtime.evaluate` result shapes from https://chromedevtools.github.io/devtools-protocol/; traversal design follows the Tiger Style Rust rule "bounded work, no recursion" (`~/workspace/skills/tiger-style-rust/SKILL.md`).
- **Validation agents:** all four cases green; `--lib` suite (64 tests) green; the 512-deep fixture was chosen after the 10,000-deep abort was reproduced twice.
- **Adversarial agents:** red-teamed with a 50 MB string (truncated, not OOM), 512-deep nesting (capped), a self-referential object (rejected), and a meter assertion on output bytes (bounded).

## Full technical depth

`bound_value` is an explicit-stack iterative walk over `serde_json::Value`: arrays and objects are opened via `open_array`/`open_object` helpers (extracted to keep the function under 70 lines), strings are prefix-truncated at 64 KiB, numbers/bools/null pass through, depth is capped at 32 with the `__bridge_depth_exceeded_at` marker, and re-visited composite pointers raise `CyclicValue`. `AllocationMeter` counts every byte the walk emits into agent-visible text; `evaluate_limited` asserts `bytes_copied ≤ MAX_RESULT_BYTES` (65,536) as an invariant, not just an observation.

The newline-delimited framing contract (task 179) sits below this: evaluation results cross the stdio pipe as framed JSON; the bounds here apply to the *content* of those frames. All fixtures are scripted CDP `Runtime.evaluate` payloads per the documented result shapes — no real-browser hostile page is needed because the traversal never touches the transport.

Ghostex lineage: the bounded hostile-payload concept is adapted from maddada/Ghostex @ `c91146607205ac49303d1bcfe2fd6f9a86741500`. Ghostex's CEF/GPUI implementation was not ported; this is a Tiger Style Rust re-implementation of the concept.

## Sources

- https://chromedevtools.github.io/devtools-protocol/ — official CDP docs: `Runtime.evaluate` return shapes (`type`, `value`, `unserializableValue`, object previews)
- `crates/phlow-gauntlet/src/bridge.rs` — `bound_value`, `open_array`, `open_object`, `AllocationMeter`, `evaluate_limited`, `MAX_RESULT_BYTES`, `MAX_RESULT_DEPTH`, `BridgeError::CyclicValue`
- `crates/phlow-gauntlet/src/tasks/task_181.rs` — the four driver cases (incl. `FIXTURE_DEPTH = 512` rationale)
- `crates/phlow-gauntlet/tests/task_181.rs` — the integration assertions
- maddada/Ghostex @ `c91146607205ac49303d1bcfe2fd6f9a86741500` — adapted concept (CEF/GPUI implementation not ported)
