//! Bounded execution of completions against hidden cases.
//!
//! # Trust boundary
//!
//! This module **executes model-generated code**. Phlow is not an OS
//! sandbox and this executor does not claim to be one: it runs the
//! completion with the operator's own interpreter, in a fresh
//! temporary directory, under a deadline, with bounded output, and
//! only when the operator explicitly acknowledged code execution in
//! [`ExecutorConfig`]. That acknowledgment is the phlow contract —
//! the same one the approved-checks system uses — not a sandbox
//! promise.
//!
//! The harness protocol: the executor writes `source.py` (prompt +
//! completion), `cases.json`, and a fixed `harness.py`, then runs
//! `interpreter harness.py source.py cases.json <entry_point>`. The
//! harness prints one `PHLOW_TRAINLAB_RESULT {json}` line; anything
//! else (no line, timeout, truncation) classifies as `Invalid`.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::error::TrainlabError;
use crate::reward::{EvalStatus, Evaluation};
use crate::task::CodingTask;

/// Maximum completion size accepted for execution (bytes).
pub const COMPLETION_BYTES_MAX: usize = 64 * 1024;
/// Default per-completion deadline.
pub const DEADLINE_DEFAULT: Duration = Duration::from_secs(10);
/// Maximum deadline a configuration may request.
pub const DEADLINE_MAX: Duration = Duration::from_secs(120);
/// Default bound on captured harness output (bytes per stream).
pub const OUTPUT_BYTES_MAX_DEFAULT: usize = 64 * 1024;
/// Maximum evaluation-message length retained (characters).
pub const MESSAGE_CHARS_MAX: usize = 512;
/// Marker prefix of the harness result line.
const RESULT_MARKER: &str = "PHLOW_TRAINLAB_RESULT ";

/// The fixed evaluation harness. It compiles the candidate (syntax
/// errors are `invalid`, not harness failures), executes it, checks
/// the entry point exists, and runs the hidden cases.
const HARNESS_PY: &str = r#"import json, sys

def emit(status, message, passed, total):
    print("PHLOW_TRAINLAB_RESULT " + json.dumps(
        {"status": status, "message": message,
         "tests_passed": passed, "tests_total": total}))

def main():
    source_path, cases_path, entry_point = sys.argv[1], sys.argv[2], sys.argv[3]
    with open(source_path, encoding="utf-8") as handle:
        source = handle.read()
    try:
        code = compile(source, "<candidate>", "exec")
    except SyntaxError as error:
        emit("invalid", "syntax error: %s" % error, 0, 0)
        return
    namespace = {}
    try:
        exec(code, namespace)
    except Exception as error:  # candidate raised at import time
        emit("invalid", "exec error: %s" % error, 0, 0)
        return
    function = namespace.get(entry_point)
    if not callable(function):
        emit("invalid", "entry point missing: %s" % entry_point, 0, 0)
        return
    with open(cases_path, encoding="utf-8") as handle:
        cases = json.load(handle)
    passed = 0
    for case in cases:
        try:
            result = function(*case["args"])
        except Exception:
            continue
        if result == case["expected"]:
            passed += 1
    total = len(cases)
    emit("passed" if passed == total else "failed", "", passed, total)

main()
"#;

/// Executor configuration. All fields are validated before any
/// completion runs.
#[derive(Debug, Clone)]
pub struct ExecutorConfig {
    /// Interpreter argv (program + fixed leading args). The harness
    /// path and its arguments are appended by the executor.
    pub interpreter_argv: Vec<String>,
    /// Per-completion wall-clock deadline.
    pub deadline: Duration,
    /// Bound on captured stdout/stderr, per stream (bytes).
    pub output_bytes_max: usize,
    /// Parent directory for per-evaluation temp dirs; `None` uses the
    /// OS temp dir.
    pub work_root: Option<PathBuf>,
    /// The operator's explicit acknowledgment that generated code
    /// will be executed. Without it, every evaluation is an error.
    pub acknowledge_code_execution: bool,
}

impl Default for ExecutorConfig {
    fn default() -> Self {
        ExecutorConfig {
            interpreter_argv: vec!["python3".to_string()],
            deadline: DEADLINE_DEFAULT,
            output_bytes_max: OUTPUT_BYTES_MAX_DEFAULT,
            work_root: None,
            acknowledge_code_execution: false,
        }
    }
}

impl ExecutorConfig {
    /// Validate bounds and the acknowledgment gate.
    pub fn validate(&self) -> Result<(), TrainlabError> {
        if self.interpreter_argv.is_empty() {
            return Err(TrainlabError::InvalidConfig(
                "interpreter_argv must not be empty".to_string(),
            ));
        }
        if self.deadline.is_zero() || self.deadline > DEADLINE_MAX {
            return Err(TrainlabError::InvalidConfig(format!(
                "deadline {:?} outside (0, {:?}]",
                self.deadline, DEADLINE_MAX
            )));
        }
        if self.output_bytes_max == 0 {
            return Err(TrainlabError::LimitExceeded(
                "output_bytes_max must be positive".to_string(),
            ));
        }
        Ok(())
    }
}

/// The bounded completion executor. Owns only its configuration;
/// every evaluation creates and removes its own temp directory.
#[derive(Debug, Clone)]
pub struct Executor {
    config: ExecutorConfig,
}

/// Harness result line, as printed by `HARNESS_PY`.
#[derive(Debug, Deserialize)]
struct HarnessResult {
    status: String,
    message: String,
    tests_passed: usize,
    tests_total: usize,
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// RAII temp directory: removed on drop. Removal is best-effort —
/// a leftover temp dir is litter, not a correctness hazard, so a
/// failed removal is intentionally not propagated.
struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn create(root: &Path) -> Result<TempDirGuard, TrainlabError> {
        let unique = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = root.join(format!("phlow-trainlab-{}-{unique}", std::process::id()));
        fs::create_dir(&path)?;
        Ok(TempDirGuard { path })
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

impl Executor {
    /// Build an executor from a validated configuration.
    pub fn new(config: ExecutorConfig) -> Result<Executor, TrainlabError> {
        config.validate()?;
        Ok(Executor { config })
    }

    /// Evaluate one completion against `task`'s hidden cases.
    ///
    /// Returns `Err` only when execution itself was impossible
    /// (unacknowledged execution, spawn failure). Every outcome of
    /// the completion — pass, fail, invalid, timeout — is an
    /// [`Evaluation`] value.
    pub fn evaluate(
        &self,
        task: &CodingTask,
        completion: &str,
    ) -> Result<Evaluation, TrainlabError> {
        if !self.config.acknowledge_code_execution {
            return Err(TrainlabError::Executor(
                "code execution not acknowledged by the operator".to_string(),
            ));
        }
        if completion.len() > COMPLETION_BYTES_MAX {
            return Ok(Evaluation::invalid(format!(
                "completion exceeds {COMPLETION_BYTES_MAX} bytes"
            )));
        }
        let root = self
            .config
            .work_root
            .clone()
            .unwrap_or_else(std::env::temp_dir);
        let temp = TempDirGuard::create(&root)?;
        let source = format!("{}{}", task.prompt, completion);
        fs::write(temp.path.join("source.py"), source)?;
        fs::write(temp.path.join("harness.py"), HARNESS_PY)?;
        fs::write(
            temp.path.join("cases.json"),
            serde_json::to_string(&task.cases)?,
        )?;
        let output = self.run_harness(&temp.path, &task.entry_point)?;
        match output {
            HarnessOutput::TimedOut => Ok(Evaluation::invalid("executor deadline exceeded")),
            HarnessOutput::Completed(stdout) => Ok(classify_harness_output(&stdout)),
        }
    }

    /// Spawn the harness and collect its stdout under the deadline.
    fn run_harness(&self, dir: &Path, entry_point: &str) -> Result<HarnessOutput, TrainlabError> {
        let (program, leading) = self
            .config
            .interpreter_argv
            .split_first()
            .expect("validated non-empty interpreter argv");
        let mut command = Command::new(program);
        command
            .args(leading)
            .arg(dir.join("harness.py"))
            .arg(dir.join("source.py"))
            .arg(dir.join("cases.json"))
            .arg(entry_point)
            .current_dir(dir)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn()?;
        let stdout = take_pipe(child.stdout.take(), self.config.output_bytes_max);
        let stderr = take_pipe(child.stderr.take(), self.config.output_bytes_max);
        let timed_out = wait_with_deadline(&mut child, self.config.deadline)?;
        let stdout_bytes = stdout.join().unwrap_or_default();
        let _stderr_bytes = stderr.join().unwrap_or_default();
        if timed_out {
            return Ok(HarnessOutput::TimedOut);
        }
        Ok(HarnessOutput::Completed(
            String::from_utf8_lossy(&stdout_bytes).into_owned(),
        ))
    }
}

/// What a harness run produced.
enum HarnessOutput {
    Completed(String),
    TimedOut,
}

/// Spawn a thread draining `pipe` into a bounded buffer; excess bytes
/// are discarded (the caller's marker parse then fails closed).
fn take_pipe<R: Read + Send + 'static>(
    pipe: Option<R>,
    cap: usize,
) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut buffer = Vec::new();
        if let Some(mut pipe) = pipe {
            let mut chunk = [0_u8; 8192];
            loop {
                match pipe.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => {
                        if buffer.len() < cap {
                            let keep = (cap - buffer.len()).min(read);
                            buffer.extend_from_slice(&chunk[..keep]);
                        }
                    }
                }
            }
        }
        buffer
    })
}

/// Wait for `child` to exit by `deadline`; kill and reap on timeout.
/// Returns `true` when the deadline fired.
fn wait_with_deadline(child: &mut Child, deadline: Duration) -> Result<bool, TrainlabError> {
    let started = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            return Ok(false);
        }
        if started.elapsed() >= deadline {
            child.kill()?;
            child.wait()?;
            return Ok(true);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Classify captured harness stdout into an evaluation.
///
/// Pure: the last `PHLOW_TRAINLAB_RESULT` line wins; a missing or
/// malformed line is `Invalid` (fail closed), never a pass.
pub fn classify_harness_output(stdout: &str) -> Evaluation {
    let line = stdout
        .lines()
        .rev()
        .find(|line| line.starts_with(RESULT_MARKER));
    let Some(line) = line else {
        return Evaluation::invalid("no harness result line");
    };
    let payload = &line[RESULT_MARKER.len()..];
    let Ok(result) = serde_json::from_str::<HarnessResult>(payload) else {
        return Evaluation::invalid("malformed harness result");
    };
    let status = match result.status.as_str() {
        "passed" => EvalStatus::Passed,
        "failed" => EvalStatus::Failed,
        _ => EvalStatus::Invalid,
    };
    let mut message = result.message;
    if message.chars().count() > MESSAGE_CHARS_MAX {
        message = message.chars().take(MESSAGE_CHARS_MAX).collect();
    }
    Evaluation {
        status,
        tests_passed: result.tests_passed,
        tests_total: result.tests_total,
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::{Split, frozen_tasks};

    fn dev_task() -> CodingTask {
        frozen_tasks(Split::Dev, 1)
            .expect("dev tasks")
            .into_iter()
            .next()
            .expect("one task")
    }

    fn acknowledged_executor() -> Executor {
        Executor::new(ExecutorConfig {
            acknowledge_code_execution: true,
            ..ExecutorConfig::default()
        })
        .expect("executor")
    }

    fn python3_available() -> bool {
        Command::new("python3")
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    #[test]
    fn unacknowledged_execution_is_an_error() {
        // Adversarial: without the operator acknowledgment nothing
        // may execute, even a correct completion.
        let executor = Executor::new(ExecutorConfig::default()).expect("executor");
        let task = dev_task();
        let result = executor.evaluate(&task, &task.reference_completion);
        assert!(matches!(result, Err(TrainlabError::Executor(_))));
    }

    #[test]
    fn classification_fails_closed() {
        assert_eq!(
            classify_harness_output("garbage\n").status,
            EvalStatus::Invalid
        );
        assert_eq!(
            classify_harness_output("PHLOW_TRAINLAB_RESULT {nope}\n").status,
            EvalStatus::Invalid
        );
        let good = "noise\nPHLOW_TRAINLAB_RESULT {\"status\":\"passed\",\"message\":\"\",\"tests_passed\":3,\"tests_total\":3}\n";
        let evaluation = classify_harness_output(good);
        assert_eq!(evaluation.status, EvalStatus::Passed);
        assert_eq!(evaluation.pass_fraction(), 1.0);
    }

    #[test]
    fn oversized_completion_is_invalid_without_execution() {
        let executor = acknowledged_executor();
        let task = dev_task();
        let huge = " ".repeat(COMPLETION_BYTES_MAX + 1);
        let evaluation = executor.evaluate(&task, &huge).expect("evaluation");
        assert_eq!(evaluation.status, EvalStatus::Invalid);
    }

    #[test]
    fn reference_completion_passes_with_real_python() {
        if !python3_available() {
            eprintln!("SKIP: python3 not available for executor integration test");
            return;
        }
        let executor = acknowledged_executor();
        let task = dev_task();
        let evaluation = executor
            .evaluate(&task, &task.reference_completion)
            .expect("evaluation");
        assert_eq!(evaluation.status, EvalStatus::Passed, "{evaluation:?}");
        assert_eq!(evaluation.tests_passed, evaluation.tests_total);
    }

    #[test]
    fn wrong_and_broken_completions_classify() {
        if !python3_available() {
            eprintln!("SKIP: python3 not available for executor integration test");
            return;
        }
        let executor = acknowledged_executor();
        let task = dev_task();
        // Valid but wrong: returns the input unchanged.
        let wrong = executor.evaluate(&task, "\n    return x\n").expect("eval");
        assert_eq!(wrong.status, EvalStatus::Failed);
        // Syntax error.
        let broken = executor
            .evaluate(&task, "\n    return x +\n")
            .expect("eval");
        assert_eq!(broken.status, EvalStatus::Invalid);
        // Empty body: the def block is missing entirely, so the
        // source does not compile (IndentationError) -> invalid.
        let missing = executor.evaluate(&task, "\n").expect("eval");
        assert_eq!(missing.status, EvalStatus::Invalid);
    }

    #[test]
    fn infinite_loop_hits_the_deadline() {
        if !python3_available() {
            eprintln!("SKIP: python3 not available for executor integration test");
            return;
        }
        // Adversarial: a completion that never returns must be cut
        // off by the deadline and classified invalid, promptly.
        let executor = Executor::new(ExecutorConfig {
            deadline: Duration::from_millis(300),
            acknowledge_code_execution: true,
            ..ExecutorConfig::default()
        })
        .expect("executor");
        let task = dev_task();
        let started = Instant::now();
        let evaluation = executor
            .evaluate(&task, "\n    while True:\n        pass\n")
            .expect("evaluation");
        assert_eq!(evaluation.status, EvalStatus::Invalid);
        assert!(started.elapsed() < Duration::from_secs(10));
    }
}
