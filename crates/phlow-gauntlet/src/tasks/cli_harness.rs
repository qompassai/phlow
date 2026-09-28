// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Shared subprocess harness for wave-32 (CLI UX doctrine) drivers.
//!
//! The wave-32 tasks test the real `phlow` binary the way the Ghostex
//! CLI doctrine is written: help-first discovery (`--help`), `--json`
//! machine output, stable ids, hostile-output sanitization. Every
//! driver here spawns the built binary and asserts on exit codes and
//! raw bytes — never on re-descriptions of the parser.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Named bound: subprocess output is never silently unbounded. The
/// wave-32 commands (`--help`, `status`, `check`) print kilobytes; the
/// cap only guards against a pathological build dumping megabytes.
pub const MAX_CAPTURE_BYTES: usize = 4 * 1024 * 1024;

/// Outcome of one `phlow` invocation. The byte streams stay raw (not
/// lossy UTF-8) so the sanitization task can assert on control bytes
/// exactly.
pub struct ProcOutcome {
    /// None when the process died by signal.
    pub code: Option<i32>,
    /// Raw stdout bytes.
    pub stdout: Vec<u8>,
    /// Raw stderr bytes.
    pub stderr: Vec<u8>,
}

/// Locate the `phlow` binary under test. Prefers Cargo's
/// `CARGO_BIN_EXE_phlow` (set when `phlow-cli` is a dev-dependency);
/// otherwise walks up from the test executable looking for a `phlow`
/// file next to a `target/debug`-style dir. The walk (not a single
/// `deps/` guess) is deliberate: the test executable may sit in
/// `target/debug/deps/` on a normal layout, but this toolchain was
/// observed placing test executables in
/// `target/debug/build/phlow-gauntlet/<hash>/out/` — both resolve by
/// walking up to the dir that actually contains `phlow`.
pub fn phlow_bin() -> PathBuf {
    if let Some(path) = option_env!("CARGO_BIN_EXE_phlow") {
        return PathBuf::from(path);
    }
    let exe = std::env::current_exe().expect("harness needs current_exe");
    let mut dir = exe
        .parent()
        .expect("test executable has a parent dir")
        .to_path_buf();
    for _ in 0..8 {
        let candidate = dir.join("phlow");
        if candidate.is_file() {
            return candidate;
        }
        if !dir.pop() {
            break;
        }
    }
    dir.join("phlow")
}

/// Fresh, per-case temp dir: parallel test cases never share a cwd.
pub fn case_temp_dir(case: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("phlow-wave32-{}-{case}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir for a wave-32 case");
    dir
}

fn truncate(mut bytes: Vec<u8>) -> Vec<u8> {
    bytes.truncate(MAX_CAPTURE_BYTES);
    bytes
}

/// Run `phlow` with `args` in `cwd`, capturing raw bytes.
pub fn run_phlow_in(cwd: &Path, args: &[&str]) -> io::Result<ProcOutcome> {
    let output = Command::new(phlow_bin())
        .args(args)
        .current_dir(cwd)
        .output()?;
    Ok(ProcOutcome {
        code: output.status.code(),
        stdout: truncate(output.stdout),
        stderr: truncate(output.stderr),
    })
}

/// Run `phlow` with `args` in a fresh per-case temp dir.
pub fn run_phlow(args: &[&str]) -> io::Result<ProcOutcome> {
    let dir = case_temp_dir("run");
    run_phlow_in(&dir, args)
}

/// Parse the `Commands:` section of `phlow --help` into subcommand
/// names. Fails closed: an unparseable listing is a fixture error,
/// never a silently empty command list. clap also lists its own
/// auto-generated `help` subcommand; it is kept — the enumeration is
/// honest about what the binary actually offers.
pub fn parse_help_commands(help_stdout: &[u8]) -> Result<Vec<String>, String> {
    let text = String::from_utf8_lossy(help_stdout);
    let mut names = Vec::new();
    let mut in_commands = false;
    for line in text.lines() {
        if line.trim() == "Commands:" {
            in_commands = true;
            continue;
        }
        if !in_commands {
            continue;
        }
        if line.trim().is_empty() {
            break;
        }
        // clap renders two-space-indented `name  description` rows; a
        // wrapped description continuation would also be indented, so
        // only the first token of each row is taken and rows are
        // validated below by actually invoking `<name> --help`.
        if line.starts_with(char::is_whitespace)
            && let Some(first) = line.split_whitespace().next()
        {
            names.push(first.to_string());
        }
    }
    if names.is_empty() {
        return Err("phlow --help has no parseable Commands: section".to_string());
    }
    Ok(names)
}

/// Assert `bytes` contain no raw control bytes: every byte is `\n`,
/// printable ASCII, or non-ASCII UTF-8 (>= 0x20, != 0x7f). JSON string
/// escapes (`\u001b`) are plain ASCII backslash sequences, so they
/// pass; a raw ESC byte fails.
pub fn assert_no_raw_control_bytes(bytes: &[u8], what: &str) -> Result<(), String> {
    for (offset, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            continue;
        }
        if *byte < 0x20 || *byte == 0x7f {
            return Err(format!(
                "{what}: raw control byte 0x{byte:02x} at offset {offset}"
            ));
        }
    }
    Ok(())
}

/// Parse one JSON value out of a command's stdout. The report commands
/// print exactly one JSON line; anything else is a fixture error.
pub fn parse_json_stdout(outcome: &ProcOutcome, what: &str) -> Result<serde_json::Value, String> {
    let text = String::from_utf8(outcome.stdout.clone())
        .map_err(|e| format!("{what}: stdout is not UTF-8: {e}"))?;
    serde_json::from_str(text.trim_end())
        .map_err(|e| format!("{what}: stdout does not parse as JSON: {e}"))
}

/// Collect every dotted field path in a JSON value (`a.b[]` for array
/// elements): the field-name set the stability task compares.
pub fn field_paths(value: &serde_json::Value, prefix: &str, out: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                out.push(path.clone());
                field_paths(child, &path, out);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                field_paths(item, &format!("{prefix}[]"), out);
            }
        }
        _ => {}
    }
}
