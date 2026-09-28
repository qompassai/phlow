# task-158: editor-socket parity and license audit

**Kind:** rust (validation + adversarial) · **Status:** pass → fixed · **Wave:** 25 · **Commit:** pending (wave 25 commit)

## ELI5

The Neovim editor talks to the daemon over its own socket — and it would be easy to give that socket a second, lazier parser ("it's just the editor"). Attackers love second parsers: every difference is a seam. This task proves there is no second parser: the editor socket uses the exact same envelope discipline as the daemon link — a malformed message is a typed error, the socket stays up, the nvim peer is undisturbed, and a 10,000-deep nesting bomb is rejected at the same depth bound. Plus the wave's license gate: every Rust file added in tasks 151–157 is scanned for the maddada attribution and the Ghostex source commit as its first three lines — missing attribution fails the task.

## What this task attempts

- **Goal:** the editor socket enforces the same wire discipline as the daemon link; all 15 wave-25 `.rs` files carry the exact attribution header.
- **Mechanism:** `crates/phlow-gauntlet/src/wire.rs` — `EditorSocket::read` (same `parse_envelope` path); driver `crates/phlow-gauntlet/src/tasks/task_158.rs` (`check_attribution`, shared with tasks 151–157 license cases); `AUDIT_FILES` list.
- **Success criterion:** malformed editor message → `SocketError::Frame(Malformed)`, socket up, 1 rejection counted, next good message reads; 10,000-deep bomb → `DepthExceeded{bound:128}` over the socket, socket up, identical typing from the raw parser (parity); attribution present-and-first on all 15 files plus task-158's own two.
- **Non-goals:** daemon-link hostile parsing (task 155 — this task asserts *parity* with it, not a second implementation).

## What happened

Passed after one fixture fix. `cargo test -p phlow-gauntlet --test task_158` → 3 passed, 0 failed. The malformed message was typed, the socket stayed up with `rejected == 1`, the nvim peer was undisturbed, and the next message read cleanly (`received == 1`). The nesting bomb was rejected at bound 128 over the socket with the raw parser agreeing exactly. The license audit checked all 15 files — attribution present and first on every one, including task-158's own driver and test.

## Where it went wrong

- **Stage:** V1 socket case, first gate run.
- **Symptom:** `malformed_socket_stays_up` failed (driver reported failure; the empty `failures` rendering in the test output initially hid the cause).
- **Evidence:** the fixture was `b"not json{{{"`.
- **Root cause:** the fixture was misclassified *by the test author*, not the parser. `parse_envelope`'s error mapping uses the depth pre-scan's state: brackets left open (`{{{`, `final_depth == 3`) means "cut off mid-frame" → `FrameError::Truncated`, which is the correct, documented behavior. The fixture was ambiguous — it genuinely looks truncated — so expecting `Malformed` was wrong.

## The fix — what changed and why

- **Changed:** `src/tasks/task_158.rs` — fixture changed from `b"not json{{{"` to `b"not json at all"` (no brackets, not in a string → unambiguously `Malformed`); clippy `repeat().take()` → `repeat_n` (×2) in the bomb builder.
- **Commit:** pending (wave 25 commit).
- **Why:** a malformed-input test must use input that is *unambiguously* malformed under the parser's documented error taxonomy; the old fixture sat exactly on the truncation/malformed boundary the parser is designed to distinguish. The parser was right; the test was wrong.
- **Source:** `parse_envelope`'s error-mapping code (`src/wire.rs`: `if scan.in_string || scan.final_depth > 0 { Truncated } else { Malformed(..) }`).
- **Validation agents:** `cargo test -p phlow-gauntlet --test task_158` → 3/3; the `Truncated` path itself is covered by task 155's `truncated_and_encoding_typed`.
- **Adversarial agents:** the parity assertion (`parse_envelope(&bomb)` types identically to `socket.read(&bomb)`) is the anti-second-parser check — any future divergence between the socket path and the raw parser breaks this test.

## Full technical depth

`EditorSocket::read` is a thin wrapper: `parse_envelope` → `route_version` → count. There is no separate editor grammar, no lenient mode, no fallback — the parity case proves it by feeding the same 10,000-deep bomb through both `socket.read` and the raw `parse_envelope` and asserting identical `DepthExceeded` typing with the same bound. The `alive` flag has no `false` transition on the read path, so "the socket stays up" is structural, not a test-time accident; `rejected`/`received` counters make refusal observable. The license audit (`check_attribution`) reads each file in `AUDIT_FILES` — the shared `src/wire.rs`, the seven task drivers, and the seven integration tests — and requires the first three lines to equal `ATTRIBUTION_LINES` exactly (copyright, `maddada/Ghostex @ c911466…` source commit, Tiger-Style re-implementation note). It is exported for reuse: every task 151–157 license case calls it, so the attribution rule is enforced in 8 places from one implementation. The audit also covers task-158's own files — the auditor is audited.

## Sources

- Ghostex adaptation: `packages/gx-protocol/src/de.rs`, `rpc.rs`, `event.rs` @ c91146607205ac49303d1bcfe2fd6f9a86741500 (adaptation map `~/workspace/ghostex-recon/adaptation-map.md`).
- `crates/phlow-gauntlet/src/wire.rs` — `EditorSocket`, `parse_envelope`.
- `crates/phlow-gauntlet/src/tasks/task_158.rs` (`check_attribution`, `AUDIT_FILES`), `crates/phlow-gauntlet/tests/task_158.rs`.
