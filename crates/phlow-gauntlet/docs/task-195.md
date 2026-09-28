# task-195: traversal refused, symlinks not followed, exec bits stripped

**Kind:** rust (adversarial) · **Status:** pass · **Wave:** 31 · **Commit:** pending (`gauntlet: wave 31 drivers, tests, docs`)

## ELI5

A sync tool that copies files between directories is a classic attack surface: what if a filename contains `../..` and escapes the target folder? What if a "file" is actually a symlink pointing at your password file — would the sync follow it and copy your secrets, or overwrite the file it points to? And what if the source file is executable — should the copy stay executable? This task attacks all three: the traversal must be refused with a typed error and nothing written outside the target; the symlink must be refused, never followed; and synced files must land non-executable (mode `0o644`) with identical content.

## What this task attempts

- **Goal:** verify the three hostile-input behaviors: `../` escape refused with `SyncError::Traversal` and zero writes outside the target; symlink in canonical refused with `SyncError::SymlinkRefused`, scan never following it; `0o755` source file lands at exactly `0o644` with byte-identical content.
- **Mechanism:** `crates/phlow-gauntlet/src/skill_sync.rs` (`contained_path`, `validate_op`, `write_file_atomic`, `scan`) driven by `crates/phlow-gauntlet/src/tasks/task_195.rs`; assertions in `crates/phlow-gauntlet/tests/task_195.rs`.
- **Success criterion:** A1 (`traversal_refused`): hand-built plan with rel `../evil.md` → `apply` returns `Err(SyncError::Traversal{..})`, nothing written outside the target, marker absent. A2 (`symlink_not_followed`): canonical symlink → `sync` returns `Err(SyncError::SymlinkRefused{..})`, link target bytes never copied anywhere. A3 (`exec_bit_stripped`): `0o755` source → target mode exactly `0o644`, content byte-identical.
- **Non-goals:** the benign plan/apply path (193), idempotence (194), concurrency/crash (196).

## What happened

PASS, all 3 scenarios, on the final tree:

- **A1:** `apply` → `Err(SyncError::Traversal { rel: "../evil.md" })`; the sibling `outside/` dir contains no `evil.md`; no marker left behind.
- **A2:** `sync` → `Err(SyncError::SymlinkRefused { rel: "link.md" })`; the secret bytes behind the link appear in no target file.
- **A3:** target `runme.sh` mode `644` (permission bits), content byte-identical to the `0o755` source.

`cargo test -p phlow-gauntlet --test task_195`: `3 passed; 0 failed`.

## Where it went wrong

- **Stage:** `cargo test --test task_195`, A3 `exec_bit_stripped`, first test run.
- **Symptom:** `evidence must show the stripped mode:\ntarget runme.sh mode: 100644`.
- **Evidence:** the driver pushed `format!("target runme.sh mode: {mode:o}")` where `mode` came from `PermissionsExt::mode()`; the test asserts the evidence contains `"mode: 644"`.
- **Root cause:** `mode()` returns the full `st_mode`, including the file-type bits (`0o100000` for a regular file) — so it printed `100644`. The driver's own assertions (`mode & 0o111 == 0`, `mode & 0o777 == 0o644`) were already masking correctly and passing; only the human-readable evidence string was wrong. A reporting bug, not a permission bug.

## The fix — what changed and why

- **Changed:** `crates/phlow-gauntlet/src/tasks/task_195.rs` — the evidence line now prints `mode & 0o777` (`format!("target runme.sh mode: {:o}", mode & 0o777)`), yielding `mode: 644`.
- **Why:** evidence strings are part of the contract the tests assert on; printing raw `st_mode` is misleading even when the masked assertions are right. Masking at the print site matches the assertions two lines above. **Source:** `std::os::unix::fs::PermissionsExt::mode` documentation (mode includes file-type bits; permission bits are `mode & 0o777`).
- **Commit:** pending (wave 31).
- **Validation agents:** the wave-31 worker — reran the focused suite on primo, 3/3 pass, no other task affected.
- **Adversarial agents:** the task is itself adversarial; the red-team reasoning is in "Full technical depth". No separate adversarial agent was assigned.
- **Citations:** `PermissionsExt::mode` (std docs); `contained_path` in `skill_sync.rs` for the traversal component check.

## Full technical depth

The traversal defense is lexical, not canonicalizing: `contained_path` rejects empty rels, absolute paths, and any `ParentDir`/`RootDir`/`Prefix` component *before* joining — canonicalization is deliberately avoided because the target path may not exist yet and because canonicalizing would resolve symlinks, which is exactly what must not happen. The symlink defense is layered three deep: `scan` uses `symlink_metadata` (never follows) and indexes links with `is_symlink: true`; `validate_op` refuses any op whose canonical entry is a symlink *and* any op whose target path is currently a symlink (TOCTOU between plan and apply — an attacker swapping a file for a link mid-sync gets a refusal, not a followed link); `execute_op` re-checks the canonical source with `symlink_metadata` right before reading. A1's hand-built plan is important: it bypasses `build_plan` entirely, proving the refusal lives in `apply`'s validation, not in plan construction — a plan built by a compromised or buggy planner still can't escape the root. The exec-bit stripping happens in `write_file_atomic`: permissions are set on the temp file *before* the rename, so there's no window where an executable file exists at the destination. Note the refusal is fail-closed but not fail-silent: typed errors (`Traversal`, `SymlinkRefused`) name the offending rel, which is what the tests assert on.

## Sources

- `crates/phlow-gauntlet/src/skill_sync.rs` — `contained_path`, `validate_op`, `execute_op`, `write_file_atomic`, `scan` (primary source for every mechanism claim)
- `crates/phlow-gauntlet/src/tasks/task_195.rs`, `crates/phlow-gauntlet/tests/task_195.rs`
- `~/workspace/scratch/ghostex/packages/agent-sync/src/apply.rs` @ `c91146607205ac49303d1bcfe2fd6f9a86741500` — the Ghostex concept source for traversal/symlink caution; the checks here are independent implementations
- https://doc.rust-lang.org/std/os/unix/fs/trait.PermissionsExt.html — `mode()` includes file-type bits
- `~/workspace/gauntlet-design-tasks-151-200.md` — Wave 31 design (lines 943–1030)
