# task-157: version-skew games

**Kind:** rust (adversarial) · **Status:** pass · **Wave:** 25 · **Commit:** pending (wave 25 commit)

## ELI5

Version numbers come from the peer — which means they come from the attacker. The classic attack is the *downgrade*: negotiate the newest, most secure protocol version, then slip in an old-version message hoping the receiver switches to the older, weaker parser. Another is the absurd version: `4294967295` (2³²−1), hoping the receiver uses the version as an index into a table and crashes — or worse, reads attacker's memory. This task proves both fail closed: a v1 message after v3 negotiation is rejected as `Downgrade`, the session keeps its v3 mark and keeps working, and the absurd version is rejected as `Unsupported` before any parser is ever consulted — the version check is an if-chain, never `parsers[version]`.

## What this task attempts

- **Goal:** downgrade replays and absurd versions fail closed; per-session version is a monotonic high-water mark.
- **Mechanism:** `crates/phlow-gauntlet/src/wire.rs` — `Session::negotiate`/`accept`/`negotiated`/`accepted`, `route_version`, `VersionError::Downgrade`; driver `crates/phlow-gauntlet/src/tasks/task_157.rs`.
- **Success criterion:** v1 after v3 → `Downgrade{got:1, session:3}`, session keeps v3 and stays usable, the downgrade doesn't count as accepted; 2³²−1 → `Unsupported{max:3}` pre-dispatch, 0 → `TooOld`, session state untouched; 1→2→3 then replayed 2 → `Downgrade`, mark stays 3.
- **Non-goals:** benign version routing (task 154); version handling on the editor socket (same code path, covered by task 158's parity).

## What happened

Passed. `cargo test -p phlow-gauntlet --test task_157` → 4 passed, 0 failed. The downgrade was typed and the session intact (mark v3, accepted count unchanged, still accepting v3 afterwards); the absurd version and version 0 were typed without touching session state; the monotonic walk 1→2→3 held and the replayed 2 was rejected as a downgrade.

## The fix — what changed and why

No fixes; the task passed on the first gate run.

## Full technical depth

The mental model is TUF-style version monotonicity: the negotiated version is a high-water mark that never moves down. `Session::accept` checks the incoming version against the mark *before* any dispatch: below the mark → `Downgrade` (never a silent parser switch to the weaker version); above `MAX_SUPPORTED_VERSION` → `Unsupported`; below the minimum → `TooOld`. Because `route_version` is an if-chain over ranges rather than a table indexed by the version, there is no index for 4,294,967,295 to corrupt — the absurd version dies in a comparison, with no parser consulted and no session state mutated (the test asserts `negotiated == 3, accepted == 0` after both rejections, proving the failures are side-effect-free). Rejected versions don't increment `accepted`, so the counter is a true record of envelopes the session actually honored. The downgrade case also proves the session remains usable afterwards — a refusal that wedged the session would be a denial-of-service vector in its own right.

## Sources

- Ghostex versioned-envelope rule (`rpc.rs` `protocolVersion` gate) @ c91146607205ac49303d1bcfe2fd6f9a86741500 (adaptation map); TUF-style monotonic versioning (secondary: the Update Framework spec's rollback-attack protection, cited as the mental model in the driver's module docs).
- `crates/phlow-gauntlet/src/wire.rs` — `Session`, `route_version`, `VersionError`.
- `crates/phlow-gauntlet/src/tasks/task_157.rs`, `crates/phlow-gauntlet/tests/task_157.rs`.
