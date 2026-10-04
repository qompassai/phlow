# Architectural Decisions — phlow

## Rust port on main (2026-09-26/28)

**Decision**: `main` is the Rust port branch. Binary renamed flow→phlow
(`crates/phlow-cli/src/bin/flow.rs` kept as compat alias).

**Context**: Matt ordered the completion program: integrate CLM + tuios
concepts in Tiger Style, iterate until complete.

**Consequence**: Product-wide rename. Python deletion authorized in
principle; staged execution with per-step auth.

## Execution model (2026-09-27)

**Decision**: ALL cargo runs on primo. VM edits in `~/workspace/repos/phlow`,
rsync to primo. Primo's LIVE diver config used as-is for Neovim validation,
never modified.

**Context**: Sandbox lacks the toolchain and Matt's live config.

## Push authorization (2026-09-28, standing)

**Decision**: Async push to qompassai/phlow `main` when exact tree is
build-green + tests-passing on primo.

**Context**: Validated-functional program with strict gates.

**Consequence**: GitHub Git Data API only. Verify fresh-clone buildability
(worktree check) before pushing. `push_phlow_stack.py` needs full 40-char
base SHA. `git bundle create` fails on primo's phlow — read objects via
primo-ssh, drive API from sandbox.

## Defense in depth (2026-09-27)

**Decision**: Mirror Python's `DISPATCH_GATE` 4-action vs `schemas()`
`["status"]` mismatch as defense-in-depth in the Rust port.

**Context**: Caught by probing real Python for contract parity, not
code-reading. Lesson: probe, don't read.
