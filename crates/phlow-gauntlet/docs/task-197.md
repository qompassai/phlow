> Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500. Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

# task-197: help-first commands

**Kind:** rust · **Status:** pass · **Wave:** 32 · **Commits:** pending (wave 32)

## ELI5

Every `phlow` subcommand must introduce itself. Run `phlow check --help` and you should get a usage block that names the `check` command — not a crash, not a wall of unrelated text. This task sweeps all subcommands the binary advertises, and separately scans the CLI source code so that any *future* subcommand without help text fails the build before it ships.

## What this task attempts

- **Goal:** every `phlow` subcommand answers `<cmd> --help` with exit 0 and a usage block containing the command name.
- **Mechanism:** `crates/phlow-gauntlet/src/tasks/task_197.rs` spawns the real `phlow` binary (subprocess), parses `phlow --help`'s `Commands:` section, and runs `<cmd> --help` for each; V2 statically scans `crates/phlow-cli/src/cli.rs` for the `Commands` enum and the build-failing gate `every_subcommand_has_help_string`.
- **Success criterion:** V1 — all enumerated subcommands exit 0 with `Usage:` naming the command; V2 — every `Commands` variant has a doc comment and the gate exists in the crate.
- **Non-goals:** help *quality* (wording, examples); the auto-generated `help` subcommand's content beyond the same byte contract.

## What happened

Both cases pass on primo. V1 enumerated 6 subcommands from `phlow --help` (run, serve, check, status, tui, plus clap's auto-generated `help`); all answer their help invocation with exit 0 and a `Usage:` block naming the command. V2's static scan found all 5 `Commands` variants with doc comments and pinned the build gate `every_subcommand_has_help_string`. Gates: `cargo test -p phlow-gauntlet --test task_197` 2/2 pass; `cargo test -p phlow-cli` 26/26 unit tests pass (including the new help gates).

## The fix — what changed and why

- **Changed:** `crates/phlow-cli/src/cli.rs` — added the `every_subcommand_has_help_string` unit test (iterates `Cli::command().get_subcommands()`, asserts each has `about`/`long_about`).
- **Why:** the doctrine's "static scan must fail the build" requirement cannot be met by a gauntlet driver alone (drivers report; they don't gate). The test lives where the violation can happen, so a new variant without a doc comment fails `cargo test -p phlow-cli` immediately.
- **Source:** clap derive: a `///` doc comment on a `Commands` variant becomes that subcommand's help text (clap 4 derive reference).
- **Validation agents:** primo gates, 2026-09-28: `cargo build -p phlow-gauntlet -p phlow-cli` clean; `cargo test -p phlow-gauntlet --test task_197` all pass; `cargo test -p phlow-gauntlet --lib` 64/64; `cargo test -p phlow-cli` 26 unit + 28 integration pass; `cargo fmt --check` clean; `cargo clippy -p phlow-gauntlet --all-targets` and `-p phlow-cli` 0 warnings.
- **Adversarial agents:** the task's own A-cases (see What happened); no separate red-team pass in this wave.

## Full technical depth

The `Commands` enum in `cli.rs` carries one Rust doc comment per variant; clap's derive turns each into the subcommand's `about`. The unit gate walks the built `clap::Command` tree (`Cli::command().get_subcommands()`) and asserts non-empty help text — it cannot drift from the real definition because it reads the same derive output the binary ships. The driver V1 enumerates dynamically (whatever `phlow --help` lists, including clap's auto-generated `help` subcommand, which is tested like any other), so a new subcommand is covered without editing the task. V2's source scan parses the enum body textually (variants sit at exactly one indent level; fields sit deeper) and checks the preceding non-blank, non-attribute line is a `///` comment — plus it pins the gate's presence so the scan is a gate, not advice.

## Sources

- `crates/phlow-cli/src/cli.rs` — `Commands` enum, `every_subcommand_has_help_string` test.
- `crates/phlow-gauntlet/src/tasks/task_197.rs`, `tests/task_197.rs`.
- clap 4 derive: doc comments → help text (upstream docs).
- Ghostex CLI doctrine: `skills/ghostex-cli/SKILL.md` (help-first commands) — concept adapted, not ported; see header.
