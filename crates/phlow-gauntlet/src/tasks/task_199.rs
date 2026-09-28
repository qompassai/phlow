// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Task 199 — verify-after-act (rust, V/A).
//!
//! The seam is the doctrine's hard rule, adapted from Ghostex's
//! `skills/ghostex-cli`: a mutating command must report its changed
//! fields AND verify them with a fresh re-read; when the backend lies
//! ("success" without writing), verification catches it, the command
//! exits non-zero, and the failure is typed as
//! `CommandError::VerifyFailed`. A false success is never printed.
//!
//! Honest scope: phlow-cli has no mutating command with an
//! injectable backend (its mutations are check/report shaped, with no
//! verify-after-act step and no way to inject a fault), so the
//! doctrine mechanism is proven here as a small, explicit state
//! machine at toy scale — `execute_verified` — rather than by
//! pretending the real CLI already honors it. The two cases are the
//! honest backend (V) and the fault-injected no-write backend (A).

use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{Ctx, TaskKind, TaskOutcome};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Task id.
pub const ID: &str = "task-199";
/// Task name.
pub const NAME: &str = "verify-after-act";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 1 validation, 1 adversarial.
pub const CASES: [&str; 2] = ["honest_backend_verified", "lying_backend_caught"];

/// Typed command failures. The doctrine's refusal is the third
/// variant: the backend claimed success but a fresh re-read of the
/// mutated state disagrees with what was reported.
#[derive(Debug, PartialEq, Eq)]
pub enum CommandError {
    /// The mutation itself failed.
    ApplyFailed {
        /// Which command was running.
        command: String,
        /// Why the mutation failed.
        reason: String,
    },
    /// The verification re-read failed (the world is unreachable).
    ReadFailed {
        /// Which command was running.
        command: String,
        /// Why the re-read failed.
        reason: String,
    },
    /// The backend reported success but the fresh re-read disagrees.
    /// This is the lie the doctrine exists to catch: it must exit
    /// non-zero and must never be printed as a success.
    VerifyFailed {
        /// Which command was running.
        command: String,
        /// The field that disagrees.
        field: String,
        /// What the command reported.
        reported: String,
        /// What the fresh re-read actually found (`<absent>` when the
        /// field is missing entirely).
        actual: String,
    },
}

/// Exit-code mapping for a verified command: success is 0, every
/// failure — including `VerifyFailed` — is 1 (the command ran but the
/// outcome is not ok, mirroring phlow-cli's exit-1 convention). A
/// verified failure is never 0.
pub fn exit_code_for(result: &Result<String, CommandError>) -> i32 {
    match result {
        Ok(_) => 0,
        Err(_) => 1,
    }
}

/// A mutating command with a separately observable state: `apply`
/// performs the mutation and reports the changed fields; `read_state`
/// re-reads the state from scratch (the verification read). The two
/// must go through different paths — otherwise verification would
/// just re-read the command's own memory.
pub trait MutatingCommand {
    /// Command name, used in reports and in `CommandError`.
    fn name(&self) -> &str;
    /// Perform the mutation; return the changed fields as reported to
    /// the user.
    fn apply(&mut self) -> Result<Vec<(String, String)>, CommandError>;
    /// Fresh re-read of the mutated state, independent of `apply`.
    fn read_state(&self) -> Result<Vec<(String, String)>, CommandError>;
}

/// Run a mutating command with verify-after-act: apply, then re-read
/// the state from scratch and compare every reported field against
/// the fresh read. On success the returned text lists the changed
/// fields plus the verification line. On mismatch the error is
/// `CommandError::VerifyFailed` and no success text exists.
pub fn execute_verified<C: MutatingCommand>(cmd: &mut C) -> Result<String, CommandError> {
    let reported = cmd.apply()?;
    let actual = cmd.read_state()?;
    let actual_map: HashMap<&str, &str> = actual
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    for (field, value) in &reported {
        match actual_map.get(field.as_str()) {
            Some(found) if found == &value.as_str() => {}
            Some(found) => {
                return Err(CommandError::VerifyFailed {
                    command: cmd.name().to_string(),
                    field: field.clone(),
                    reported: value.clone(),
                    actual: (*found).to_string(),
                });
            }
            None => {
                return Err(CommandError::VerifyFailed {
                    command: cmd.name().to_string(),
                    field: field.clone(),
                    reported: value.clone(),
                    actual: "<absent>".to_string(),
                });
            }
        }
    }
    let mut out = String::new();
    for (field, value) in &reported {
        out.push_str(&format!("changed {field}={value}\n"));
    }
    out.push_str(&format!(
        "verified: {} fields match fresh re-read\n",
        reported.len()
    ));
    Ok(out)
}

/// The honest backend: `apply` writes `field=value` lines to a state
/// file; `read_state` reads the file back from disk. Both go through
/// the filesystem, so the verification read is genuinely fresh.
pub struct FileStateCommand {
    name: String,
    state_file: PathBuf,
    pending: Vec<(String, String)>,
}

impl FileStateCommand {
    /// Build a command that will write `pending` fields to
    /// `state_file` on `apply`.
    pub fn new(name: &str, state_file: PathBuf, pending: Vec<(String, String)>) -> Self {
        Self {
            name: name.to_string(),
            state_file,
            pending,
        }
    }

    fn read_pairs(path: &Path) -> Result<Vec<(String, String)>, CommandError> {
        let text = std::fs::read_to_string(path).map_err(|e| CommandError::ReadFailed {
            command: "read_state".to_string(),
            reason: format!("cannot read {}: {e}", path.display()),
        })?;
        let mut pairs = Vec::new();
        for line in text.lines() {
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| CommandError::ReadFailed {
                    command: "read_state".to_string(),
                    reason: format!("malformed state line: {line}"),
                })?;
            pairs.push((key.to_string(), value.to_string()));
        }
        Ok(pairs)
    }
}

impl MutatingCommand for FileStateCommand {
    fn name(&self) -> &str {
        &self.name
    }

    fn apply(&mut self) -> Result<Vec<(String, String)>, CommandError> {
        let mut text = String::new();
        for (field, value) in &self.pending {
            text.push_str(&format!("{field}={value}\n"));
        }
        std::fs::write(&self.state_file, text).map_err(|e| CommandError::ApplyFailed {
            command: self.name.clone(),
            reason: format!("cannot write {}: {e}", self.state_file.display()),
        })?;
        Ok(self.pending.clone())
    }

    fn read_state(&self) -> Result<Vec<(String, String)>, CommandError> {
        Self::read_pairs(&self.state_file)
    }
}

/// The fault-injected backend: `apply` reports success without
/// writing anything (the lie), while `read_state` reads the real
/// (unmutated) state file. The doctrine's verification step must
/// catch the divergence.
pub struct LyingBackend {
    name: String,
    reported: Vec<(String, String)>,
    actual: Vec<(String, String)>,
}

impl LyingBackend {
    /// Build a backend that will claim `reported` while the world
    /// actually holds `actual`.
    pub fn new(name: &str, reported: Vec<(String, String)>, actual: Vec<(String, String)>) -> Self {
        Self {
            name: name.to_string(),
            reported,
            actual,
        }
    }
}

impl MutatingCommand for LyingBackend {
    fn name(&self) -> &str {
        &self.name
    }

    fn apply(&mut self) -> Result<Vec<(String, String)>, CommandError> {
        // The injected fault: report success, write nothing.
        Ok(self.reported.clone())
    }

    fn read_state(&self) -> Result<Vec<(String, String)>, CommandError> {
        Ok(self.actual.clone())
    }
}

fn fixture(what: &str, detail: String) -> TaskDriverError {
    TaskDriverError::Fixture {
        what: what.to_string(),
        detail: format!("task-199: {detail}"),
    }
}

/// The script-level check: reported state equals actual state, read
/// back independently of the command (a fresh file read, not the
/// command's own memory).
fn script_reread_matches(
    state_file: &Path,
    expected: &[(&str, &str)],
    failures: &mut Vec<String>,
    evidence: &mut Vec<String>,
) {
    match FileStateCommand::read_pairs(state_file) {
        Ok(actual) => {
            for (field, value) in expected {
                let found = actual.iter().find(|(k, _)| k == field);
                if found.map(|(_, v)| v.as_str()) != Some(*value) {
                    failures.push(format!(
                        "script re-read: field '{field}' is {found:?}, want '{value}'"
                    ));
                }
            }
            evidence.push(format!("script re-read of state file: {actual:?}"));
        }
        Err(e) => failures.push(format!("script re-read failed: {e:?}")),
    }
}

/// V: the honest backend. `execute_verified` must succeed, print one
/// `changed <field>=<value>` line per field plus the verification
/// line — and a separate script-level re-read of the state file must
/// show the reported state equals the actual state.
fn case_honest_backend_verified() -> Result<CaseReport, TaskDriverError> {
    let dir = std::env::temp_dir().join(format!("phlow-wave32-199v-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)
        .map_err(|e| fixture("temp", format!("cannot create temp dir: {e}")))?;
    let state_file = dir.join("state.txt");
    let mut cmd = FileStateCommand::new(
        "set-brightness",
        state_file.clone(),
        vec![
            ("brightness".to_string(), "80".to_string()),
            ("mode".to_string(), "warm".to_string()),
        ],
    );
    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    let result = execute_verified(&mut cmd);
    let code = exit_code_for(&result);
    if code != 0 {
        failures.push(format!("honest backend exited {code}, want 0"));
    }
    match &result {
        Ok(text) => {
            for line in ["changed brightness=80", "changed mode=warm"] {
                if !text.contains(line) {
                    failures.push(format!("success output missing '{line}'"));
                }
            }
            if !text.contains("verified: 2 fields match fresh re-read") {
                failures.push("success output missing the verification line".to_string());
            }
            evidence.push(format!("command output:\n{text}"));
        }
        Err(e) => failures.push(format!("honest backend failed verification: {e:?}")),
    }
    script_reread_matches(
        &state_file,
        &[("brightness", "80"), ("mode", "warm")],
        &mut failures,
        &mut evidence,
    );
    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "exit_code": code,
            "fields": 2,
            "backend": "honest-file",
        }),
        evidence,
    );
    if !failures.is_empty() {
        report.passed = false;
        report.failures = failures;
    }
    Ok(report)
}

/// A: the fault-injected backend reports success but writes nothing.
/// Verification must catch it: `execute_verified` returns
/// `CommandError::VerifyFailed`, the exit code is non-zero, and no
/// success text is ever produced.
fn case_lying_backend_caught() -> Result<CaseReport, TaskDriverError> {
    let mut cmd = LyingBackend::new(
        "set-brightness",
        vec![
            ("brightness".to_string(), "80".to_string()),
            ("mode".to_string(), "warm".to_string()),
        ],
        // The world was never mutated: brightness stayed at 20 and
        // `mode` was never written at all.
        vec![("brightness".to_string(), "20".to_string())],
    );
    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    let result = execute_verified(&mut cmd);
    let code = exit_code_for(&result);
    if code == 0 {
        failures.push("lying backend exited 0: a false success was printed".to_string());
    }
    match &result {
        Ok(text) => failures.push(format!(
            "lying backend returned success text (never print a false success): {text}"
        )),
        Err(CommandError::VerifyFailed {
            command,
            field,
            reported,
            actual,
        }) => {
            evidence.push(format!(
                "caught: {command}: field '{field}' reported '{reported}' but fresh re-read found '{actual}'"
            ));
            if field != "brightness" || reported != "80" || actual != "20" {
                failures.push(format!(
                    "VerifyFailed named the wrong field/values: {field}={reported} vs {actual}"
                ));
            }
        }
        Err(other) => failures.push(format!("wrong error type: {other:?}, want VerifyFailed")),
    }
    evidence.push(format!("exit code for the caught lie: {code} (non-zero)"));
    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "exit_code": code,
            "error": "VerifyFailed",
            "backend": "lying-no-write",
        }),
        evidence,
    );
    if !failures.is_empty() {
        report.passed = false;
        report.failures = failures;
    }
    Ok(report)
}

/// Run one driver case by name.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "honest_backend_verified" => case_honest_backend_verified(),
        "lying_backend_caught" => case_lying_backend_caught(),
        _ => Err(fixture("case", format!("unknown case '{case}'"))),
    }
}

/// Task-level entry for the gauntlet runner: the headline case is the
/// honest-backend verification.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    match run_case(CASES[0]) {
        Ok(r) if r.passed => TaskOutcome::Pass {
            evidence: r.evidence,
        },
        Ok(r) => TaskOutcome::Fail {
            where_: "task-199".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-199".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
