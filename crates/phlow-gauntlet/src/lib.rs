#![forbid(unsafe_code)]

//! Gauntlet: the 130-task agent-orchestration proving ground for phlow (130 implemented).
//!
//! # Status: EXPERIMENTAL
//!
//! Each task in [`tasks`] attempts one difficult orchestration scenario
//! against either phlow's Rust crates or Matt's Neovim config (diver's
//! `lua/ai/harness`, driven through headless Neovim). Tasks are allowed to
//! fail — that is the point. Every attempt records a [`TaskReport`] with
//! evidence; failures name exactly where and how the task went wrong so the
//! fix between iterations can cite them.
//!
//! # Hard constraints (inherited, unchanged)
//!
//! - No autonomous self-modification; no self-approval or self-promotion.
//! - Human promotion gates stay mandatory; read-only defaults.
//! - Fail closed on missing evidence, timeouts, and exhausted budgets.
//! - The `nvim-lua` tasks execute only the crate's own `lua/gauntlet/`
//!   drivers plus diver's harness modules. They never touch Matt's live
//!   editor, plugins, or files outside the task scratch directory.
//!
//! # Layout
//!
//! - [`tasks`]: the 158 task modules, one per file, disjoint ownership.
//! - [`bounty`]: the shared scaffold for tasks 131–150 (async bug-bounty
//!   cyclical workflow): scope feeds, scheduler, finding lifecycle,
//!   validation pipeline, submission gate, fake platform.
//! - `lua/gauntlet/`: the Neovim-side drivers for `nvim-lua` tasks.
//! - `docs/gauntlet/`: the per-task learning corpus (ELI5 + cited + depth).

pub mod bounty;
pub mod bridge;
pub mod daemon_client;
pub mod pairing;
pub mod session_find;
pub mod skill_sync;
pub mod skillopt;
pub mod state_machine;
pub mod tasks;
pub mod wire;

use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Maximum number of tasks in the gauntlet. The task list is closed: adding
/// a 159th task is a design change, not an iteration.
pub const TASK_COUNT_MAX: usize = 250;

/// Maximum length in characters of a single evidence line in a report.
/// Evidence is diagnostic text, not bulk data; oversized lines are truncated
/// by the recorder, never by silent drop.
pub const EVIDENCE_LINE_CHARS_MAX: usize = 2000;

/// Maximum number of evidence lines kept per task report.
pub const EVIDENCE_LINES_MAX: usize = 64;

/// Default per-task wall-clock budget in milliseconds.
pub const TASK_TIMEOUT_MS_DEFAULT: u64 = 120_000;

/// How a task is driven.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskKind {
    /// Driven through headless Neovim running diver's `lua/ai/harness`.
    NvimLua,
    /// Driven directly against phlow's Rust crates.
    Rust,
}

impl fmt::Display for TaskKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TaskKind::NvimLua => write!(f, "nvim-lua"),
            TaskKind::Rust => write!(f, "rust"),
        }
    }
}

/// Execution context handed to every task.
#[derive(Debug, Clone)]
pub struct Ctx {
    /// Headless Neovim binary used by `nvim-lua` tasks.
    pub nvim_bin: PathBuf,
    /// Diver's `lua/` directory, put on the runtimepath so
    /// `require("ai.harness")` loads Matt's actual config code.
    pub diver_lua_dir: PathBuf,
    /// This crate's `lua/gauntlet/` directory with the task drivers.
    pub gauntlet_lua_dir: PathBuf,
    /// Scratch directory the task may write to. Nothing else is writable.
    pub work_dir: PathBuf,
    /// Wall-clock budget for the task attempt.
    pub timeout: Duration,
}

impl Ctx {
    /// Build a context from the crate layout plus explicit overrides.
    ///
    /// `nvim_bin` and `diver_lua_dir` must be supplied: the gauntlet never
    /// guesses where Matt's editor or config live.
    pub fn new(
        nvim_bin: PathBuf,
        diver_lua_dir: PathBuf,
        work_dir: PathBuf,
    ) -> Result<Self, GauntletError> {
        if nvim_bin.as_os_str().is_empty() {
            return Err(GauntletError::EmptyField { field: "nvim_bin" });
        }
        if diver_lua_dir.as_os_str().is_empty() {
            return Err(GauntletError::EmptyField {
                field: "diver_lua_dir",
            });
        }
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        Ok(Ctx {
            nvim_bin,
            diver_lua_dir,
            gauntlet_lua_dir: manifest_dir.join("lua").join("gauntlet"),
            work_dir,
            timeout: Duration::from_millis(TASK_TIMEOUT_MS_DEFAULT),
        })
    }
}

/// The outcome of one task attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskOutcome {
    /// The task did what its spec claims. Evidence shows the mechanism.
    Pass {
        /// Bounded diagnostic lines (see [`EVIDENCE_LINES_MAX`]).
        evidence: Vec<String>,
    },
    /// The task went wrong. Names where and how, with evidence.
    Fail {
        /// Which stage or component went wrong, e.g. `"fan-in"`.
        where_: String,
        /// What happened, in one or two sentences.
        how: String,
        /// Bounded diagnostic lines supporting the diagnosis.
        evidence: Vec<String>,
    },
}

/// One recorded task attempt.
#[derive(Debug, Clone)]
pub struct TaskReport {
    /// e.g. `"task-01"`.
    pub id: &'static str,
    /// Human-readable name.
    pub name: &'static str,
    /// How the task is driven.
    pub kind: TaskKind,
    /// What happened.
    pub outcome: TaskOutcome,
    /// Wall-clock time of the attempt in milliseconds.
    pub duration_ms: u64,
}

impl TaskReport {
    /// True when the task passed.
    pub fn passed(&self) -> bool {
        matches!(self.outcome, TaskOutcome::Pass { .. })
    }
}

/// Failures of the gauntlet framework itself (not of tasks under test).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GauntletError {
    /// A required text field was empty.
    EmptyField {
        /// Which field was empty.
        field: &'static str,
    },
    /// An unknown task id was requested.
    UnknownTask {
        /// The id that was requested.
        id: String,
    },
    /// A task id failed its `task-NN` shape check.
    BadTaskId {
        /// The offered id.
        id: String,
    },
}

impl fmt::Display for GauntletError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GauntletError::EmptyField { field } => {
                write!(f, "gauntlet: required field '{field}' was empty")
            }
            GauntletError::UnknownTask { id } => {
                write!(f, "gauntlet: unknown task id '{id}'")
            }
            GauntletError::BadTaskId { id } => {
                write!(f, "gauntlet: malformed task id '{id}' (want task-NN)")
            }
        }
    }
}

impl std::error::Error for GauntletError {}

/// Truncate an evidence line to [`EVIDENCE_LINE_CHARS_MAX`] characters and
/// cap the vector at [`EVIDENCE_LINES_MAX`] lines. Truncation is marked with
/// `…[truncated]` so it is never silent.
pub fn bound_evidence(lines: Vec<String>) -> Vec<String> {
    lines
        .into_iter()
        .take(EVIDENCE_LINES_MAX)
        .map(|line| {
            if line.chars().count() > EVIDENCE_LINE_CHARS_MAX {
                let kept: String = line.chars().take(EVIDENCE_LINE_CHARS_MAX).collect();
                format!("{kept}…[truncated]")
            } else {
                line
            }
        })
        .collect()
}

/// Run a headless-Neovim task driver and parse its JSON verdict.
///
/// Spawns `ctx.nvim_bin --headless -l <gauntlet_lua_dir>/<script>` with
/// `DIVER_LUA_DIR` and `GAUNTLET_WORK_DIR` in the environment. `DIVER_LUA_DIR`
/// is a scratch rtp-root shim built from `ctx.diver_lua_dir` (see
/// [`diver_rtp_shim`]): the drivers append it to the runtimepath and
/// `require('ai.harness')`. The driver
/// must print exactly one JSON verdict line to stdout:
///
/// ```json
/// {"id":"task-01","outcome":"pass","evidence":["..."]}
/// {"id":"task-01","outcome":"fail","where":"fan-in","how":"...","evidence":["..."]}
/// ```
///
/// `script` must have the shape `task-NN.lua` (no path separators); anything
/// else is rejected before spawning. `ctx.timeout` is enforced by killing
/// the child on expiry. This function never panics: spawn failures,
/// timeouts, and unparseable output all become `TaskOutcome::Fail`.
pub fn run_nvim_lua_driver(ctx: &Ctx, script: &str, expected_id: &'static str) -> TaskOutcome {
    run_nvim_lua_driver_with_env(ctx, script, expected_id, &[])
}

/// Build the diver runtimepath shim for one driver invocation.
///
/// The Lua drivers append `DIVER_LUA_DIR` to Neovim's runtimepath and then
/// `require('ai.harness')`, which needs `<rtp>/lua/ai/...` on the rtp.
/// `ctx.diver_lua_dir` is diver's *lua* directory, not an rtp root, so it is
/// bridged here with two symlinks — `lua -> <diver-lua>` and
/// `ai -> <diver-lua>/ai` — in a process-unique scratch dir under the temp
/// dir, never inside the driver's work dir (tests assert on work-dir
/// contents). The real tree is never touched. A test that already built its
/// own shim and passed it as `diver_lua_dir` simply gets a nested one, which
/// resolves through both symlink layers to the same files.
static RTP_SHIM_SEQ: AtomicU64 = AtomicU64::new(0);

fn diver_rtp_shim(ctx: &Ctx) -> Result<PathBuf, String> {
    let shim = std::env::temp_dir().join(format!(
        "gauntlet-diver-rtp-{}-{}",
        std::process::id(),
        RTP_SHIM_SEQ.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&shim)
        .map_err(|e| format!("cannot create rtp shim {}: {e}", shim.display()))?;
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&ctx.diver_lua_dir, shim.join("lua"))
            .map_err(|e| format!("cannot symlink rtp lua dir: {e}"))?;
        std::os::unix::fs::symlink(ctx.diver_lua_dir.join("ai"), shim.join("ai"))
            .map_err(|e| format!("cannot symlink rtp ai dir: {e}"))?;
    }
    Ok(shim)
}

/// [`run_nvim_lua_driver`] with extra environment variables for the driver
/// Outcome of [`wait_for_child`].
enum WaitOutcome {
    /// The child finished within the deadline: its exit status plus its
    /// captured output (or the error from reading it).
    Finished {
        status: std::process::ExitStatus,
        output: Result<std::process::Output, String>,
    },
    /// The child was killed after the deadline. Its pipes were dropped
    /// without draining, so there is no output to report.
    TimedOut,
}

/// Wait for a spawned driver child, killing it if `timeout` expires.
///
/// Returns [`WaitOutcome::Finished`] when the child exits in time and
/// [`WaitOutcome::TimedOut`] when it is killed after the deadline. `Err`
/// carries a `try_wait` failure.
///
/// On the timeout path the child is reaped with [`std::process::Child::wait`]
/// — which waits only for the process to exit — and its pipes are dropped
/// WITHOUT draining to EOF: any surviving process that inherited the pipes
/// (e.g. a grandchild of the driver) keeps EOF open, so `wait_with_output`
/// there turned an 800 ms budget into a ~19 s stall. The caller's timeout
/// arm already discards driver output, so nothing observable is lost.
fn wait_for_child(
    mut child: std::process::Child,
    timeout: Duration,
) -> Result<WaitOutcome, String> {
    use std::time::Instant;

    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let output = child.wait_with_output().map_err(|e| e.to_string());
                return Ok(WaitOutcome::Finished { status, output });
            }
            Ok(None) => {
                if started.elapsed() > timeout {
                    // Wedged driver: reap the process, drop the pipes, leave.
                    let _ = child.kill();
                    let _ = child.wait();
                    drop(child.stdout.take());
                    drop(child.stderr.take());
                    return Ok(WaitOutcome::TimedOut);
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

/// child (e.g. `GAUNTLET_SCENARIO`). Names and values are passed through
/// unchanged; the caller owns their meaning.
pub fn run_nvim_lua_driver_with_env(
    ctx: &Ctx,
    script: &str,
    expected_id: &'static str,
    extra_env: &[(&str, &str)],
) -> TaskOutcome {
    use std::process::{Command, Stdio};

    if !is_driver_script_name(script) {
        return TaskOutcome::Fail {
            where_: "spawn".to_string(),
            how: format!(
                "rejected driver script name '{script}' (want task-NN.lua or task-NNN.lua)"
            ),
            evidence: vec![],
        };
    }
    let script_path = ctx.gauntlet_lua_dir.join(script);
    let work_dir = ctx.work_dir.join(expected_id);
    if let Err(e) = std::fs::create_dir_all(&work_dir) {
        return TaskOutcome::Fail {
            where_: "spawn".to_string(),
            how: format!("cannot create work dir: {e}"),
            evidence: vec![],
        };
    }
    // Bridge `ctx.diver_lua_dir` (diver's lua dir) to the rtp-root layout the
    // drivers need; see `diver_rtp_shim`. Fail closed on I/O errors.
    let diver_rtp = match diver_rtp_shim(ctx) {
        Ok(shim) => shim,
        Err(e) => {
            return TaskOutcome::Fail {
                where_: "spawn".to_string(),
                how: e,
                evidence: vec![],
            };
        }
    };

    let mut cmd = Command::new(&ctx.nvim_bin);
    cmd.arg("--headless")
        .arg("-l")
        .arg(&script_path)
        .env("DIVER_LUA_DIR", &diver_rtp)
        .env("GAUNTLET_WORK_DIR", &work_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    let child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => {
            return TaskOutcome::Fail {
                where_: "spawn".to_string(),
                how: format!("cannot spawn nvim: {e}"),
                evidence: vec![],
            };
        }
    };

    // Bounded wait: poll try_wait, kill on expiry. No blocking wait without
    // a deadline — a wedged driver must not wedge the gauntlet.
    let outcome = match wait_for_child(child, ctx.timeout) {
        Err(e) => TaskOutcome::Fail {
            where_: "wait".to_string(),
            how: format!("cannot wait on nvim child: {e}"),
            evidence: vec![],
        },
        Ok(WaitOutcome::TimedOut) => TaskOutcome::Fail {
            where_: "timeout".to_string(),
            how: format!(
                "nvim driver exceeded {} ms and was killed",
                ctx.timeout.as_millis()
            ),
            evidence: vec![],
        },
        Ok(WaitOutcome::Finished {
            status: _,
            output: Err(e),
        }) => TaskOutcome::Fail {
            where_: "output".to_string(),
            how: format!("cannot read nvim output: {e}"),
            evidence: vec![],
        },
        Ok(WaitOutcome::Finished {
            status,
            output: Ok(out),
        }) => parse_driver_verdict(&out, expected_id, status.code()),
    };
    // The shim is scratch, never evidence: release it on every path.
    let _ = std::fs::remove_dir_all(&diver_rtp);
    outcome
}

/// Accept only `task_NN.lua` or `task_NNN.lua`: no directories, no
/// surprises on the command line. The underscore matches the on-disk
/// driver names (`lua/gauntlet/task_01.lua`); three-digit indices serve
/// tasks 100-130 while two-digit indices keep working unchanged.
fn is_driver_script_name(script: &str) -> bool {
    let bytes = script.as_bytes();
    (bytes.len() == 11 || bytes.len() == 12)
        && &bytes[0..5] == b"task_"
        && bytes[5].is_ascii_digit()
        && bytes[6].is_ascii_digit()
        && if bytes.len() == 12 {
            bytes[7].is_ascii_digit() && &bytes[8..12] == b".lua"
        } else {
            &bytes[7..11] == b".lua"
        }
}

/// A verdict line is a JSON object carrying a string `id` — not merely a
/// line that starts with `{`. Lua table dumps and log noise (`{1,2}`,
/// `{ not json`) used to shadow the real verdict; requiring a parseable
/// object with a string id keeps those out while preserving the id-mismatch
/// and missing-outcome diagnostics downstream.
fn is_verdict_line(line: &str) -> bool {
    match serde_json::from_str::<serde_json::Value>(line) {
        Ok(serde_json::Value::Object(map)) => {
            matches!(map.get("id"), Some(serde_json::Value::String(_)))
        }
        _ => false,
    }
}

/// Find the verdict line: stdout first, then stderr.
///
/// Rationale (empirically verified 2026-09-28): `nvim --headless -l` routes
/// Lua `print()` to stderr, while `io.stdout:write` reaches stdout. Drivers
/// written either way are heard; when the verdict comes from stderr the
/// report says so, so the stream choice stays visible instead of silently
/// accepted.
fn find_verdict_line<'a>(stdout: &'a str, stderr: &'a str) -> Option<(&'a str, &'static str)> {
    if let Some(line) = stdout.lines().find(|l| is_verdict_line(l)) {
        return Some((line, "stdout"));
    }
    stderr
        .lines()
        .find(|l| is_verdict_line(l))
        .map(|line| (line, "stderr"))
}

/// Parse the driver's single JSON verdict line from captured output.
fn parse_driver_verdict(
    out: &std::process::Output,
    expected_id: &'static str,
    exit_code: Option<i32>,
) -> TaskOutcome {
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let Some((line, stream)) = find_verdict_line(&stdout, &stderr) else {
        return TaskOutcome::Fail {
            where_: "verdict".to_string(),
            how: format!("no JSON verdict line on stdout or stderr (exit={exit_code:?})",),
            evidence: bound_evidence(vec![format!("stderr: {stderr}")]),
        };
    };
    let value: serde_json::Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => {
            return TaskOutcome::Fail {
                where_: "verdict".to_string(),
                how: format!("verdict line is not JSON: {e}"),
                evidence: bound_evidence(vec![format!("line: {line}")]),
            };
        }
    };
    let id = value.get("id").and_then(|v| v.as_str()).unwrap_or("");
    if id != expected_id {
        return TaskOutcome::Fail {
            where_: "verdict".to_string(),
            how: format!("verdict id '{id}' does not match '{expected_id}'"),
            evidence: vec![],
        };
    }
    let mut evidence: Vec<String> = value
        .get("evidence")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    if stream == "stderr" {
        evidence.insert(
            0,
            "note: verdict read from stderr (nvim -l routes print() there); prefer io.stdout:write"
                .to_string(),
        );
    }
    match value.get("outcome").and_then(|v| v.as_str()) {
        Some("pass") => TaskOutcome::Pass {
            evidence: bound_evidence(evidence),
        },
        Some("fail") => {
            let where_ = value
                .get("where")
                .and_then(|v| v.as_str())
                .unwrap_or("lua-driver")
                .to_string();
            let how = value
                .get("how")
                .and_then(|v| v.as_str())
                .unwrap_or("driver reported failure")
                .to_string();
            TaskOutcome::Fail {
                where_,
                how,
                evidence: bound_evidence(evidence),
            }
        }
        other => TaskOutcome::Fail {
            where_: "verdict".to_string(),
            how: format!("bad outcome value: {other:?}"),
            evidence: bound_evidence(vec![format!("stderr: {stderr}")]),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{WaitOutcome, find_verdict_line, is_driver_script_name, wait_for_child};

    /// Validation: every real driver name is accepted.
    #[test]
    fn driver_names_valid() {
        for name in [
            "task_00.lua",
            "task_01.lua",
            "task_09.lua",
            "task_10.lua",
            "task_20.lua",
            "task_99.lua",
            "task_100.lua",
            "task_115.lua",
            "task_116.lua",
            "task_120.lua",
            "task_130.lua",
        ] {
            assert!(is_driver_script_name(name), "{name:?} should be accepted");
        }
    }

    /// Adversarial: hostile or malformed names are all rejected. Each of
    /// these must never reach the nvim command line.
    #[test]
    fn driver_names_rejected() {
        let hostile = [
            "../task_01.lua",  // path traversal
            "task_01.lua ",    // trailing space
            " task_01.lua",    // leading space
            "TASK_01.LUA",     // wrong case
            "task-01.lua",     // dash instead of underscore
            "task_1.lua",      // short index
            "task_0111.lua",   // overlong index (four digits)
            "task_01.luax",    // wrong extension
            "task_01lua",      // missing dot
            "",                // empty
            "task_01.lua\n",   // embedded newline
            "sub/task_01.lua", // subdirectory
            "task_11a.lua",    // non-digit in three-digit index
        ];
        for name in hostile {
            assert!(!is_driver_script_name(name), "{name:?} should be rejected");
        }
    }

    /// Validation: stdout wins when both streams carry a verdict line.
    #[test]
    fn verdict_prefers_stdout() {
        let (line, stream) =
            find_verdict_line("{\"id\":\"task_01\"}", "{\"id\":\"task_02\"}").unwrap();
        assert_eq!(line, "{\"id\":\"task_01\"}");
        assert_eq!(stream, "stdout");
    }

    /// Validation: a print()-style driver (verdict on stderr) is still heard.
    #[test]
    fn verdict_falls_back_to_stderr() {
        let (line, stream) = find_verdict_line("startup noise\n", "{\"id\":\"task_02\"}").unwrap();
        assert_eq!(line, "{\"id\":\"task_02\"}");
        assert_eq!(stream, "stderr");
    }

    /// Adversarial: lines that merely contain braces are not verdicts; a
    /// missing verdict on both streams is None, not a guess.
    #[test]
    fn verdict_absent_is_none() {
        assert!(find_verdict_line("no json here", "lua error: boom").is_none());
        assert!(find_verdict_line("x = {1,2}", "traceback...").is_none());
    }

    /// Validation (the reported bug): `{`-leading garbage — a Lua table
    /// dump, a half-written line — no longer shadows the real verdict on
    /// stdout.
    #[test]
    fn verdict_skips_garbage_brace_lines() {
        let stdout = "{ not json\n{1,2}\n{\"id\":\"task_07\",\"outcome\":\"pass\"}\n";
        let (line, stream) = find_verdict_line(stdout, "").unwrap();
        assert_eq!(line, "{\"id\":\"task_07\",\"outcome\":\"pass\"}");
        assert_eq!(stream, "stdout");
    }

    /// Validation: stdout garbage with no verdict falls through to a real
    /// verdict on stderr, and the stream is reported honestly.
    #[test]
    fn verdict_found_on_stderr_past_stdout_garbage() {
        let stdout = "{garbage}\n{1,2}\n";
        let stderr = "noise\n{\"id\":\"task_07\",\"outcome\":\"pass\"}\n";
        let (line, stream) = find_verdict_line(stdout, stderr).unwrap();
        assert_eq!(line, "{\"id\":\"task_07\",\"outcome\":\"pass\"}");
        assert_eq!(stream, "stderr");
    }

    /// Adversarial: valid JSON that is not a verdict — no `id`, a non-object,
    /// a non-string `id` — is skipped in favor of the real verdict.
    #[test]
    fn verdict_skips_valid_json_non_verdicts() {
        let stdout = concat!(
            "{\"x\":1}\n",
            "[1,2]\n",
            "{\"id\":123}\n",
            "{\"id\":\"task_07\",\"outcome\":\"fail\",\"where\":\"w\",\"how\":\"h\"}\n",
        );
        let (line, stream) = find_verdict_line(stdout, "").unwrap();
        assert_eq!(
            line,
            "{\"id\":\"task_07\",\"outcome\":\"fail\",\"where\":\"w\",\"how\":\"h\"}"
        );
        assert_eq!(stream, "stdout");
    }

    /// Adversarial: when no line anywhere is a verdict-shaped object, the
    /// answer stays None — unchanged behavior, no guessing.
    #[test]
    fn verdict_absent_among_non_verdicts_is_none() {
        assert!(find_verdict_line("{\"x\":1}\n", "[1,2]\n{\"id\":42}\n").is_none());
        assert!(find_verdict_line("", "").is_none());
    }

    /// Spawn a piped `/bin/sh` child for the wait-path tests: no nvim in
    /// this sandbox, and the bug is in the wait logic, not the driver.
    fn piped_sh(script: &str) -> std::process::Child {
        use std::process::{Command, Stdio};
        Command::new("sh")
            .arg("-c")
            .arg(script)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("sh must be spawnable in the sandbox")
    }

    /// Validation (reproducer for the 800 ms -> ~19 s bug): a child whose
    /// grandchild inherits stdout still times out on budget. `sleep 30 &
    /// wait` parks the parent for 30 s while the background sleep holds the
    /// stdout pipe open; the old `wait_with_output` path drained to EOF and
    /// stalled. The orphaned sleep exits on its own.
    #[test]
    fn timeout_survives_pipe_holding_grandchild() {
        use std::time::{Duration, Instant};
        let started = Instant::now();
        let outcome = wait_for_child(piped_sh("sleep 30 & wait"), Duration::from_millis(800))
            .expect("try_wait must not fail");
        let elapsed = started.elapsed();
        assert!(matches!(outcome, WaitOutcome::TimedOut));
        assert!(
            elapsed < Duration::from_secs(10),
            "timeout took {elapsed:?}; the bug stalled ~19 s on an 800 ms budget"
        );
    }

    /// Validation: a fast child keeps the normal path — status and output
    /// both come back, so the restructure regresses nothing.
    #[test]
    fn fast_child_returns_status_and_output() {
        use std::time::Duration;
        let outcome = wait_for_child(piped_sh("echo hello-verdict"), Duration::from_secs(10))
            .expect("try_wait must not fail");
        match outcome {
            WaitOutcome::Finished { status, output } => {
                assert!(status.success());
                let out = output.expect("output read must succeed");
                assert!(String::from_utf8_lossy(&out.stdout).contains("hello-verdict"));
            }
            WaitOutcome::TimedOut => panic!("a fast child must not time out"),
        }
    }

    /// Adversarial: a grandchild writing continuously to stdout must not
    /// wedge the timeout path or grow memory without bound. The background
    /// loop hammers the pipe while the parent sleeps; we drop our pipe ends
    /// without draining, so the writer dies on SIGPIPE by itself and we
    /// return on budget.
    #[test]
    fn timeout_survives_chattering_grandchild() {
        use std::time::{Duration, Instant};
        let started = Instant::now();
        let outcome = wait_for_child(
            piped_sh("(while true; do echo chatter; done) & sleep 30"),
            Duration::from_millis(800),
        )
        .expect("try_wait must not fail");
        let elapsed = started.elapsed();
        assert!(matches!(outcome, WaitOutcome::TimedOut));
        assert!(
            elapsed < Duration::from_secs(10),
            "timeout took {elapsed:?} with a chattering grandchild"
        );
    }

    /// Adversarial: the child exits just as the deadline fires. Whichever
    /// way `try_wait` lands, the result is coherent — no panic, no hang,
    /// and `TimedOut` never carries output.
    #[test]
    fn kill_race_stays_coherent() {
        use std::time::{Duration, Instant};
        let started = Instant::now();
        let outcome = wait_for_child(piped_sh("true"), Duration::from_millis(0))
            .expect("try_wait must not fail");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the kill race must not hang"
        );
        match outcome {
            WaitOutcome::TimedOut => {}
            WaitOutcome::Finished { status, .. } => assert!(status.success()),
        }
    }
}
