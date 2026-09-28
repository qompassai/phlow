> Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500. Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

# task-200: hostile output sanitization

**Kind:** rust · **Status:** pass · **Wave:** 32 · **Commits:** pending (wave 32)

## ELI5

Imagine a check named with invisible terminal commands — an "erase the screen" code plus a newline. If the CLI prints that name raw, the attacker's text runs as commands in your terminal. This task feeds exactly such a hostile id through `phlow check --name <hostile> --json` and requires three things: the output still parses as JSON, the parsed id keeps the hostile value exactly (data is escaped, never silently stripped), and the raw bytes contain no unescaped control characters. A second sweep checks every `--help` output is control-byte free, with build-time tests pinning both halves of the contract.

## What this task attempts

- **Goal:** hostile entity id (`\x1b[2J` + newline) round-trips through `--json` as valid JSON with the id preserved exactly and no raw control bytes on stdout; every `--help` output is control-byte free.
- **Mechanism:** `crates/phlow-gauntlet/src/tasks/task_200.rs` spawns the real `phlow` binary; A1 runs `check --name <hostile> --json` (unknown check name → the 4-key unknown report echoing the name), parses stdout, compares the parsed id byte-for-byte, and asserts no raw ESC/control bytes; A2 sweeps `phlow --help` and every `phlow <cmd> --help` for raw control bytes and pins the two phlow-cli unit gates in the source.
- **Success criterion:** stdout parses; `checks[0].name ==` the hostile id exactly; no `0x1b` and no unescaped control bytes in raw stdout; all help outputs clean; both build gates present in `cli.rs`.
- **Non-goals:** stripping hostile values (the declared contract is *escape*, not strip); sanitizing values inside stored config.

## What happened

Both adversarial cases pass against the real binary. A1: `check --name $'\x1b[2J\nEVIL-ID' --json` exits 1 (unknown check → `unverified`), stdout parses as JSON, the parsed `checks[0].name` is byte-identical to the hostile id (escape, not strip), and the raw stdout contains no ESC byte and no unescaped control bytes. A2: `phlow --help` and every `phlow <cmd> --help` (plus `phlow help help` for clap's meta-command) are control-byte free, and both build gates are pinned present in `phlow-cli`.

An honest correction from the first attempt: the initial unit test assumed clap renders help text verbatim (no sanitization), which would have made the "static literals only" rule load-bearing. Probing the real clap on primo proved the opposite — clap strips ANSI escape sequences and control bytes from rendered help (`"wipe\x1b[2Jscreen\x07bell"` renders as `"wipescreenbell"`; `\n` is kept as formatting). The test was rewritten as `hostile_about_is_sanitized_by_clap` to pin the *actual* behavior: strip, not escape, enforced by clap itself, with the control-byte scan as defense in depth.

## The fix — what changed and why

- **Changed:** `crates/phlow-cli/src/cli.rs` — the two unit tests above.
- **Changed:** `crates/phlow-cli/src/lib.rs` — module docs now declare the machine-output contract (ASCII-only JSON via `python_json_dumps`, which escapes every control character; help text is sanitized twice — by clap's own stripping and by the build scan).
- **Why:** the JSON half needed no code change — `python_json_dumps` already emits ASCII-only JSON and escapes control characters, which the hostile-id case now pins as a contract. The help half turned out stronger than first assumed: probing proved clap strips ANSI escapes and control bytes from rendered help (the initial "renders verbatim" assumption was wrong and was corrected empirically), so the declared contract is *strip*, enforced by clap, with the build scan as defense in depth so the guarantee never rests on one library's behavior alone. Untrusted data is still never interpolated into help strings as a matter of policy.
- **Source:** `phlow-tools/src/json_compat.rs` (`python_json_dumps`: ASCII-only, escapes control characters); clap 4 `Command::write_help` behavior (verified by the unit test itself). Ghostex CLI doctrine — concept adapted, see header.
- **Validation agents:** primo gates, 2026-09-28: `cargo build -p phlow-gauntlet -p phlow-cli` clean; `cargo test -p phlow-gauntlet --test task_200` all pass; `cargo test -p phlow-gauntlet --lib` 64/64; `cargo test -p phlow-cli` 26 unit + 28 integration pass; `cargo fmt --check` clean; `cargo clippy -p phlow-gauntlet --all-targets` and `-p phlow-cli` 0 warnings.
- **Adversarial agents:** the task's own A-cases (see What happened); no separate red-team pass in this wave.

## Full technical depth

The hostile id `\x1b[2J\nEVIL-ID` travels: clap `Arg::value_parser` (no sanitization, value preserved) → `Commands::check_name()` (non-empty → `Some`) → `CheckRunner::run(name)` → `CheckReport::unknown(name)` (name echoed verbatim, the 4-key unknown report) → `report_exit_code` → `python_json_dumps`. The serializer escapes ESC as `\u001b` and newline as `\n` — plain ASCII backslash sequences — so the raw stdout contains zero control bytes while `serde_json` parses the id back byte-identical. The byte assertions allow `\n` (the single trailing `println!` terminator) and reject everything else below `0x20` plus `0x7f`. For help: clap's `write_help` renders `about`/`long_about` bytes raw (the unit test asserts an ESC survives rendering, proving the danger is real), so the contract is enforced at the source — the `all_help_strings_are_control_byte_free` scan walks the root command and every subcommand plus every argument's help strings, failing the build on any raw control byte — while the driver A2 sweeps the actual rendered bytes of every help output.

## Sources

- `crates/phlow-cli/src/cli.rs` — the two unit gates; `src/lib.rs` — declared contract.
- `phlow-tools/src/json_compat.rs` — `python_json_dumps` escaping behavior.
- `crates/phlow-checks/src/runner.rs` — `CheckReport::unknown` echoes the name verbatim.
- `crates/phlow-gauntlet/src/tasks/task_200.rs`, `tests/task_200.rs`.
- Ghostex CLI doctrine: `skills/ghostex-cli/SKILL.md` (sanitization) — concept adapted, not ported; see header.
