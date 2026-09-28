# task-156: variant-confusion refusal

**Kind:** rust (adversarial) · **Status:** pass · **Wave:** 25 · **Commit:** pending (wave 25 commit)

## ELI5

Task 151 built the `Other(String)` bucket for unknown message kinds. Now the attacker tries to abuse it: they craft a kind name that *looks* privileged — `"admin_override"`, or `"Ping"` with a capital P hoping your code lowercases it into `"ping"`, or `"subscribe"` with a sneaky invisible space, or even a lookalike letter from another alphabet. If dispatch normalizes names before matching, one of these slips into a privileged handler. This task proves it can't: all seven hostile spellings land in `Other(..)` with their payloads byte-identical (never case-folded, trimmed, or Unicode-normalized), all route to the default handler, and the privileged `Subscribe` arm fires zero times — while the exact `"subscribe"` spelling still reaches it, proving the arm exists and is simply unreachable by forgery.

## What this task attempts

- **Goal:** hostile kind spellings cannot reach privileged dispatch arms; `Other` payloads are never normalized.
- **Mechanism:** `crates/phlow-gauntlet/src/wire.rs` — `Kind::from_wire`, `Kind::as_str`, `dispatch`, `Dispatch::Subscribe` (the privileged arm: it mutates daemon state); driver `crates/phlow-gauntlet/src/tasks/task_156.rs`.
- **Success criterion:** 7 hostile spellings → all `Other(..)`, all `Dispatch::Default`, 0 `Subscribe` dispatches; payloads byte-identical through `from_wire`/`as_str`; exact `"subscribe"` → `Subscribe` (control).
- **Non-goals:** benign unknowns (task 151); hostile *frames* (task 155).

## What happened

Passed. `cargo test -p phlow-gauntlet --test task_156` → 4 passed, 0 failed. All seven hostile spellings (`"admin_override"`, `"Ping"`, `"PING"`, `" subscribe"`, `"subscribe "`, `"pi\0ng"`, `"pіng"` with Cyrillic і U+0456 — verified in the source bytes) were captured as `Other` with payloads byte-identical, dispatched to the default handler, and produced zero privileged dispatches; the exact `"subscribe"` reached `Subscribe`.

## The fix — what changed and why

No fixes; the task passed on the first gate run. (The Cyrillic confusable was verified at the byte level — `0x70 0x456 0x6e 0x67` — because a confusable that isn't actually confusable proves nothing.)

## Full technical depth

The security property is *exact-match dispatch with a non-normalizing catch-all*. `from_wire` compares the raw `&str` against the known spellings with `==` — no `to_lowercase`, no `trim`, no Unicode normalization — so `"Ping"`, `" subscribe"`, and `"pіng"` (Cyrillic і) are all distinct from their lookalikes and fall into `Other` carrying the original bytes. `dispatch` then matches `Other(raw)` to `Dispatch::Default { raw }`; there is no path from `Other` to any known arm, so variant confusion is impossible by construction rather than by denylist (a denylist of hostile spellings would be infinite — Unicode alone defeats it). The control case matters: `"subscribe"` → `Subscribe` proves the privileged arm is live and the test isn't vacuously passing because dispatch is broken. The embedded-NUL case (`"pi\0ng"`) covers C-string-style truncation attacks where a downstream consumer might stop at the NUL and see `"pi"` — here the payload is kept whole and default-routed, so no truncation happens at the dispatch layer.

## Sources

- Ghostex open-enum rule: `packages/gx-protocol/src/open_enum.rs` @ c91146607205ac49303d1bcfe2fd6f9a86741500 (adaptation map).
- `crates/phlow-gauntlet/src/wire.rs` — `Kind`, `dispatch`, `Dispatch`.
- `crates/phlow-gauntlet/src/tasks/task_156.rs`, `crates/phlow-gauntlet/tests/task_156.rs`.
