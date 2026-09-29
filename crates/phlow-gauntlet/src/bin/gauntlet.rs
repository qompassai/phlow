#![forbid(unsafe_code)]

//! `gauntlet` — CLI driver for the 200-task orchestration gauntlet (200 implemented).
//!
//! ```sh
//! gauntlet list
//! gauntlet run <task-id> [--nvim-bin PATH] [--diver-lua PATH] [--work-dir PATH]
//! gauntlet run-all [--report-dir DIR] [--nvim-bin PATH] [--diver-lua PATH] [--work-dir PATH]
//! gauntlet verify-claims <ledger.json>
//! ```
//!
//! Argument parsing is manual and bounded: four subcommands, four flags, no
//! external CLI framework. Unknown flags are rejected, never ignored.

use phlow_gauntlet::tasks::TaskEntry;
use phlow_gauntlet::{Ctx, GauntletError, TaskOutcome, TaskReport, bound_evidence, tasks};
use serde_json::{Map, Value};
use std::path::PathBuf;
use std::process::ExitCode;

/// Usage text printed on bad invocation.
const USAGE: &str = "usage:\n  gauntlet list\n  gauntlet run <task-id> [--nvim-bin PATH] [--diver-lua PATH] [--work-dir PATH]\n  gauntlet run-all [--report-dir DIR] [--nvim-bin PATH] [--diver-lua PATH] [--work-dir PATH]\n  gauntlet verify-claims <ledger.json>";

/// Parsed CLI options. All paths are required for `run`/`run-all`: the
/// gauntlet never guesses where Matt's editor or config live.
struct Opts {
    nvim_bin: Option<PathBuf>,
    diver_lua: Option<PathBuf>,
    work_dir: Option<PathBuf>,
    report_dir: Option<PathBuf>,
}

fn parse_flags(args: &[String]) -> Result<Opts, String> {
    let mut opts = Opts {
        nvim_bin: None,
        diver_lua: None,
        work_dir: None,
        report_dir: None,
    };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--nvim-bin" => {
                i += 1;
                opts.nvim_bin = Some(flag_value(args, i, "--nvim-bin")?);
            }
            "--diver-lua" => {
                i += 1;
                opts.diver_lua = Some(flag_value(args, i, "--diver-lua")?);
            }
            "--work-dir" => {
                i += 1;
                opts.work_dir = Some(flag_value(args, i, "--work-dir")?);
            }
            "--report-dir" => {
                i += 1;
                opts.report_dir = Some(flag_value(args, i, "--report-dir")?);
            }
            other => return Err(format!("unknown flag '{other}'\n{USAGE}")),
        }
        i += 1;
    }
    Ok(opts)
}

fn flag_value(args: &[String], i: usize, flag: &str) -> Result<PathBuf, String> {
    args.get(i)
        .map(PathBuf::from)
        .ok_or_else(|| format!("flag '{flag}' needs a value\n{USAGE}"))
}

fn build_ctx(opts: &Opts) -> Result<Ctx, GauntletError> {
    let nvim_bin = opts
        .nvim_bin
        .clone()
        .ok_or(GauntletError::EmptyField { field: "nvim_bin" })?;
    let diver_lua = opts.diver_lua.clone().ok_or(GauntletError::EmptyField {
        field: "diver_lua_dir",
    })?;
    let work_dir = opts
        .work_dir
        .clone()
        .unwrap_or_else(|| std::env::temp_dir().join("phlow-gauntlet"));
    Ctx::new(nvim_bin, diver_lua, work_dir)
}

/// Render one report as a JSON object. Built from a `Map` so the shape is
/// fixed and every string is escaped by `serde_json`, not by hand.
fn report_json(r: &TaskReport) -> String {
    let mut m = Map::new();
    m.insert("id".to_string(), Value::from(r.id));
    m.insert("name".to_string(), Value::from(r.name));
    m.insert("kind".to_string(), Value::from(r.kind.to_string()));
    m.insert("duration_ms".to_string(), Value::from(r.duration_ms));
    match &r.outcome {
        TaskOutcome::Pass { evidence } => {
            m.insert("outcome".to_string(), Value::from("pass"));
            m.insert(
                "evidence".to_string(),
                Value::from(bound_evidence(evidence.clone())),
            );
        }
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => {
            m.insert("outcome".to_string(), Value::from("fail"));
            m.insert("where".to_string(), Value::from(where_.clone()));
            m.insert("how".to_string(), Value::from(how.clone()));
            m.insert(
                "evidence".to_string(),
                Value::from(bound_evidence(evidence.clone())),
            );
        }
    }
    Value::Object(m).to_string()
}

fn cmd_list() -> ExitCode {
    let mut unwired = 0;
    for id in tasks::TASK_IDS {
        // Loud, never silent: a listed-but-unwired id is exactly how the
        // wave-28 summary shipped fiction. task_meta alone would hide it.
        if tasks::is_wired(id) {
            if let Some((name, kind)) = tasks::task_meta(id) {
                println!("{id}\t{kind}\t{name}");
            }
        } else {
            println!("{id}\tUNWIRED");
            unwired += 1;
        }
    }
    if unwired > 0 {
        eprintln!("gauntlet: {unwired} listed task(s) have no dispatch/metadata arms");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

fn cmd_run(id: &str, opts: &Opts) -> ExitCode {
    let ctx = match build_ctx(opts) {
        Ok(ctx) => ctx,
        Err(e) => {
            eprintln!("gauntlet: {e}");
            return ExitCode::from(2);
        }
    };
    match tasks::run_task(id, &ctx) {
        Ok(report) => {
            let passed = report.passed();
            println!("{}", report_json(&report));
            if passed {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        Err(e) => {
            eprintln!("gauntlet: {e}");
            ExitCode::from(2)
        }
    }
}

fn cmd_run_all(opts: &Opts) -> ExitCode {
    let ctx = match build_ctx(opts) {
        Ok(ctx) => ctx,
        Err(e) => {
            eprintln!("gauntlet: {e}");
            return ExitCode::from(2);
        }
    };
    let mut reports: Vec<String> = Vec::new();
    let mut failed: Vec<&str> = Vec::new();
    for id in tasks::TASK_IDS {
        match tasks::run_task(id, &ctx) {
            Ok(report) => {
                if !report.passed() {
                    failed.push(id);
                }
                // One JSON object per line: streaming, bounded memory.
                println!("{}", report_json(&report));
                reports.push(report_json(&report));
            }
            Err(e) => {
                eprintln!("gauntlet: task {id}: {e}");
                failed.push(id);
            }
        }
    }
    let failed_json = Value::from(failed.clone()).to_string();
    let summary = format!(
        "{{\"tasks\":[{}],\"failed\":{failed_json}}}",
        reports.join(",")
    );
    if let Some(dir) = &opts.report_dir {
        let path = dir.join("gauntlet-report.json");
        if let Err(e) = std::fs::create_dir_all(dir).and_then(|_| std::fs::write(&path, &summary)) {
            eprintln!("gauntlet: cannot write report: {e}");
            return ExitCode::from(2);
        }
        eprintln!("gauntlet: report written to {}", path.display());
    }
    if failed.is_empty() {
        ExitCode::SUCCESS
    } else {
        eprintln!("gauntlet: {} task(s) failed", failed.len());
        ExitCode::from(1)
    }
}

/// Maximum ledger file size: 1 MiB. Ledgers are small claim lists; anything
/// larger is hostile or a wrong file, and is rejected before allocation.
const LEDGER_BYTES_MAX: u64 = 1_048_576;

/// Maximum claims per ledger: bounds the duplicate-check set and the
/// verification loop.
const CLAIMS_MAX: usize = 1024;

/// Full SHA hex length. The ledger must name the exact commit in full:
/// prefixes are ambiguous and fail closed.
const COMMIT_SHA_LEN: usize = 40;

/// Typed ledger rejection reasons. Every variant is a fail-closed
/// verification failure: the ledger does not advance the program.
#[derive(Debug, Clone, PartialEq, Eq)]
enum LedgerError {
    /// `wave` missing, not a string, or blank.
    MissingWave,
    /// `commit` missing or not a string.
    MissingCommit,
    /// `commit` is not a 40-character lowercase hex SHA.
    BadCommitShape,
    /// `claims` missing or not an array.
    BadClaimsShape,
    /// `claims` is empty.
    EmptyClaims,
    /// `claims` exceeds [`CLAIMS_MAX`].
    TooManyClaims { count: usize },
    /// A claim is not a string.
    NonStringClaim { index: usize },
    /// A task id is claimed twice.
    DuplicateClaim { id: String },
    /// A claimed id is not carried by the registry: unknown id or dispatch
    /// arm removed --- one table, one rejection. (The wave-28 ledger listed
    /// tasks whose dispatch arms were never written.)
    UnknownClaim { id: String },
}

impl std::fmt::Display for LedgerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LedgerError::MissingWave => write!(f, "ledger needs a non-empty 'wave' string"),
            LedgerError::MissingCommit => write!(f, "ledger needs a 'commit' string"),
            LedgerError::BadCommitShape => write!(
                f,
                "ledger 'commit' must be a {COMMIT_SHA_LEN}-character lowercase hex SHA"
            ),
            LedgerError::BadClaimsShape => write!(f, "ledger needs a 'claims' array of task ids"),
            LedgerError::EmptyClaims => write!(f, "ledger must claim at least one task"),
            LedgerError::TooManyClaims { count } => {
                write!(
                    f,
                    "ledger claims {count} tasks, over the {CLAIMS_MAX} limit"
                )
            }
            LedgerError::NonStringClaim { index } => {
                write!(f, "ledger claim at index {index} is not a string")
            }
            LedgerError::DuplicateClaim { id } => write!(f, "duplicate claim '{id}'"),
            LedgerError::UnknownClaim { id } => write!(
                f,
                "claim '{id}' is not in the task registry: unknown id or dispatch arm removed"
            ),
        }
    }
}

/// A ledger that passed every static check. `commit` is borrowed from the
/// parsed JSON: the caller keeps `ledger` alive through the ancestor check.
#[derive(Debug)]
struct VerifiedLedger<'a> {
    claim_count: usize,
    commit: &'a str,
}

/// Read the ledger file with an explicit size bound. The bound is checked
/// on the file metadata BEFORE `read_to_string` allocates.
fn read_ledger_bytes(path: &str) -> Result<String, String> {
    let len = std::fs::metadata(path)
        .map_err(|e| format!("gauntlet: cannot stat ledger '{path}': {e}"))?
        .len();
    if len > LEDGER_BYTES_MAX {
        return Err(format!(
            "gauntlet: ledger '{path}' is {len} bytes, over the {LEDGER_BYTES_MAX}-byte limit"
        ));
    }
    std::fs::read_to_string(path).map_err(|e| format!("gauntlet: cannot read ledger '{path}': {e}"))
}

/// Load and parse the ledger: size-bounded read, then strict JSON.
fn load_ledger(path: &str) -> Result<Value, String> {
    let text = read_ledger_bytes(path)?;
    serde_json::from_str(&text).map_err(|e| format!("gauntlet: ledger is not valid JSON: {e}"))
}

/// True for exactly 40 lowercase hex chars: the full commit SHA, no
/// prefixes, no uppercase, no truncation.
fn commit_shape_ok(commit: &str) -> bool {
    commit.len() == COMMIT_SHA_LEN
        && commit
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Gate zero: check a parsed claim ledger against a task registry.
///
/// `registry` is the single authority: a claim is accepted iff the
/// registry carries the id. Tests pass a tampered registry (an arm
/// removed) to simulate the wave-28 shape; the binary passes
/// `&tasks::TASKS`.
///
/// Ledger shape:
///   {"wave": "wave-32", "commit": "<40-char sha>", "claims": ["task-197", ...]}
///
/// All checks fail closed. Returns the verified claim count plus the
/// commit for the caller's ancestor check.
fn verify_ledger<'a>(
    ledger: &'a Value,
    registry: &[TaskEntry],
) -> Result<VerifiedLedger<'a>, LedgerError> {
    let wave = ledger.get("wave").and_then(Value::as_str).unwrap_or("");
    if wave.trim().is_empty() {
        return Err(LedgerError::MissingWave);
    }
    let commit = match ledger.get("commit").and_then(Value::as_str) {
        Some(commit) => commit,
        None => return Err(LedgerError::MissingCommit),
    };
    if !commit_shape_ok(commit) {
        return Err(LedgerError::BadCommitShape);
    }
    let claims = match ledger.get("claims").and_then(Value::as_array) {
        Some(claims) => claims,
        None => return Err(LedgerError::BadClaimsShape),
    };
    if claims.is_empty() {
        return Err(LedgerError::EmptyClaims);
    }
    if claims.len() > CLAIMS_MAX {
        return Err(LedgerError::TooManyClaims {
            count: claims.len(),
        });
    }
    let mut seen = std::collections::HashSet::with_capacity(claims.len());
    for (index, claim) in claims.iter().enumerate() {
        let id = match claim.as_str() {
            Some(id) => id,
            None => return Err(LedgerError::NonStringClaim { index }),
        };
        if !seen.insert(id) {
            return Err(LedgerError::DuplicateClaim { id: id.to_string() });
        }
        if !registry.iter().any(|entry| entry.id == id) {
            return Err(LedgerError::UnknownClaim { id: id.to_string() });
        }
    }
    Ok(VerifiedLedger {
        claim_count: claims.len(),
        commit,
    })
}

/// The ledger names the exact code commit the producer's claims were
/// written against. The verifier's checkout must contain that commit
/// (ancestor check, not equality: the ledger itself lives in a child
/// commit, since a commit hash covers the ledger and can never name its
/// own tree). Fail closed on any git failure. The commit SHAPE is checked
/// by `verify_ledger` before this runs: git is never spawned for garbage.
fn ledger_commit_ok(commit: &str) -> Result<(), String> {
    let status = std::process::Command::new("git")
        .args(["merge-base", "--is-ancestor", commit, "HEAD"])
        .status()
        .map_err(|e| format!("git unavailable ({e}); cannot bind ledger to a commit"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "ledger names commit '{commit}' which is not an ancestor of HEAD: \
             check out the claimed tree first"
        ))
    }
}

/// Exit 0 = verified, 1 = ledger rejected (content or policy failure),
/// 2 = bad invocation or unreadable input.
fn cmd_verify_claims(path: &str) -> ExitCode {
    let ledger = match load_ledger(path) {
        Ok(ledger) => ledger,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let verified = match verify_ledger(&ledger, &tasks::TASKS) {
        Ok(verified) => verified,
        Err(e) => {
            eprintln!("gauntlet: ledger rejected: {e}");
            return ExitCode::from(1);
        }
    };
    if let Err(e) = ledger_commit_ok(verified.commit) {
        eprintln!("gauntlet: ledger rejected: {e}");
        return ExitCode::from(1);
    }
    println!(
        "gauntlet: {} claim(s) verified: every claimed task is carried by the registry",
        verified.claim_count
    );
    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((sub, rest)) = args.split_first() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    match sub.as_str() {
        "list" => {
            if !rest.is_empty() {
                eprintln!("gauntlet list takes no flags\n{USAGE}");
                return ExitCode::from(2);
            }
            cmd_list()
        }
        "run" => {
            let Some((id, flags)) = rest.split_first() else {
                eprintln!("gauntlet run needs a task id\n{USAGE}");
                return ExitCode::from(2);
            };
            match parse_flags(flags) {
                Ok(opts) => cmd_run(id, &opts),
                Err(e) => {
                    eprintln!("gauntlet: {e}");
                    ExitCode::from(2)
                }
            }
        }
        "run-all" => match parse_flags(rest) {
            Ok(opts) => cmd_run_all(&opts),
            Err(e) => {
                eprintln!("gauntlet: {e}");
                ExitCode::from(2)
            }
        },
        "verify-claims" => {
            let Some((path, flags)) = rest.split_first() else {
                eprintln!("gauntlet verify-claims needs a ledger path\n{USAGE}");
                return ExitCode::from(2);
            };
            if !flags.is_empty() {
                eprintln!("gauntlet verify-claims takes no flags\n{USAGE}");
                return ExitCode::from(2);
            }
            cmd_verify_claims(path)
        }
        other => {
            eprintln!("gauntlet: unknown subcommand '{other}'\n{USAGE}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod ledger_tests {
    use super::*;
    use serde_json::json;

    /// Build a ledger object with explicit parts (missing parts are left
    /// out entirely, unlike `json!` literals that always include the key).
    fn ledger_with(wave: Option<Value>, commit: Option<Value>, claims: Option<Value>) -> Value {
        let mut m = Map::new();
        if let Some(wave) = wave {
            m.insert("wave".to_string(), wave);
        }
        if let Some(commit) = commit {
            m.insert("commit".to_string(), commit);
        }
        if let Some(claims) = claims {
            m.insert("claims".to_string(), claims);
        }
        Value::Object(m)
    }

    /// A well-shaped commit; never an ancestor, so it only exercises shape.
    fn shape_commit() -> Value {
        Value::from("a".repeat(COMMIT_SHA_LEN))
    }

    fn good_ledger() -> Value {
        ledger_with(
            Some(json!("wave-32")),
            Some(shape_commit()),
            Some(json!(["task-01", "task-170", "task-200"])),
        )
    }

    fn claims_of(ids: &[&str]) -> Value {
        Value::from(ids.iter().map(|id| Value::from(*id)).collect::<Vec<_>>())
    }

    /// Validation: a well-formed ledger with real registry ids verifies.
    #[test]
    fn valid_ledger_accepted() {
        let ledger = good_ledger();
        let verified = verify_ledger(&ledger, &tasks::TASKS).expect("valid ledger must verify");
        assert_eq!(verified.claim_count, 3);
        assert_eq!(verified.commit, "a".repeat(COMMIT_SHA_LEN));
    }

    /// Validation: the full 200-claim ledger verifies against the registry.
    #[test]
    fn full_200_claim_ledger_accepted() {
        let all: Vec<Value> = tasks::TASK_IDS.iter().map(|id| Value::from(*id)).collect();
        let ledger = ledger_with(
            Some(json!("wave-32")),
            Some(shape_commit()),
            Some(Value::from(all)),
        );
        let verified = verify_ledger(&ledger, &tasks::TASKS).expect("200-claim ledger must verify");
        assert_eq!(verified.claim_count, phlow_gauntlet::TASK_COUNT_MAX);
    }

    /// Validation: a real ancestor commit passes the git binding. HEAD is
    /// always an ancestor of itself.
    #[test]
    fn head_commit_passes_ancestor_check() {
        let out = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .output()
            .expect("git must run for this test");
        assert!(out.status.success(), "git rev-parse HEAD failed");
        let sha = String::from_utf8(out.stdout).expect("HEAD must be UTF-8");
        let sha = sha.trim();
        assert_eq!(sha.len(), COMMIT_SHA_LEN, "unexpected HEAD shape: {sha}");
        ledger_commit_ok(sha).expect("HEAD must be an ancestor of HEAD");
    }

    /// Validation: a ledger at exactly the size limit is still read.
    #[test]
    fn ledger_at_size_limit_is_read() {
        let dir =
            std::env::temp_dir().join(format!("gauntlet-ledger-limit-{}", std::process::id()));
        let mut text = format!(
            "{{\"wave\":\"w\",\"commit\":\"{}\",\"claims\":[\"task-01\"]}}",
            "b".repeat(COMMIT_SHA_LEN)
        );
        while text.len() < LEDGER_BYTES_MAX as usize {
            text.push(' ');
        }
        assert_eq!(text.len(), LEDGER_BYTES_MAX as usize);
        std::fs::write(&dir, &text).expect("must write temp ledger");
        let result = read_ledger_bytes(dir.to_str().expect("temp path must be UTF-8"));
        std::fs::remove_file(&dir).ok();
        result.expect("ledger at exactly the limit must be read");
    }

    /// Adversarial: unknown ids are rejected even when everything else is
    /// well-formed.
    #[test]
    fn unknown_task_rejected() {
        let ledger = ledger_with(
            Some(json!("wave-32")),
            Some(shape_commit()),
            Some(claims_of(&["task-01", "task-999"])),
        );
        let err = verify_ledger(&ledger, &tasks::TASKS).expect_err("task-999 must be rejected");
        assert!(
            matches!(err, LedgerError::UnknownClaim { ref id } if id == "task-999"),
            "unexpected: {err}"
        );
    }

    /// Adversarial: a claim is a single-use ticket; doubles are rejected.
    #[test]
    fn duplicate_claim_rejected() {
        let ledger = ledger_with(
            Some(json!("wave-32")),
            Some(shape_commit()),
            Some(claims_of(&["task-01", "task-01"])),
        );
        let err = verify_ledger(&ledger, &tasks::TASKS).expect_err("duplicate must be rejected");
        assert!(
            matches!(err, LedgerError::DuplicateClaim { ref id } if id == "task-01"),
            "unexpected: {err}"
        );
    }

    /// Adversarial: an empty claims list verifies nothing.
    #[test]
    fn empty_claims_rejected() {
        let ledger = ledger_with(
            Some(json!("wave-32")),
            Some(shape_commit()),
            Some(json!([])),
        );
        let err = verify_ledger(&ledger, &tasks::TASKS).expect_err("empty claims must be rejected");
        assert!(matches!(err, LedgerError::EmptyClaims), "unexpected: {err}");
    }

    /// Adversarial: the wave-28 failure mode was a missing commit silently
    /// accepted. Missing is now a hard rejection.
    #[test]
    fn missing_commit_rejected() {
        let ledger = ledger_with(Some(json!("wave-32")), None, Some(claims_of(&["task-01"])));
        let err = verify_ledger(&ledger, &tasks::TASKS).expect_err("missing commit must fail");
        assert!(
            matches!(err, LedgerError::MissingCommit),
            "unexpected: {err}"
        );
    }

    /// Adversarial: malformed commits are rejected on shape, before git is
    /// ever spawned. Each entry: (label, commit value).
    #[test]
    fn malformed_commits_rejected() {
        let cases: Vec<(&str, String)> = vec![
            ("empty", String::new()),
            ("short", "abc".to_string()),
            ("one-short", "a".repeat(COMMIT_SHA_LEN - 1)),
            ("one-long", "a".repeat(COMMIT_SHA_LEN + 1)),
            ("uppercase", "A".repeat(COMMIT_SHA_LEN)),
            ("non-hex", format!("{}z", "a".repeat(COMMIT_SHA_LEN - 1))),
            (
                "whitespace-pad",
                format!(" {}", "a".repeat(COMMIT_SHA_LEN - 1)),
            ),
        ];
        for (label, commit) in cases {
            let ledger = ledger_with(
                Some(json!("wave-32")),
                Some(Value::from(commit)),
                Some(claims_of(&["task-01"])),
            );
            let err = verify_ledger(&ledger, &tasks::TASKS)
                .expect_err("malformed commit must be rejected");
            assert!(
                matches!(err, LedgerError::BadCommitShape),
                "{label}: unexpected {err}"
            );
        }
    }

    /// Adversarial: a well-shaped commit that is not an ancestor is
    /// rejected by the git binding (the all-zero SHA is the canonical
    /// non-ancestor).
    #[test]
    fn non_ancestor_commit_rejected() {
        let zero = "0".repeat(COMMIT_SHA_LEN);
        assert!(
            commit_shape_ok(&zero),
            "all-zero SHA must be well-shaped: shape is not the check that rejects it"
        );
        ledger_commit_ok(&zero).expect_err("all-zero SHA must not be an ancestor");
    }

    /// Adversarial: THE wave-28 regression shape. A task listed by the wave
    /// summary but with its dispatch arm removed from the registry must be
    /// rejected. The registry is one table, so "arm removed" is modeled by
    /// removing the entry; the control arm proves the claim itself is
    /// well-formed and accepted by the intact registry.
    #[test]
    fn removed_dispatch_arm_rejected() {
        let mut registry = tasks::TASKS.to_vec();
        let pos = registry
            .iter()
            .position(|entry| entry.id == "task-170")
            .expect("task-170 must be in the registry");
        registry.remove(pos);
        assert_eq!(registry.len(), phlow_gauntlet::TASK_COUNT_MAX - 1);

        let ledger = ledger_with(
            Some(json!("wave-28")),
            Some(shape_commit()),
            Some(claims_of(&["task-170"])),
        );
        verify_ledger(&ledger, &tasks::TASKS)
            .expect("control: intact registry must accept task-170");
        let err =
            verify_ledger(&ledger, &registry).expect_err("removed dispatch arm must be rejected");
        assert!(
            matches!(err, LedgerError::UnknownClaim { ref id } if id == "task-170"),
            "unexpected: {err}"
        );
    }

    /// Adversarial: same shape for a missing registry entry (metadata and
    /// dispatch are one record; either missing is the same rejection).
    #[test]
    fn missing_registry_entry_rejected() {
        let registry: Vec<TaskEntry> = tasks::TASKS
            .iter()
            .filter(|entry| entry.id != "task-01")
            .copied()
            .collect();
        assert_eq!(registry.len(), phlow_gauntlet::TASK_COUNT_MAX - 1);
        let ledger = ledger_with(
            Some(json!("wave-32")),
            Some(shape_commit()),
            Some(claims_of(&["task-01"])),
        );
        let err =
            verify_ledger(&ledger, &registry).expect_err("missing registry entry must be rejected");
        assert!(
            matches!(err, LedgerError::UnknownClaim { ref id } if id == "task-01"),
            "unexpected: {err}"
        );
    }

    /// Adversarial: the wave is required and must be non-blank.
    #[test]
    fn missing_or_blank_wave_rejected() {
        for (label, wave) in [
            ("missing", None),
            ("empty", Some(json!(""))),
            ("blank", Some(json!("   "))),
            ("non-string", Some(json!(32))),
        ] {
            let ledger = ledger_with(wave, Some(shape_commit()), Some(claims_of(&["task-01"])));
            let err = verify_ledger(&ledger, &tasks::TASKS).expect_err("wave must be required");
            assert!(
                matches!(err, LedgerError::MissingWave),
                "{label}: unexpected {err}"
            );
        }
    }

    /// Adversarial: claims must be an array, not a bare string.
    #[test]
    fn non_array_claims_rejected() {
        let ledger = ledger_with(
            Some(json!("wave-32")),
            Some(shape_commit()),
            Some(json!("task-01")),
        );
        let err = verify_ledger(&ledger, &tasks::TASKS).expect_err("non-array claims must fail");
        assert!(
            matches!(err, LedgerError::BadClaimsShape),
            "unexpected: {err}"
        );
    }

    /// Adversarial: every claim must be a string id.
    #[test]
    fn non_string_claim_rejected() {
        let ledger = ledger_with(
            Some(json!("wave-32")),
            Some(shape_commit()),
            Some(json!(["task-01", 42])),
        );
        let err = verify_ledger(&ledger, &tasks::TASKS).expect_err("non-string claim must fail");
        assert!(
            matches!(err, LedgerError::NonStringClaim { index: 1 }),
            "unexpected: {err}"
        );
    }

    /// Adversarial: the claim list is bounded at CLAIMS_MAX.
    #[test]
    fn too_many_claims_rejected() {
        let many: Vec<Value> = (0..=CLAIMS_MAX).map(|_| Value::from("task-01")).collect();
        assert_eq!(many.len(), CLAIMS_MAX + 1);
        let ledger = ledger_with(
            Some(json!("wave-32")),
            Some(shape_commit()),
            Some(Value::from(many)),
        );
        let err = verify_ledger(&ledger, &tasks::TASKS).expect_err("over-limit claims must fail");
        assert!(
            matches!(err, LedgerError::TooManyClaims { count } if count == CLAIMS_MAX + 1),
            "unexpected: {err}"
        );
    }

    /// Adversarial: garbage bytes are not a ledger.
    #[test]
    fn malformed_json_rejected() {
        let dir = std::env::temp_dir().join(format!("gauntlet-ledger-bad-{}", std::process::id()));
        std::fs::write(&dir, "{not json").expect("must write temp ledger");
        let result = load_ledger(dir.to_str().expect("temp path must be UTF-8"));
        std::fs::remove_file(&dir).ok();
        let err = result.expect_err("malformed JSON must be rejected");
        assert!(err.contains("not valid JSON"), "unexpected: {err}");
    }

    /// Adversarial: the size bound is enforced BEFORE allocation, so an
    /// oversized file is rejected without being read.
    #[test]
    fn oversized_ledger_rejected() {
        let dir = std::env::temp_dir().join(format!("gauntlet-ledger-huge-{}", std::process::id()));
        let filler = "x".repeat(LEDGER_BYTES_MAX as usize + 1);
        std::fs::write(&dir, &filler).expect("must write temp ledger");
        let result = read_ledger_bytes(dir.to_str().expect("temp path must be UTF-8"));
        std::fs::remove_file(&dir).ok();
        let err = result.expect_err("oversized ledger must be rejected");
        assert!(err.contains("over the"), "unexpected: {err}");
    }
}
