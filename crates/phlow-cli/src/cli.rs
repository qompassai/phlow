// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Command-line surface, ported from `flow/main.py`'s `parser()`.
//!
//! CLI UX doctrine (adapted from Ghostex's `skills/ghostex-cli`):
//! help-first discovery (every subcommand documents itself), `--json`
//! machine output (valid JSON always, stable field names, stable ids),
//! verify-after-act for mutating commands. Attribution: see header.
//!
//! Python builds a `common` parent parser with suppressed defaults so the
//! global flags are accepted before **or** after the subcommand; clap's
//! `global = true` is the same mechanism. `--version` is deliberately *not*
//! global: Python registers it on the root parser only, so
//! `phlow run --version` is a usage error (exit 2) there and here.

use std::path::PathBuf;

use clap::{ArgAction, Parser, Subcommand};

/// Parse a workspace path, accepting the empty string: Python's argparse
/// accepts `--workspace ""` and resolves it to the caller's cwd, so the
/// value parser must not reject it (clap's default `PathBuf` parser does).
fn parse_workspace_path(raw: &str) -> Result<PathBuf, std::convert::Infallible> {
    Ok(PathBuf::from(raw))
}

/// Bounded local multi-agent coding.
///
/// Exit codes mirror `flow/main.py`: 0 success, 1 the command ran but the
/// report status is not `ok`, 2 usage/config error, 130 interrupted.
#[derive(Parser, Debug)]
#[command(name = "phlow", disable_version_flag = true)]
pub struct Cli {
    /// Explicitly approved TOML (project-local config is never auto-loaded).
    #[arg(long, short = 'c', global = true)]
    pub config: Option<PathBuf>,

    /// Workspace directory; resolved before any tool registration.
    /// The empty string is accepted and resolves to the caller's cwd,
    /// exactly like Python's `--workspace ""`.
    #[arg(long, short = 'w', global = true, value_parser = parse_workspace_path)]
    pub workspace: Option<PathBuf>,

    /// Override all role models.
    #[arg(long, short = 'm', global = true)]
    pub model: Option<String>,

    /// Allow workspace writes and configured named checks (not an OS sandbox).
    #[arg(long, global = true, action = ArgAction::SetTrue)]
    pub trusted: bool,

    /// Explicit private Neovim socket.
    #[arg(long, global = true)]
    pub nvim: Option<String>,

    /// Bounded reverse editor request timeout in seconds (default 120).
    /// Negative numbers are parsed as values (not flags) so `-1` reaches
    /// the range check, exactly like argparse: Python reports
    /// `--editor-timeout must be between 0.1 and 660 seconds`, exit 2.
    /// Values are parsed with [`parse_python_float`], matching CPython's
    /// `float()` grammar (argparse's `type=float`).
    #[arg(long, global = true, allow_negative_numbers = true, value_parser = parse_python_float)]
    pub editor_timeout: Option<f64>,

    /// Print `Phlow 0.2.0` and exit. Root-only, like Python's version action.
    #[arg(long, action = ArgAction::SetTrue)]
    pub version: bool,

    /// Machine-readable output contract (Ghostex `--json` doctrine, adapted).
    /// Report commands (`status`, `check`, `run`) already emit one JSON
    /// line; this flag asserts that contract. Commands that cannot honor
    /// it (the interactive TUI) refuse with a usage error instead of
    /// printing human text.
    #[arg(long, global = true, action = ArgAction::SetTrue)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

/// Subcommands, mirroring the `argparse` subparsers in `flow/main.py`.
#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum Commands {
    /// Run one task and print a JSON report.
    Run {
        /// Task words; joined with single spaces like Python's
        /// `" ".join(args.task)`.
        #[arg(num_args = 1.., required = true)]
        task: Vec<String>,
    },
    /// Serve newline-framed MCP over stdio.
    Serve,
    /// Run configured named checks.
    Check {
        /// Check name (positional).
        name: Option<String>,
        /// Run one configured named check. Wins over the positional name,
        /// mirroring `args.check_name or args.name`. The flag is spelled
        /// `--name`, exactly like Python's `add_argument("--name")`.
        #[arg(long = "name")]
        check_name: Option<String>,
    },
    /// Show local capabilities without model calls.
    Status,
    /// Interactive terminal frontend.
    Tui,
}

impl Commands {
    /// The check name Python resolves as `args.check_name or args.name`.
    /// Python's `or` takes the first *truthy* operand, so an empty
    /// `--name ""` falls through to the positional, and `check --name ""`
    /// runs all checks.
    pub fn check_name(&self) -> Option<&str> {
        match self {
            Commands::Check { name, check_name } => {
                let flag = check_name.as_deref().filter(|text| !text.is_empty());
                let positional = name.as_deref().filter(|text| !text.is_empty());
                flag.or(positional)
            }
            _ => None,
        }
    }
}

/// Parse a CLI value the way CPython's `float()` (argparse `type=float`)
/// does: surrounding ASCII whitespace is stripped, single underscores are
/// allowed only between ASCII digits, and `inf`/`infinity`/`nan` are
/// accepted case-insensitively. Non-finite or out-of-range values are NOT
/// rejected here — the existing range check in `lib.rs` handles them with
/// Python's exact message.
///
/// Accepted: `"  5 "`, `"1_0"` (10.0), `" 1_0 "`, `"infinity"`, `"INF"`,
/// `"nan"`. Rejected: `"1__0"`, `"_1"`, `"1_"`, `"1d5"` (no `d` exponent).
fn parse_python_float(text: &str) -> Result<f64, String> {
    // CPython strips space, \t, \n, \r, \x0b, \x0c. Rust's
    // `str::trim` is close but not identical (its whitespace table
    // differs), so the exact CPython set is spelled out.
    let trimmed = text.trim_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c']);
    let (negative, rest) = match trimmed.strip_prefix(['+', '-']) {
        Some(rest) => (trimmed.starts_with('-'), rest),
        None => (false, trimmed),
    };
    if rest.is_empty() {
        return Err(invalid_float(text));
    }
    let lowered = rest.to_ascii_lowercase();
    if lowered == "inf" || lowered == "infinity" {
        return Ok(if negative {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        });
    }
    if lowered == "nan" {
        // Sign is ignored for NaN on both sides (argparse type=float).
        return Ok(f64::NAN);
    }
    // CPython's rule: every underscore must sit between two ASCII digits.
    let bytes = rest.as_bytes();
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'_' {
            let left_digit = index > 0 && bytes[index - 1].is_ascii_digit();
            let right_digit = index + 1 < bytes.len() && bytes[index + 1].is_ascii_digit();
            if !(left_digit && right_digit) {
                return Err(invalid_float(text));
            }
        }
    }
    let digits: String = rest.chars().filter(|ch| *ch != '_').collect();
    let value: f64 = digits.parse().map_err(|_| invalid_float(text))?;
    // The sign was stripped before underscore validation; re-apply it.
    Ok(if negative { -value } else { value })
}

/// argparse renders a bad float as `invalid float value: '<text>'`.
fn invalid_float(text: &str) -> String {
    format!("invalid float value: '{text}'")
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory as _;

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(args).expect("test argv must parse")
    }

    #[test]
    fn global_flags_parse_before_the_subcommand() {
        let cli = parse(&["phlow", "--trusted", "--workspace", "/tmp", "status"]);
        assert!(cli.trusted);
        assert_eq!(cli.workspace, Some(PathBuf::from("/tmp")));
        assert!(matches!(cli.command, Some(Commands::Status)));
    }

    #[test]
    fn global_flags_parse_after_the_subcommand() {
        let cli = parse(&["phlow", "status", "--trusted", "--workspace", "/tmp"]);
        assert!(cli.trusted);
        assert_eq!(cli.workspace, Some(PathBuf::from("/tmp")));
    }

    #[test]
    fn short_flags_match_python() {
        let cli = parse(&["phlow", "-c", "a.toml", "-w", "/tmp", "-m", "m", "status"]);
        assert_eq!(cli.config, Some(PathBuf::from("a.toml")));
        assert_eq!(cli.workspace, Some(PathBuf::from("/tmp")));
        assert_eq!(cli.model.as_deref(), Some("m"));
    }

    #[test]
    fn empty_workspace_value_is_accepted_like_python() {
        // Python's argparse accepts `--workspace ""`; it resolves to the
        // caller's cwd. clap's default PathBuf parser rejects empty
        // values, so the flag uses a custom parser (verified 2026-09-26).
        let cli = parse(&["phlow", "--workspace", "", "status"]);
        assert_eq!(cli.workspace, Some(PathBuf::from("")));
    }

    #[test]
    fn run_requires_at_least_one_task_word() {
        assert!(Cli::try_parse_from(["phlow", "run"]).is_err());
        let cli = parse(&["phlow", "run", "hello", "world"]);
        assert!(matches!(cli.command, Some(Commands::Run { .. })));
    }

    #[test]
    fn run_accepts_global_flags_after_the_task_words() {
        let cli = parse(&["phlow", "run", "hello", "--trusted"]);
        assert!(cli.trusted);
        assert!(matches!(cli.command, Some(Commands::Run { .. })));
    }

    #[test]
    fn version_flag_is_root_only_like_python() {
        // Python: `phlow run --version` -> argparse error, exit 2.
        assert!(Cli::try_parse_from(["phlow", "run", "--version"]).is_err());
        let cli = parse(&["phlow", "--version"]);
        assert!(cli.version);
        assert!(cli.command.is_none());
    }

    #[test]
    fn unknown_flag_is_a_usage_error() {
        assert!(Cli::try_parse_from(["phlow", "--bogus"]).is_err());
        assert!(Cli::try_parse_from(["phlow", "status", "--bogus"]).is_err());
    }

    #[test]
    fn check_name_flag_wins_over_positional() {
        let cli = parse(&["phlow", "check", "positional", "--name", "flag"]);
        assert_eq!(
            cli.command.as_ref().and_then(Commands::check_name),
            Some("flag")
        );
        let cli = parse(&["phlow", "check", "positional"]);
        assert_eq!(
            cli.command.as_ref().and_then(Commands::check_name),
            Some("positional")
        );
        let cli = parse(&["phlow", "check"]);
        assert_eq!(cli.command.as_ref().and_then(Commands::check_name), None);
    }

    #[test]
    fn empty_check_name_falls_through_like_python_or() {
        // Python: `args.check_name or args.name` — empty strings are
        // falsy, so `--name ""` falls through to the positional, and a
        // bare `--name ""` runs all checks.
        let cli = parse(&["phlow", "check", "--name", ""]);
        assert_eq!(cli.command.as_ref().and_then(Commands::check_name), None);
        let cli = parse(&["phlow", "check", "positional", "--name", ""]);
        assert_eq!(
            cli.command.as_ref().and_then(Commands::check_name),
            Some("positional")
        );
    }

    #[test]
    fn editor_timeout_parses_like_cpython_float() {
        // Accepted by CPython's float(): surrounding ASCII whitespace,
        // single underscores between digits, inf/infinity/nan (any case).
        for (text, expected) in [
            ("5", 5.0),
            ("  5 ", 5.0),
            ("\t5\n", 5.0),
            ("1_0", 10.0),
            (" 1_0 ", 10.0),
            ("1_2.5_6", 12.56),
            (".5", 0.5),
            ("5.", 5.0),
            ("+5", 5.0),
            ("-5", -5.0),
            ("1e3", 1000.0),
            ("infinity", f64::INFINITY),
            ("INF", f64::INFINITY),
            ("+inf", f64::INFINITY),
            ("nan", f64::NAN),
            ("NaN", f64::NAN),
        ] {
            let cli = parse(&["phlow", "--editor-timeout", text]);
            let value = cli.editor_timeout.expect("must parse");
            if expected.is_nan() {
                assert!(value.is_nan(), "{text:?} must parse as NaN");
            } else {
                assert_eq!(value, expected, "{text:?}");
            }
        }
    }

    #[test]
    fn editor_timeout_rejects_non_python_floats() {
        // CPython's float() rejects these: doubled/leading/trailing
        // underscores, non-decimal exponents, empty input. ("-inf" and
        // "-nan" never reach the parser: like argparse, clap treats a
        // leading '-' followed by non-digits as a flag, so both sides
        // report a usage error.)
        for text in [
            "1__0", "_1", "1_", "1d5", "", "   ", "abc", "1_.5", "1e_5", "-inf", "-nan",
        ] {
            assert!(
                Cli::try_parse_from(["phlow", "--editor-timeout", text]).is_err(),
                "{text:?} must be a usage error"
            );
        }
    }

    #[test]
    fn about_text_is_bin_neutral() {
        // The about text must not hardcode a binary name: only `phlow`
        // ships now, and the text must stay accurate if that changes.
        let about = Cli::command()
            .get_about()
            .map(|s| s.to_string())
            .unwrap_or_default();
        assert!(
            !about.starts_with("`phlow`"),
            "about names a binary: {about}"
        );
        assert!(!about.contains("`flow`"), "about names a binary: {about}");
    }

    #[test]
    fn no_subcommand_means_tui_like_python() {
        let cli = parse(&["phlow"]);
        assert!(cli.command.is_none());
        assert!(!cli.version);
    }

    #[test]
    fn all_subcommands_parse() {
        for argv in [
            vec!["phlow", "run", "x"],
            vec!["phlow", "serve"],
            vec!["phlow", "check"],
            vec!["phlow", "status"],
            vec!["phlow", "tui"],
        ] {
            assert!(Cli::try_parse_from(argv).is_ok());
        }
    }

    #[test]
    fn json_flag_is_global_like_other_flags() {
        // --json is accepted before or after the subcommand, like the
        // other global flags (Python-parity convention).
        let cli = parse(&["phlow", "--json", "status"]);
        assert!(cli.json);
        let cli = parse(&["phlow", "status", "--json"]);
        assert!(cli.json);
        let cli = parse(&["phlow"]);
        assert!(!cli.json);
    }

    #[test]
    fn every_subcommand_has_help_string() {
        // Static scan (gauntlet task-197 V2): the help-first doctrine
        // requires every subcommand to document itself. A new Commands
        // variant without a doc comment fails the build here.
        let mut count = 0;
        for sub in Cli::command().get_subcommands() {
            count += 1;
            let name = sub.get_name();
            let about = sub.get_about().map(|s| s.to_string()).unwrap_or_default();
            let long_about = sub
                .get_long_about()
                .map(|s| s.to_string())
                .unwrap_or_default();
            assert!(
                !about.trim().is_empty() || !long_about.trim().is_empty(),
                "subcommand '{name}' has no help text: add a doc comment \
                 to the Commands variant"
            );
        }
        assert!(count > 0, "expected at least one subcommand");
    }

    /// Fail when any help/about string in the CLI definition contains a
    /// raw control byte. The tree is two levels deep (root + subcommands),
    /// so no recursion is needed.
    fn assert_help_text_clean(label: &str, text: &str) {
        for (offset, byte) in text.bytes().enumerate() {
            assert!(
                byte == b'\n' || byte >= 0x20,
                "{label}: raw control byte 0x{byte:02x} at offset {offset}"
            );
        }
    }

    #[test]
    fn all_help_strings_are_control_byte_free() {
        // Static scan (gauntlet task-200 A2): help text is static,
        // compile-time literals; untrusted data is never interpolated
        // into it. This test enforces the literal half at build time.
        let root = Cli::command();
        let mut commands: Vec<(String, &clap::Command)> = vec![("phlow".to_string(), &root)];
        // Two levels only: root, then each subcommand's own strings.
        for sub in root.get_subcommands() {
            commands.push((format!("phlow {}", sub.get_name()), sub));
        }
        for (label, cmd) in &commands {
            if let Some(about) = cmd.get_about() {
                assert_help_text_clean(&format!("{label} about"), &about.to_string());
            }
            if let Some(long_about) = cmd.get_long_about() {
                assert_help_text_clean(&format!("{label} long_about"), &long_about.to_string());
            }
            for arg in cmd.get_arguments() {
                let arg_label = format!("{label} --{}", arg.get_id());
                if let Some(help) = arg.get_help() {
                    assert_help_text_clean(&arg_label, &help.to_string());
                }
                if let Some(long_help) = arg.get_long_help() {
                    assert_help_text_clean(&arg_label, &long_help.to_string());
                }
            }
        }
        assert!(
            commands.len() > 1,
            "expected subcommands under the root command"
        );
    }

    #[test]
    fn hostile_about_is_sanitized_by_clap() {
        // Declared contract, verified empirically (not assumed): clap
        // strips ANSI escape sequences and control bytes from help
        // text, keeping `\n` as legitimate formatting. A hostile
        // `about` containing `\x1b[2J` renders with the whole sequence
        // removed — the terminal-injection half of task-200's contract
        // is enforced by clap itself (strip, not escape). The
        // `all_help_strings_are_control_byte_free` scan above is the
        // defense-in-depth half: help literals must be clean at the
        // source, so the contract never depends on one library's
        // behavior alone. If clap ever changes its sanitization, this
        // test fails and the contract is revisited.
        let mut hostile = clap::Command::new("demo").about("wipe\x1b[2Jscreen\x07bell");
        let mut rendered = Vec::new();
        hostile.write_help(&mut rendered).expect("help must render");
        assert!(
            !rendered.contains(&0x1b),
            "clap must not render a raw ESC byte into help output"
        );
        assert!(
            !rendered.contains(&0x07),
            "clap must not render a raw BEL byte into help output"
        );
        assert!(
            !String::from_utf8_lossy(&rendered).contains("[2J"),
            "the ANSI clear-screen sequence must not survive rendering"
        );
    }
}
