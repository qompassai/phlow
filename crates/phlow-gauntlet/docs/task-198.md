> Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500. Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

# task-198: JSON is a contract

**Kind:** rust · **Status:** pass · **Wave:** 32 · **Commits:** pending (wave 32)

## ELI5

When a program prints JSON for other programs to read, that JSON is a promise: same fields every time, same ids every time, valid every time. This task runs `phlow status --json` twice and checks both outputs parse and have identical field names; checks that the same checks keep the same names across runs; and saves a "schema snapshot" (just the field names and types, not the values) for `status`, `check`, and `run` so that renaming a field or dropping a key breaks the build instead of breaking someone's script.

## What this task attempts

- **Goal:** `--json` output is valid JSON, has stable field names, and stable entity ids; schema snapshots exist and pass for every tested report command.
- **Mechanism:** `crates/phlow-gauntlet/src/tasks/task_198.rs` spawns the real `phlow` binary; V1 runs `status --json` twice (field-path sets compared) plus `tui --json` (must be a usage error, exit 2); V2 runs `status --json` twice against a seeded config (check names compared); V3 computes the type-only schema of `status`/`check`/`run --json` output and compares against `crates/phlow-gauntlet/tests/data/wave32/schema-{status,check,run}.json`.
- **Success criterion:** both runs parse with identical field-name sets; entity ids identical; `tui --json` exits 2; all three schemas match their snapshots.
- **Non-goals:** value stability (timestamps/durations legitimately vary); `serve` (MCP frames, not one-shot JSON).

## What happened

All three cases pass on primo. V1: `status --json` twice — both parse, identical field-name sets; `--json` before the subcommand behaves the same; `tui --json` is a usage error (exit 2) with the refusal on stderr. V2: entity ids `[alpha, beta]` identical across runs. V3: all three schemas (`status`, `check`, `run`) match their checked-in snapshots. Gates: `cargo test -p phlow-gauntlet --test task_198` 3/3 pass.

The required phlow-cli behavior (explicit `--json` flag; refusal for commands that cannot honor it) did not exist and was implemented minimally in `crates/phlow-cli/src/cli.rs` (`--json` global flag) and `lib.rs` (`--version --json` → JSON object; `tui --json` → usage error, exit 2). Snapshot files were generated from real primo output under the test's own conditions (temp dir + seed config) and committed with the wave.

## The fix — what changed and why

- **Changed:** `crates/phlow-cli/src/cli.rs` — added `#[arg(long, global = true)] pub json: bool`.
- **Changed:** `crates/phlow-cli/src/lib.rs` — `dispatch` rejects `--json` with `tui`/no subcommand as a usage error (exit 2); `--version --json` prints `{"version": ...}` instead of the human string.
- **Why:** the doctrine's `--json` requirement had no real counterpart: the CLI printed JSON by default but offered no flag to *assert* the contract and no way to refuse it where machine output is impossible (the TUI). A silent `--json` on the TUI would print a human interface while promising JSON — the refusal is the honest behavior.
- **Source:** existing global-flag convention in `cli.rs` (Python parity: flags accepted before or after the subcommand); Ghostex `--json` doctrine — concept adapted, see header.
- **Validation agents:** primo gates, 2026-09-28: `cargo build -p phlow-gauntlet -p phlow-cli` clean; `cargo test -p phlow-gauntlet --test task_198` all pass; `cargo test -p phlow-gauntlet --lib` 64/64; `cargo test -p phlow-cli` 26 unit + 28 integration pass; `cargo fmt --check` clean; `cargo clippy -p phlow-gauntlet --all-targets` and `-p phlow-cli` 0 warnings.
- **Adversarial agents:** the task's own A-cases (see What happened); no separate red-team pass in this wave.

## Full technical depth

`--json` is global like the other flags, so `phlow --json status` and `phlow status --json` behave identically (V1 pins the parity). Report commands already print one JSON line via `python_json_dumps` (ASCII-only, control characters escaped), so `--json` asserts an existing contract rather than changing output. The field-path collector (`field_paths` in `cli_harness.rs`) walks objects into dotted paths and arrays into `[]` markers, so a renamed nested field changes the set. Schema snapshots keep only types (`{"type": "object", "fields": {...}}`), making them immune to value noise (durations, timestamps) while catching structural drift. The `run` snapshot uses the no-backend error report: without Ollama the model call fails fast, but the report envelope (`status`, `model`, `workspace`, `actions`, `summary`, `trace`) has the same stable shape as a success — verified on primo during snapshot generation.

## Sources

- `crates/phlow-cli/src/cli.rs` (`--json` flag), `src/lib.rs` (`dispatch` refusal).
- `crates/phlow-gauntlet/src/tasks/task_198.rs`, `tests/task_198.rs`, `tests/data/wave32/schema-*.json`.
- Ghostex CLI doctrine: `skills/ghostex-cli/SKILL.md` (machine-readable output) — concept adapted, not ported; see header.
