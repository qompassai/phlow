# task-151: open enums with Other(String)

**Kind:** rust (validation) · **Status:** pass · **Wave:** 25 · **Commit:** pending (wave 25 commit)

## ELI5

Two programs talk over a socket by sending labeled messages — each message has a `kind` like `"ping"` or `"event"`. Sooner or later the other side invents a new kind your program has never heard of. The naive move is to crash ("unknown variant!"); the Ghostex rule is to catch it in a bucket called `Other(String)` that keeps the exact spelling, routes it to a safe default handler, and never confuses it with a kind you do know. This task proves the bucket works: an unknown `kind` arrives, is kept byte-identical, goes to the default handler, and 1,200 mixed known/unknown messages all parse with zero errors.

## What this task attempts

- **Goal:** unknown enum variants deserialize to `Other(String)`, round-trip byte-identical, and dispatch to the default handler without aliasing known variants.
- **Mechanism:** `crates/phlow-gauntlet/src/wire.rs` — `Kind::from_wire`, `Kind::to_json`, `dispatch`; driver `crates/phlow-gauntlet/src/tasks/task_151.rs`.
- **Success criterion:** 4/4 validation cases pass: unknown variant round-trips; 1,000 unknown + 200 known variants → zero errors with the catch-all bucket holding exactly 1,000; 8 near-miss spellings never alias the 7 known variants; the adapted module carries the maddada attribution + source commit.
- **Non-goals:** unknown *fields* (task 152), hostile variant spellings aimed at privileged handlers (task 156).

## What happened

Passed. `cargo test -p phlow-gauntlet --test task_151` → 4 passed, 0 failed. The 1,200-variant fixture parsed with zero errors; the catch-all bucket held exactly the 1,000 unknowns; `"ping_v9"` re-serialized byte-identical to its input and dispatched to `Dispatch::Default`; none of the 8 near-miss spellings (`"Ping"`, `"PING"`, `" ping"`, `"ping "`, `"ping_v9"`, `"admin_override"`, `""`, `"pong\0"`) aliased a known variant.

## The fix — what changed and why

No driver fixes were needed; the task passed on the first gate run. Two test-harness fixes during gating (shared with the wave):

- **Changed:** `tests/task_151.rs` — replaced `assert_eq!(x.as_bool().unwrap(), true)` with `assert!(x.as_bool().unwrap())` (19 sites wave-wide, plus one multi-line assert with a custom message here).
- **Why:** `cargo clippy --all-targets -- -D warnings` denies `bool_assert_comparison`. The assertion is identical; only the macro form changed.
- **Source:** clippy `bool_assert_comparison` lint documentation.
- **Validation:** `cargo clippy -p phlow-gauntlet --all-targets -- -D warnings` → 0 errors, 0 warnings; tests re-run green.
- **Adversarial:** none — mechanical lint fix, no behavior change.

## Full technical depth

The wire `Kind` is an open enum: `Ping | Pong | Event | Subscribe | Unsubscribe | Other(String)`. `from_wire` matches the seven known spellings exactly and wraps anything else in `Other` with the payload verbatim — no case-folding, no trimming, no Unicode normalization (that strictness is what task 156 attacks). `dispatch` maps each known variant to its handler arm and `Other(raw)` to `Dispatch::Default { raw }`, so an unknown variant can never reach a privileged arm by construction: the match has no fallthrough from `Other` to a known arm. The round-trip property is structural — `to_json(Other(s))` emits the string `s` unchanged, so re-serialization is byte-identical to the input. The 1,200-variant case exists because the interesting failure is partial: a decoder that handles unknowns but drops or miscounts them at volume. The near-miss list covers the classic aliasing attacks on exact-match dispatch: case variants, whitespace padding, a name containing a known name as a substring (`ping_v9`), an empty string, and an embedded NUL.

## Sources

- Ghostex open-enum rule: `packages/gx-protocol/src/open_enum.rs` @ c91146607205ac49303d1bcfe2fd6f9a86741500 (adaptation map `~/workspace/ghostex-recon/adaptation-map.md`).
- `crates/phlow-gauntlet/src/wire.rs` — `Kind`, `dispatch`.
- `crates/phlow-gauntlet/src/tasks/task_151.rs`, `crates/phlow-gauntlet/tests/task_151.rs`.
