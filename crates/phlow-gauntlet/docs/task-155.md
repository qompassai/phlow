# task-155: hostile frame rejection

**Kind:** rust (adversarial) · **Status:** pass · **Wave:** 25 · **Commit:** pending (wave 25 commit)

## ELI5

The parser sits on the boundary between your program and an attacker. The attacker sends the nastiest bytes imaginable: a message nested 10,000 layers deep (to blow the call stack), a 100-megabyte "message" (to eat all memory), a message cut off halfway, bytes that aren't even valid text. The parser must reject every one of these with a clear, typed error — never crash, never hang, never allocate a mountain of memory first and ask questions later. This task feeds all five hostile fixtures through the read path with a panic trap armed: zero panics, five typed rejections, the connection stays up, and a counting allocator proves the read path peaks at a few KiB even when the input is 100 MB.

## What this task attempts

- **Goal:** hostile frames are rejected pre-buffering with typed errors, zero panics, bounded allocation; the connection survives.
- **Mechanism:** `crates/phlow-gauntlet/src/wire.rs` — `parse_envelope` (size → UTF-8 → iterative depth pre-scan → parse), `FrameError`, `arm_panic_trap`/`panic_trapped`, `EditorSocket::read`; driver `crates/phlow-gauntlet/src/tasks/task_155.rs` (`nesting_bomb`, `oversize_frame`, `truncated_frame`, `feed_hostile`); integration test with a counting global allocator.
- **Success criterion:** 10,000-deep bomb → `DepthExceeded{bound:128}` while a 32-deep envelope parses; 100 MB frame → `TooLarge` *before* UTF-8 validation; truncated → `Truncated`, bad UTF-8 → `Encoding`, empty → `Malformed`; 5 fixtures → 0 panics, 5 rejections, socket up and usable; allocator peak ≤ 8 KiB for the hostile read loop.
- **Non-goals:** merely-imperfect input (task 153); the editor-socket instance of the same rule (task 158).

## What happened

Passed. `cargo test -p phlow-gauntlet --test task_155` → 6 passed, 0 failed. The 10,000-deep bomb was rejected at the depth bound (bound reported as 128, depth reported above it); the 100 MB frame was rejected as `TooLarge` on the 1 MiB bound before UTF-8 validation (proven: the fixture's first byte is invalid UTF-8, and the error is `TooLarge`, not `Encoding`); truncation/encoding/empty were each distinctly typed; the panic trap recorded zero panics; the socket stayed up and read a good envelope afterwards. The allocator case measured the hostile read loop peaking within 8 KiB — the 100 MB frame never got buffered.

## The fix — what changed and why

No behavior fixes; two correctness fixes to the driver's own test logic during development:

- **Changed:** `src/tasks/task_155.rs` — the "legitimate depth" control was a bare 32-deep JSON array, which `parse_envelope` correctly rejects as `Malformed` (not an envelope); rebuilt as a real envelope with a 32-deep body array.
- **Why:** the control must prove the bound rejects bombs without rejecting legitimate depth; a bare array tests the wrong property (shape, not depth).
- **Changed:** clippy fixes — `std::iter::repeat(b'[').take(n)` → `repeat_n` (×2), removed a redundant `as u32` cast (`MAX_NESTING` is already `u32`).
- **Source:** clippy `manual_repeat_n`, `unnecessary_cast`.
- **Validation:** full gate sequence green.

## Full technical depth

The defense is ordered, and the order is the point. `parse_envelope` runs four gates: (1) size — `bytes.len() > MAX_FRAME_BYTES` (1 MiB) dies *before* any UTF-8 validation or buffering, which is why the 100 MB fixture (invalid UTF-8 first byte) still reports `TooLarge`; (2) UTF-8 — `Encoding`; (3) iterative depth pre-scan — a single linear pass tracking bracket depth and string state, so a 10,000-deep bomb is rejected with `DepthExceeded` before serde_json's recursive descent ever sees it (no stack overflow possible: the scan uses a loop, and serde never runs); (4) parse — on failure the pre-scan's `in_string`/`final_depth` state distinguishes `Truncated` (cut off mid-string or brackets left open) from `Malformed`. The 8 KiB allocator bound is measured around the socket read loop with fixtures built *outside* the measured region, so the number is the parser's own doing: hostile input is classified and dropped without retention. The panic trap (`arm_panic_trap` installing a hook that records rather than suppresses) is armed for the whole hostile feed — zero trapped panics is the pass criterion, and the socket's counters (`rejected == 5`, `is_alive()`) plus a post-attack good read prove the connection is unaffected.

## Sources

- `crates/phlow-gauntlet/src/wire.rs` — `parse_envelope`, `scan_depth`, `FrameError`, `MAX_FRAME_BYTES`, `MAX_NESTING`, panic-trap helpers.
- `crates/phlow-gauntlet/src/tasks/task_155.rs`, `crates/phlow-gauntlet/tests/task_155.rs` (counting allocator).
