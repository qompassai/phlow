//! The fleet run contract (INDEX §2.1), as a reference implementation.
//!
//! Every operation that can outlive a tool call is a **run**. A run is a
//! directory under the server's state root, named by its `run_id`, holding
//! three small files:
//!
//! - `run.json` — the [`RunRecord`]: run id, absolute deadline, heartbeat
//!   interval, and the run's process-group id when it has one. Written
//!   once at start; the deadline is never extended by the run itself.
//! - `heartbeat` — a file whose content is the last heartbeat time in
//!   Unix milliseconds, rewritten atomically at the stated interval. A
//!   heartbeat older than [`HEARTBEAT_STALE_FACTOR`] intervals means the
//!   run is dead even if its deadline has not passed.
//! - `done.json` — the [`DoneMarker`]: written last, atomically (sibling
//!   temp file + rename), with the exit state (`done`, `failed`, or
//!   `abandoned`) and the SHA-256 hash of every artifact the run claims.
//!
//! **Reaping.** Anyone may reap a run whose deadline has passed or whose
//! heartbeat is stale: kill the run's process group, then write an
//! `abandoned` done-marker. Reaping is a single bounded pass
//! ([`reap_expired_runs`]); no run is ever *waited on* and this module
//! contains no poll-a-peer loop — a consumer reads the done-marker or
//! does other work (INDEX §2.1, "No waiter loops"). The one sleep-free
//! wait inside the K1 kit is the reaper's own assertion that a killed
//! group is empty, bounded by the run's deadline.
//!
//! The process discipline mirrors [`crate::runner`]: children start in
//! their own process group (`process_group(0)` at spawn, in the caller),
//! and one group kill reaps the whole tree.
//!
//! # Bounds
//!
//! - [`RUN_ID_CHARS_MAX`]: longest run id.
//! - [`RUN_RECORD_BYTES_MAX`]: largest `run.json` / `done.json` read.
//! - [`ARTIFACTS_MAX`]: most artifacts in one done-marker.
//! - [`RUNS_PER_PASS_MAX`]: most run directories one reaping pass scans.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

/// Longest accepted run id, in characters. Run ids name directories, so
/// they are short labels, never payloads.
pub const RUN_ID_CHARS_MAX: usize = 64;

/// Largest `run.json` or `done.json` this module will read, in bytes.
/// Both files are small fixed-shape records; anything larger is refused
/// as malformed rather than parsed.
pub const RUN_RECORD_BYTES_MAX: u64 = 16_000;

/// Most artifacts one done-marker may claim.
pub const ARTIFACTS_MAX: usize = 256;

/// Longest artifact name in a done-marker, in characters.
pub const ARTIFACT_NAME_CHARS_MAX: usize = 256;

/// Most run directories one [`reap_expired_runs`] pass scans. A state
/// root with more runs than this needs an operator, not a bigger pass.
pub const RUNS_PER_PASS_MAX: usize = 1024;

/// A heartbeat is stale when its age exceeds this many intervals: one
/// missed beat is scheduling noise, two is a dead run.
pub const HEARTBEAT_STALE_FACTOR: u64 = 2;

/// File name of the run record inside a run directory.
pub const RUN_RECORD_FILE: &str = "run.json";
/// File name of the heartbeat inside a run directory.
pub const HEARTBEAT_FILE: &str = "heartbeat";
/// File name of the done-marker inside a run directory.
pub const DONE_FILE: &str = "done.json";

/// Current Unix time in milliseconds; a pre-epoch clock saturates to 0.
pub fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// SHA-256 of `bytes`, lowercase hex — the artifact-hash shape used by
/// done-markers and cues.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Streamed SHA-256 of a file, lowercase hex. Reads in bounded chunks;
/// the file itself is never loaded whole.
pub fn hash_file(path: &Path) -> Result<String, RunContractError> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut chunk = [0u8; 8192];
    loop {
        let read = file.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        hasher.update(&chunk[..read]);
    }
    let digest = hasher.finalize();
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    Ok(out)
}

/// Exit state recorded in a [`DoneMarker`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoneState {
    /// The run finished its work.
    Done,
    /// The run finished and reported failure.
    Failed,
    /// A reaper ended the run (deadline passed or heartbeat stale).
    Abandoned,
}

impl DoneState {
    /// Wire string for this state.
    pub fn as_str(self) -> &'static str {
        match self {
            DoneState::Done => "done",
            DoneState::Failed => "failed",
            DoneState::Abandoned => "abandoned",
        }
    }

    /// Parse a wire string; anything else is not a done state.
    pub fn parse(text: &str) -> Option<DoneState> {
        match text {
            "done" => Some(DoneState::Done),
            "failed" => Some(DoneState::Failed),
            "abandoned" => Some(DoneState::Abandoned),
            _ => None,
        }
    }
}

/// One artifact a run claims, with the SHA-256 of its bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactHash {
    /// Artifact name (a label, not a path).
    pub name: String,
    /// Lowercase hex SHA-256 of the artifact's bytes.
    pub sha256: String,
}

/// The done-marker: written last, atomically, when a run ends — by the
/// run itself, or by a reaper writing [`DoneState::Abandoned`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoneMarker {
    /// The run this marker closes.
    pub run_id: String,
    /// How the run ended.
    pub state: DoneState,
    /// Unix milliseconds when the marker was written.
    pub finished_unix_ms: u64,
    /// Hashes of the artifacts the run produced (empty for `abandoned`
    /// unless the reaper could still hash them).
    pub artifacts: Vec<ArtifactHash>,
}

impl DoneMarker {
    fn to_json(&self) -> String {
        let artifacts: Vec<serde_json::Value> = self
            .artifacts
            .iter()
            .map(|a| serde_json::json!({"name": a.name, "sha256": a.sha256}))
            .collect();
        serde_json::json!({
            "run_id": self.run_id,
            "state": self.state.as_str(),
            "finished_unix_ms": self.finished_unix_ms,
            "artifacts": artifacts,
        })
        .to_string()
    }

    fn from_json(text: &str) -> Result<DoneMarker, RunContractError> {
        let value: serde_json::Value =
            serde_json::from_str(text).map_err(|e| malformed(DONE_FILE, &e.to_string()))?;
        let run_id = value
            .get("run_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| malformed(DONE_FILE, "missing run_id"))?
            .to_owned();
        validate_run_id(&run_id)?;
        let state = value
            .get("state")
            .and_then(|v| v.as_str())
            .and_then(DoneState::parse)
            .ok_or_else(|| malformed(DONE_FILE, "missing or unknown state"))?;
        let finished_unix_ms = value
            .get("finished_unix_ms")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| malformed(DONE_FILE, "missing finished_unix_ms"))?;
        let raw_artifacts = value
            .get("artifacts")
            .and_then(|v| v.as_array())
            .ok_or_else(|| malformed(DONE_FILE, "missing artifacts"))?;
        if raw_artifacts.len() > ARTIFACTS_MAX {
            return Err(malformed(DONE_FILE, "too many artifacts"));
        }
        let mut artifacts = Vec::with_capacity(raw_artifacts.len());
        for raw in raw_artifacts {
            let name = raw
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| malformed(DONE_FILE, "artifact missing name"))?;
            if name.chars().count() > ARTIFACT_NAME_CHARS_MAX {
                return Err(malformed(DONE_FILE, "artifact name too long"));
            }
            let sha256 = raw
                .get("sha256")
                .and_then(|v| v.as_str())
                .ok_or_else(|| malformed(DONE_FILE, "artifact missing sha256"))?;
            if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(malformed(DONE_FILE, "artifact sha256 is not 64 hex"));
            }
            artifacts.push(ArtifactHash {
                name: name.to_owned(),
                sha256: sha256.to_owned(),
            });
        }
        Ok(DoneMarker {
            run_id,
            state,
            finished_unix_ms,
            artifacts,
        })
    }
}

/// The run record: the contract fields of INDEX §2.1, written at start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRecord {
    /// Unique run id; also the directory name under the state root.
    pub run_id: String,
    /// Unix milliseconds when the run started.
    pub started_unix_ms: u64,
    /// Absolute deadline in Unix milliseconds. Never extended.
    pub deadline_unix_ms: u64,
    /// Stated heartbeat interval in milliseconds.
    pub heartbeat_interval_ms: u64,
    /// The run's process-group id, when the run owns processes.
    pub pgid: Option<u32>,
}

impl RunRecord {
    fn to_json(&self) -> String {
        serde_json::json!({
            "run_id": self.run_id,
            "started_unix_ms": self.started_unix_ms,
            "deadline_unix_ms": self.deadline_unix_ms,
            "heartbeat_interval_ms": self.heartbeat_interval_ms,
            "pgid": self.pgid,
        })
        .to_string()
    }

    fn from_json(text: &str) -> Result<RunRecord, RunContractError> {
        let value: serde_json::Value =
            serde_json::from_str(text).map_err(|e| malformed(RUN_RECORD_FILE, &e.to_string()))?;
        let run_id = value
            .get("run_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| malformed(RUN_RECORD_FILE, "missing run_id"))?
            .to_owned();
        validate_run_id(&run_id)?;
        let started_unix_ms = value
            .get("started_unix_ms")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| malformed(RUN_RECORD_FILE, "missing started_unix_ms"))?;
        let deadline_unix_ms = value
            .get("deadline_unix_ms")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| malformed(RUN_RECORD_FILE, "missing deadline_unix_ms"))?;
        let heartbeat_interval_ms = value
            .get("heartbeat_interval_ms")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| malformed(RUN_RECORD_FILE, "missing heartbeat_interval_ms"))?;
        if deadline_unix_ms < started_unix_ms {
            return Err(malformed(RUN_RECORD_FILE, "deadline before start"));
        }
        if heartbeat_interval_ms == 0 {
            return Err(malformed(RUN_RECORD_FILE, "heartbeat interval is zero"));
        }
        let pgid = match value.get("pgid") {
            Some(serde_json::Value::Null) | None => None,
            Some(v) => Some(
                v.as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .ok_or_else(|| malformed(RUN_RECORD_FILE, "pgid is not a u32"))?,
            ),
        };
        Ok(RunRecord {
            run_id,
            started_unix_ms,
            deadline_unix_ms,
            heartbeat_interval_ms,
            pgid,
        })
    }
}

/// What a reaper should do with a run, decided from its record alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReapDecision {
    /// A done-marker exists; the run is closed.
    AlreadyDone,
    /// `now` is at or past the deadline.
    DeadlinePassed,
    /// The heartbeat is stale (or was never written).
    HeartbeatStale,
    /// The run is inside its deadline with a fresh heartbeat.
    Alive,
}

impl ReapDecision {
    /// True when the reaper must kill the group and mark `abandoned`.
    pub fn is_reapable(self) -> bool {
        matches!(
            self,
            ReapDecision::DeadlinePassed | ReapDecision::HeartbeatStale
        )
    }
}

/// Pure reap decision from record fields — the contract's decision table,
/// with no I/O, so every server can test its adoption against it.
///
/// Deadline wins over staleness: a run past both is reported by the
/// stronger fact.
pub fn decide_reap(
    record: &RunRecord,
    last_heartbeat_unix_ms: Option<u64>,
    has_done_marker: bool,
    now_unix_ms: u64,
) -> ReapDecision {
    if has_done_marker {
        return ReapDecision::AlreadyDone;
    }
    if now_unix_ms >= record.deadline_unix_ms {
        return ReapDecision::DeadlinePassed;
    }
    let stale_after = record
        .heartbeat_interval_ms
        .saturating_mul(HEARTBEAT_STALE_FACTOR);
    match last_heartbeat_unix_ms {
        None => ReapDecision::HeartbeatStale,
        Some(last) => {
            if now_unix_ms.saturating_sub(last) > stale_after {
                ReapDecision::HeartbeatStale
            } else {
                ReapDecision::Alive
            }
        }
    }
}

/// One run directory, opened or created.
#[derive(Debug, Clone)]
pub struct RunDir {
    dir: PathBuf,
    record: RunRecord,
}

impl RunDir {
    /// Create a fresh run directory under `state_root` and write its
    /// record and first heartbeat. Fails if the directory already
    /// exists: a run id is used exactly once.
    pub fn create(
        state_root: &Path,
        run_id: &str,
        started_unix_ms: u64,
        deadline_unix_ms: u64,
        heartbeat_interval_ms: u64,
        pgid: Option<u32>,
    ) -> Result<RunDir, RunContractError> {
        validate_run_id(run_id)?;
        if deadline_unix_ms < started_unix_ms {
            return Err(malformed(RUN_RECORD_FILE, "deadline before start"));
        }
        if heartbeat_interval_ms == 0 {
            return Err(malformed(RUN_RECORD_FILE, "heartbeat interval is zero"));
        }
        std::fs::create_dir_all(state_root)?;
        let dir = state_root.join(run_id);
        std::fs::create_dir(&dir)?;
        let run = RunDir {
            dir,
            record: RunRecord {
                run_id: run_id.to_owned(),
                started_unix_ms,
                deadline_unix_ms,
                heartbeat_interval_ms,
                pgid,
            },
        };
        write_atomic(&run.record_path(), run.record.to_json().as_bytes())?;
        run.heartbeat(started_unix_ms)?;
        Ok(run)
    }

    /// Open an existing run directory and read its record.
    pub fn open(state_root: &Path, run_id: &str) -> Result<RunDir, RunContractError> {
        validate_run_id(run_id)?;
        let dir = state_root.join(run_id);
        let text = read_bounded(&dir.join(RUN_RECORD_FILE))?;
        let record = RunRecord::from_json(&text)?;
        if record.run_id != run_id {
            return Err(malformed(
                RUN_RECORD_FILE,
                "run_id does not match directory",
            ));
        }
        Ok(RunDir { dir, record })
    }

    /// The run's record.
    pub fn record(&self) -> &RunRecord {
        &self.record
    }

    /// The run directory path.
    pub fn path(&self) -> &Path {
        &self.dir
    }

    fn record_path(&self) -> PathBuf {
        self.dir.join(RUN_RECORD_FILE)
    }

    fn heartbeat_path(&self) -> PathBuf {
        self.dir.join(HEARTBEAT_FILE)
    }

    fn done_path(&self) -> PathBuf {
        self.dir.join(DONE_FILE)
    }

    /// Touch the heartbeat at `at_unix_ms` (atomic sibling-temp rename).
    pub fn heartbeat(&self, at_unix_ms: u64) -> Result<(), RunContractError> {
        write_atomic(&self.heartbeat_path(), at_unix_ms.to_string().as_bytes())
    }

    /// The last heartbeat time, or `None` when no heartbeat file exists.
    /// A heartbeat file that does not parse is malformed, not missing.
    pub fn last_heartbeat_unix_ms(&self) -> Result<Option<u64>, RunContractError> {
        match std::fs::read(self.heartbeat_path()) {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                let parsed = text
                    .trim()
                    .parse::<u64>()
                    .map_err(|_| malformed(HEARTBEAT_FILE, "heartbeat is not unix ms"))?;
                Ok(Some(parsed))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Read the done-marker, or `None` when the run has not ended.
    pub fn read_done(&self) -> Result<Option<DoneMarker>, RunContractError> {
        if !self.done_path().exists() {
            return Ok(None);
        }
        let text = read_bounded(&self.done_path())?;
        let marker = DoneMarker::from_json(&text)?;
        if marker.run_id != self.record.run_id {
            return Err(malformed(DONE_FILE, "run_id does not match directory"));
        }
        Ok(Some(marker))
    }

    /// Write the done-marker atomically. Fails with
    /// [`RunContractError::AlreadyDone`] when a marker already exists:
    /// a run ends exactly once.
    pub fn write_done(
        &self,
        state: DoneState,
        artifacts: Vec<ArtifactHash>,
        finished_unix_ms: u64,
    ) -> Result<DoneMarker, RunContractError> {
        if artifacts.len() > ARTIFACTS_MAX {
            return Err(malformed(DONE_FILE, "too many artifacts"));
        }
        if self.done_path().exists() {
            return Err(RunContractError::AlreadyDone {
                run_id: self.record.run_id.clone(),
            });
        }
        let marker = DoneMarker {
            run_id: self.record.run_id.clone(),
            state,
            finished_unix_ms,
            artifacts,
        };
        write_atomic(&self.done_path(), marker.to_json().as_bytes())?;
        Ok(marker)
    }

    /// This run's reap decision at `now_unix_ms`.
    pub fn reap_decision(&self, now_unix_ms: u64) -> Result<ReapDecision, RunContractError> {
        let has_done = self.done_path().exists();
        let last = self.last_heartbeat_unix_ms()?;
        Ok(decide_reap(&self.record, last, has_done, now_unix_ms))
    }
}

/// The result of one [`reap_expired_runs`] pass.
#[derive(Debug, Default)]
pub struct ReapPass {
    /// `(run_id, decision)` for every run directory that parsed, in
    /// directory-name order. Reapable runs were killed and marked.
    pub decisions: Vec<(String, ReapDecision)>,
    /// Directory names that were skipped because they did not parse as
    /// run directories. Named, never silently dropped.
    pub skipped: Vec<String>,
}

/// One bounded reaping pass over `state_root`: for every run whose
/// deadline passed or heartbeat is stale, kill its process group (when
/// it has one) and write an `abandoned` done-marker. Live and finished
/// runs are reported, not touched. This function never sleeps and never
/// loops waiting for a run; a group that survives its SIGKILL is the
/// caller's K1 assertion to catch, not a reason to poll here.
pub fn reap_expired_runs(
    state_root: &Path,
    now_unix_ms: u64,
) -> Result<ReapPass, RunContractError> {
    let mut pass = ReapPass::default();
    let entries = match std::fs::read_dir(state_root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(pass),
        Err(e) => return Err(e.into()),
    };
    let mut names: Vec<String> = Vec::new();
    for entry in entries {
        let entry = entry?;
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
        if names.len() > RUNS_PER_PASS_MAX {
            break;
        }
    }
    names.sort();
    for name in names {
        let run = match RunDir::open(state_root, &name) {
            Ok(run) => run,
            Err(_) => {
                pass.skipped.push(name);
                continue;
            }
        };
        let decision = match run.reap_decision(now_unix_ms) {
            Ok(decision) => decision,
            Err(_) => {
                pass.skipped.push(name);
                continue;
            }
        };
        if decision.is_reapable() {
            #[cfg(unix)]
            if let Some(pgid) = run.record().pgid {
                kill_group(pgid);
            }
            // A marker write can race a run finishing itself; AlreadyDone
            // there is the correct end state, not an error.
            match run.write_done(DoneState::Abandoned, Vec::new(), now_unix_ms) {
                Ok(_) | Err(RunContractError::AlreadyDone { .. }) => {}
                Err(e) => return Err(e),
            }
        }
        pass.decisions.push((name, decision));
    }
    Ok(pass)
}

/// Pids of every process in group `pgid`, read from `/proc` (Linux).
/// Empty on non-Linux Unix, where the K1 kit's group assertion degrades
/// to checking the leader only; the estate's servers run on Linux.
#[cfg(target_os = "linux")]
pub fn group_member_pids(pgid: u32) -> Vec<u32> {
    let mut members = Vec::new();
    let Ok(proc_entries) = std::fs::read_dir("/proc") else {
        return members;
    };
    for entry in proc_entries.flatten() {
        let name = entry.file_name();
        let Some(pid) = name.to_str().and_then(|s| s.parse::<u32>().ok()) else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        // comm is parenthesised and may contain spaces or parens; the
        // fields after the LAST ')' are: state ppid pgrp ...
        let Some(close) = stat.rfind(')') else {
            continue;
        };
        let fields: Vec<&str> = stat[close + 1..].split_whitespace().collect();
        if fields.len() >= 3
            && let Ok(member_pgid) = fields[2].parse::<u32>()
            && member_pgid == pgid
        {
            members.push(pid);
        }
    }
    members.sort_unstable();
    members
}

/// Non-Linux fallback: only the group leader is observable here.
#[cfg(all(unix, not(target_os = "linux")))]
pub fn group_member_pids(pgid: u32) -> Vec<u32> {
    if process_alive(pgid) {
        vec![pgid]
    } else {
        Vec::new()
    }
}

/// True when `pid` names a live process. On Linux this is a `/proc`
/// presence check; elsewhere it conservatively reports `false` (the
/// estate's servers run on Linux, and a false "dead" is the fail-safe
/// direction for a reap assertion, never for a kill decision — kills
/// always go through the recorded process group).
#[cfg(unix)]
pub fn process_alive(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        Path::new(&format!("/proc/{pid}")).exists()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        false
    }
}

/// Kill a whole process group with SIGKILL (best-effort: an
/// already-gone group reports ESRCH, which is the wanted end state).
#[cfg(unix)]
pub fn kill_group(pgid: u32) {
    use rustix::process::{Pid, Signal, kill_process_group};
    if let Some(pid) = i32::try_from(pgid).ok().and_then(Pid::from_raw) {
        let _ = kill_process_group(pid, Signal::KILL);
    }
}

/// Failure modes of the run contract's filesystem operations.
#[derive(Debug)]
pub enum RunContractError {
    /// Filesystem I/O failed.
    Io(std::io::Error),
    /// A run id failed validation.
    InvalidRunId(String),
    /// A contract file existed but did not parse or violated a bound.
    Malformed {
        /// Which contract file.
        file: &'static str,
        /// What was wrong with it.
        reason: String,
    },
    /// A done-marker already exists for this run.
    AlreadyDone {
        /// The run that already ended.
        run_id: String,
    },
}

impl std::fmt::Display for RunContractError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunContractError::Io(e) => write!(f, "run contract I/O error: {e}"),
            RunContractError::InvalidRunId(id) => write!(f, "invalid run_id: {id:?}"),
            RunContractError::Malformed { file, reason } => {
                write!(f, "malformed {file}: {reason}")
            }
            RunContractError::AlreadyDone { run_id } => {
                write!(f, "run {run_id} already has a done-marker")
            }
        }
    }
}

impl std::error::Error for RunContractError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            RunContractError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for RunContractError {
    fn from(e: std::io::Error) -> RunContractError {
        RunContractError::Io(e)
    }
}

fn malformed(file: &'static str, reason: &str) -> RunContractError {
    RunContractError::Malformed {
        file,
        reason: reason.to_owned(),
    }
}

/// Run ids name directories: `[A-Za-z0-9._-]`, starting alphanumeric,
/// never containing `..`, at most [`RUN_ID_CHARS_MAX`] characters.
pub fn validate_run_id(run_id: &str) -> Result<(), RunContractError> {
    let valid = !run_id.is_empty()
        && run_id.chars().count() <= RUN_ID_CHARS_MAX
        && !run_id.contains("..")
        && run_id
            .chars()
            .next()
            .map(|c| c.is_ascii_alphanumeric())
            .unwrap_or(false)
        && run_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if valid {
        Ok(())
    } else {
        Err(RunContractError::InvalidRunId(run_id.to_owned()))
    }
}

/// Write `bytes` to `path` atomically: sibling temp file, then rename.
/// The temp name carries the writer's pid so concurrent writers to the
/// same path never share a temp file.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), RunContractError> {
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_owned());
    let tmp = path.with_file_name(format!(".{file_name}.tmp-{}", std::process::id()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Read a contract file with the [`RUN_RECORD_BYTES_MAX`] bound.
fn read_bounded(path: &Path) -> Result<String, RunContractError> {
    use std::io::Read;
    let file = std::fs::File::open(path)?;
    let mut text = String::new();
    file.take(RUN_RECORD_BYTES_MAX + 1)
        .read_to_string(&mut text)?;
    if text.len() as u64 > RUN_RECORD_BYTES_MAX {
        return Err(malformed("contract file", "exceeds RUN_RECORD_BYTES_MAX"));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_root(prefix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "phlow-run-contract-test-{prefix}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).expect("test setup: create temp root");
        dir
    }

    fn record(started: u64, deadline: u64, interval: u64) -> RunRecord {
        RunRecord {
            run_id: "run-1".to_owned(),
            started_unix_ms: started,
            deadline_unix_ms: deadline,
            heartbeat_interval_ms: interval,
            pgid: None,
        }
    }

    #[test]
    fn run_id_validation_bounds_the_name() {
        assert!(validate_run_id("run-2026.11_10-a").is_ok());
        assert!(validate_run_id("").is_err());
        assert!(validate_run_id("../escape").is_err());
        assert!(validate_run_id("a/b").is_err());
        assert!(validate_run_id("-leading-dash").is_err());
        assert!(validate_run_id(&"x".repeat(RUN_ID_CHARS_MAX + 1)).is_err());
    }

    #[test]
    fn decide_reap_decision_table() {
        let rec = record(1_000, 10_000, 1_000);
        // Done beats everything.
        assert_eq!(
            decide_reap(&rec, None, true, 99_000),
            ReapDecision::AlreadyDone
        );
        // Deadline beats staleness.
        assert_eq!(
            decide_reap(&rec, None, false, 10_000),
            ReapDecision::DeadlinePassed
        );
        // No heartbeat at all is stale.
        assert_eq!(
            decide_reap(&rec, None, false, 5_000),
            ReapDecision::HeartbeatStale
        );
        // Fresh heartbeat inside the deadline is alive.
        assert_eq!(
            decide_reap(&rec, Some(4_500), false, 5_000),
            ReapDecision::Alive
        );
        // Age beyond 2 intervals is stale; exactly 2 is not.
        assert_eq!(
            decide_reap(&rec, Some(3_000), false, 5_000),
            ReapDecision::Alive
        );
        assert_eq!(
            decide_reap(&rec, Some(2_999), false, 5_000),
            ReapDecision::HeartbeatStale
        );
    }

    #[test]
    fn create_open_round_trip_and_done_marker() {
        let root = temp_root("roundtrip");
        let run = RunDir::create(&root, "run-1", 1_000, 10_000, 500, Some(4242)).unwrap();
        assert_eq!(run.last_heartbeat_unix_ms().unwrap(), Some(1_000));
        run.heartbeat(2_000).unwrap();
        let opened = RunDir::open(&root, "run-1").unwrap();
        assert_eq!(opened.record().pgid, Some(4242));
        assert_eq!(opened.last_heartbeat_unix_ms().unwrap(), Some(2_000));
        assert!(opened.read_done().unwrap().is_none());
        let marker = opened
            .write_done(
                DoneState::Done,
                vec![ArtifactHash {
                    name: "out.bin".to_owned(),
                    sha256: sha256_hex(b"out"),
                }],
                3_000,
            )
            .unwrap();
        assert_eq!(marker.state, DoneState::Done);
        let read_back = opened.read_done().unwrap().unwrap();
        assert_eq!(read_back, marker);
        // A run ends exactly once.
        assert!(matches!(
            opened.write_done(DoneState::Failed, Vec::new(), 4_000),
            Err(RunContractError::AlreadyDone { .. })
        ));
        assert_eq!(
            opened.reap_decision(99_000).unwrap(),
            ReapDecision::AlreadyDone
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn duplicate_run_id_is_refused() {
        let root = temp_root("dup");
        RunDir::create(&root, "run-1", 0, 10_000, 500, None).unwrap();
        assert!(RunDir::create(&root, "run-1", 0, 10_000, 500, None).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn reap_pass_marks_only_expired_runs() {
        let root = temp_root("reappass");
        RunDir::create(&root, "expired", 0, 1_000, 100, None).unwrap();
        let live = RunDir::create(&root, "live", 0, 100_000, 100, None).unwrap();
        live.heartbeat(5_000).unwrap();
        std::fs::create_dir(root.join("not-a-run")).unwrap();
        let pass = reap_expired_runs(&root, 5_000).unwrap();
        assert_eq!(pass.skipped, vec!["not-a-run".to_owned()]);
        let expired = RunDir::open(&root, "expired").unwrap();
        assert_eq!(
            expired.read_done().unwrap().unwrap().state,
            DoneState::Abandoned
        );
        assert!(live.read_done().unwrap().is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn sha256_hex_matches_known_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
