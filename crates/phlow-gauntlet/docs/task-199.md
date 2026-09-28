> Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500. Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

# task-199: verify-after-act

**Kind:** rust · **Status:** pass · **Wave:** 32 · **Commits:** pending (wave 32)

## ELI5

When a program changes something, don't trust its word that it worked — read the thing back and check. This task's rule: a mutating command must report which fields it changed, then re-read them fresh from the source and compare. To prove the check is real, the adversarial case uses a fake backend that *claims* success but changes nothing: the verification step must catch the lie, exit with a non-zero code, and report a typed `CommandError::VerifyFailed`. A false "success" is never printed.

## What this task attempts

- **Goal:** mutating commands report changed fields plus a verification line from a fresh re-read; a fault-injected no-write backend is caught with `CommandError::VerifyFailed` and a non-zero exit.
- **Mechanism:** `crates/phlow-gauntlet/src/tasks/task_199.rs` — the small explicit state machine `execute_verified` over the `MutatingCommand` trait (`apply` + `read_state`); V uses the honest file-backed `FileStateCommand` (apply writes a state file, read_state reads it back from disk) plus an independent script-level re-read; A uses `LyingBackend` (apply returns success without writing, read_state returns the unmutated state).
- **Success criterion:** V — exit 0, `changed <field>=<value>` lines, `verified: 2 fields match fresh re-read`, script re-read agrees; A — `Err(CommandError::VerifyFailed)` naming the field and both values, exit non-zero, no success text.
- **Non-goals:** wiring this into phlow-cli's real commands (see below — deliberately not attempted).

## What happened

Both cases pass on primo. V: the honest file-backed backend applied 2 fields, printed `changed brightness=80` / `changed mode=warm` plus `verified: 2 fields match fresh re-read`, exited 0, and the independent script re-read of the state file confirmed reported state equals actual state. A: the lying backend (reported success, wrote nothing) was caught — `execute_verified` returned `CommandError::VerifyFailed` naming `brightness` (reported `80`, actual `20`), exit code 1, no success text produced. Gates: `cargo test -p phlow-gauntlet --test task_199` 2/2 pass.

## Where it went wrong

Nothing failed mechanically — the honest limitation is scope, documented here deliberately. phlow-cli's real commands have no mutating command with an injectable backend: `run`'s mutation is "ask the model and append to a trace" (no verify-after-act step, no injectable state), and `check`'s contract is report-shaped (status/verdict), not state-mutating. Bolting a generic verify-after-act framework onto phlow-cli would have changed report shapes and the Python-parity contract for a doctrine point the real CLI has no seam for. The mechanism is therefore proven at toy scale (`execute_verified`) instead of pretended against the real CLI. This is the batch's stated honest-limitation policy: test what's real, document what isn't.

## The fix — what changed and why

- **Changed:** `crates/phlow-gauntlet/src/tasks/task_199.rs` — new driver with `CommandError` (typed: `ApplyFailed`, `ReadFailed`, `VerifyFailed`), `exit_code_for` (0 on success, 1 on any failure — mirroring phlow-cli's exit-1 "command ran but outcome not ok"), the `MutatingCommand` trait, `execute_verified`, `FileStateCommand`, `LyingBackend`.
- **Why:** the trait separates the mutation path from the verification path — verification must not re-read the command's own memory, or it proves nothing. `FileStateCommand` goes through the filesystem for both, so the re-read is genuinely fresh. The lying backend diverges exactly the way the doctrine fears: success claimed, nothing written.
- **Source:** Ghostex CLI doctrine `skills/ghostex-cli/SKILL.md` (verify-after-act; `CommandError::VerifyFailed`) — concept adapted, see header. phlow-cli exit-code convention (`EXIT_COMMAND_FAILED = 1`) in `crates/phlow-cli/src/lib.rs`.
- **Validation agents:** primo gates, 2026-09-28: `cargo build -p phlow-gauntlet -p phlow-cli` clean; `cargo test -p phlow-gauntlet --test task_199` all pass; `cargo test -p phlow-gauntlet --lib` 64/64; `cargo test -p phlow-cli` 26 unit + 28 integration pass; `cargo fmt --check` clean; `cargo clippy -p phlow-gauntlet --all-targets` and `-p phlow-cli` 0 warnings.
- **Adversarial agents:** the task's own A-cases (see What happened); no separate red-team pass in this wave.

## Full technical depth

`execute_verified` applies, then re-reads via `read_state` and compares every reported `(field, value)` pair against the fresh map: a mismatched value or a missing field both produce `VerifyFailed { command, field, reported, actual }` (`<absent>` when the field is missing entirely). The success text is built only after all comparisons pass — there is no code path that prints success on a failed verification. In the adversarial case the backend reports `brightness=80, mode=warm` while the world holds `brightness=20` with no `mode`: verification fails on the first field (`brightness`: reported `80` vs actual `20`), `exit_code_for` maps the error to 1, and the driver asserts no success text was ever produced. The V case additionally re-reads the state file with an independent `read_pairs` call (not through the command), proving reported state equals actual state from a second vantage point.

## Sources

- `crates/phlow-gauntlet/src/tasks/task_199.rs`, `tests/task_199.rs`.
- `crates/phlow-cli/src/lib.rs` — exit-code conventions.
- Ghostex CLI doctrine: `skills/ghostex-cli/SKILL.md` (verify-after-act) — concept adapted, not ported; see header.
