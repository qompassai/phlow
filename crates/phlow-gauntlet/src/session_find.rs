// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.
//! Session search engine: scan once, then answer ranked queries.
//!
//! ELI5: phlow keeps a folder of session files (one JSON file per past
//! agent session). This module reads every file once into a SQLite
//! database, then answers searches from the database — the TUI and the
//! daemon share the one file, so both always rank the same way.
//!
//! Adapted concept (from Ghostex `packages/find`, the zehn engine):
//! "scan once, then answer ranked queries" with one shared SQLite engine.
//! What is NOT lifted: Ghostex's per-agent parsers for other agent
//! CLIs' history directories; only phlow's own session format is
//! indexed. The scanner walks exactly the configured session root and
//! skips dot-directories, so foreign trees are never entered.
//!
//! The phlow session record format (ours, defined here): one JSON object
//! per file, file name ending in `.session.json`:
//! `{"id": "...", "title": "...", "project": "...",
//!   "transcript": "...", "started_at": 1234567890}`.
//! Missing fields get defaults (`id` falls back to the file stem); a
//! file that is not valid JSON is skipped and counted.
//!
//! Design rules enforced by this module:
//! - Incremental rescan key is `(mtime, size)` per file: unchanged files
//!   are never re-read (proven by the [`FsLog`] read counter).
//! - ALL SQL goes through bound parameters; [`SqlLog`] records every
//!   statement template so tests can prove no session content is ever
//!   interpolated into SQL.
//! - User queries are data, not code: each whitespace-separated term is
//!   LIKE-escaped and matched literally (conjunction of terms); ranking
//!   is a deterministic fuzzy score computed in Rust, with ties broken
//!   by session id — no nondeterministic ordering anywhere.
//! - Oversized text fields are truncated at [`MAX_FIELD_BYTES`] with a
//!   `truncated` flag stored per row.
//! - The index opens in WAL mode so TUI and daemon readers never block
//!   each other; every rescan commits in a single transaction and bumps
//!   a generation counter, so readers see pre- or post-rescan state,
//!   never a mix.
//! - A corrupt database is detected with `PRAGMA integrity_check` and
//!   reported as [`IndexError::Corrupt`]; [`open_or_rebuild`] quarantines
//!   the corrupt file and rebuilds from a fresh scan.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, UNIX_EPOCH};

use rusqlite::Connection;

/// Maximum bytes stored per text field. Larger fields are truncated
/// (with the row's `truncated` flag set) so one hostile session file
/// cannot blow up the index.
pub const MAX_FIELD_BYTES: usize = 1_048_576;
/// Maximum accepted user query length in bytes.
pub const MAX_QUERY_LEN: usize = 512;
/// Maximum whitespace-separated terms taken from one query.
pub const MAX_QUERY_TERMS: usize = 16;
/// Maximum hits returned by one search call.
pub const MAX_RESULTS: usize = 100;
/// Maximum session files indexed in one scan (bomb guard).
pub const MAX_SCAN_FILES: usize = 100_000;
/// Only files with this suffix are indexed; everything else under the
/// session root (including dot-directories) is ignored.
pub const SESSION_SUFFIX: &str = ".session.json";
/// Name of the SQLite file inside a session root's index directory.
pub const DB_FILE_NAME: &str = "index.sqlite";
/// Integrity probe run on every index open.
pub const INTEGRITY_SQL: &str = "PRAGMA integrity_check";
/// Expected single-row answer of a healthy integrity probe.
pub const INTEGRITY_OK: &str = "ok";

/// Typed failures of the session index. Corruption is reported, never
/// panicked on, and never answered with partial data.
#[derive(Debug)]
pub enum IndexError {
    /// Filesystem failure while scanning or quarantining.
    Io(std::io::Error),
    /// SQLite failure (query, transaction, pragma).
    Sql(rusqlite::Error),
    /// The index file failed `PRAGMA integrity_check`.
    Corrupt(String),
    /// A session file was not valid JSON; the path is named.
    Malformed { path: PathBuf },
    /// The user query exceeded [`MAX_QUERY_LEN`].
    QueryTooLong,
    /// The scan hit [`MAX_SCAN_FILES`].
    TooManyFiles,
}

impl std::fmt::Display for IndexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IndexError::Io(e) => write!(f, "session index io error: {e}"),
            IndexError::Sql(e) => write!(f, "session index sql error: {e}"),
            IndexError::Corrupt(d) => write!(f, "session index corrupt: {d}"),
            IndexError::Malformed { path } => {
                write!(f, "malformed session file: {}", path.display())
            }
            IndexError::QueryTooLong => {
                write!(f, "query exceeds {MAX_QUERY_LEN} bytes")
            }
            IndexError::TooManyFiles => {
                write!(f, "scan exceeds {MAX_SCAN_FILES} files")
            }
        }
    }
}

impl std::error::Error for IndexError {}

impl From<std::io::Error> for IndexError {
    fn from(e: std::io::Error) -> Self {
        IndexError::Io(e)
    }
}

impl From<rusqlite::Error> for IndexError {
    fn from(e: rusqlite::Error) -> Self {
        IndexError::Sql(e)
    }
}

/// Log of session-file reads. The scanner records every file it opens
/// for reading; tests assert on the log to prove incremental rescans
/// touch only new/changed files and never foreign trees.
#[derive(Clone, Debug, Default)]
pub struct FsLog {
    reads: Vec<PathBuf>,
}

impl FsLog {
    /// Create an empty log.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one file read.
    pub fn record(&mut self, path: &Path) {
        self.reads.push(path.to_path_buf());
    }

    /// All recorded reads, in order.
    pub fn reads(&self) -> &[PathBuf] {
        &self.reads
    }

    /// Number of recorded reads.
    pub fn read_count(&self) -> usize {
        self.reads.len()
    }

    /// Number of recorded reads at or under `dir`.
    pub fn reads_under(&self, dir: &Path) -> usize {
        self.reads.iter().filter(|p| p.starts_with(dir)).count()
    }

    /// Drop all recorded reads (used between build and rescan).
    pub fn clear(&mut self) {
        self.reads.clear();
    }
}

/// Log of SQL statement templates. Every statement the engine runs is
/// recorded here as its fixed template text (never with data), so a
/// test can prove all inserts go through bound parameters.
#[derive(Clone, Debug, Default)]
pub struct SqlLog {
    statements: Vec<String>,
}

impl SqlLog {
    /// Create an empty log.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one statement template.
    pub fn record(&mut self, sql: &str) {
        self.statements.push(sql.to_string());
    }

    /// All recorded templates, in order.
    pub fn statements(&self) -> &[String] {
        &self.statements
    }
}

/// Outcome of one [`scan_and_index`] run.
#[derive(Clone, Debug)]
pub struct ScanStats {
    /// Session files actually read (not skipped as unchanged).
    pub files_read: usize,
    /// Rows inserted or updated.
    pub rows_upserted: usize,
    /// Rows deleted (files gone from the session root).
    pub rows_deleted: usize,
    /// Files skipped because they were not valid JSON.
    pub malformed_skipped: usize,
    /// The generation counter value stamped on this scan.
    pub generation: u64,
}

impl ScanStats {
    /// Zeroed counters for the given generation.
    fn new(generation: u64) -> Self {
        Self {
            files_read: 0,
            rows_upserted: 0,
            rows_deleted: 0,
            malformed_skipped: 0,
            generation,
        }
    }
}

/// One ranked search hit.
#[derive(Clone, Debug)]
pub struct Hit {
    /// Session id from the session file.
    pub id: String,
    /// Session title from the session file.
    pub title: String,
    /// Deterministic fuzzy score (higher is better).
    pub score: i64,
    /// Rescan generation the hit's row belongs to.
    pub generation: u64,
}

/// A scripted session fixture (what a test writes to disk).
#[derive(Clone, Debug)]
pub struct SessionSpec {
    /// Session id.
    pub id: String,
    /// Session title.
    pub title: String,
    /// Project path.
    pub project: String,
    /// Session transcript text.
    pub transcript: String,
    /// Start time as unix seconds.
    pub started_at: i64,
}

impl SessionSpec {
    /// Build a spec from the parts tests vary.
    pub fn new(id: &str, title: &str, transcript: &str) -> Self {
        Self {
            id: id.to_string(),
            title: title.to_string(),
            project: String::new(),
            transcript: transcript.to_string(),
            started_at: 1_700_000_000,
        }
    }
}

/// The single upsert template for session rows. Every session write
/// goes through this one fixed string with bound parameters — session
/// content is never interpolated into SQL.
const UPSERT_SQL: &str = "INSERT INTO sessions(path,id,title,project,transcript,truncated,mtime,size,started_at,generation) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(path) DO UPDATE SET id=excluded.id,title=excluded.title,project=excluded.project,transcript=excluded.transcript,truncated=excluded.truncated,mtime=excluded.mtime,size=excluded.size,started_at=excluded.started_at,generation=excluded.generation";
/// Fixed delete template (path is a bound parameter).
const DELETE_SQL: &str = "DELETE FROM sessions WHERE path=?1";
/// Fixed generation-stamp template.
const STAMP_GENERATION_SQL: &str = "UPDATE sessions SET generation=?1";
/// Fixed meta-write template (the key is a fixed literal).
const META_SQL: &str = "INSERT INTO meta(key,value) VALUES('generation',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value";

const SCHEMA_SQL: &str = "CREATE TABLE IF NOT EXISTS sessions(path TEXT PRIMARY KEY,id TEXT NOT NULL,title TEXT NOT NULL,project TEXT NOT NULL,transcript TEXT NOT NULL,truncated INTEGER NOT NULL,mtime INTEGER NOT NULL,size INTEGER NOT NULL,started_at INTEGER NOT NULL,generation INTEGER NOT NULL);CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY,value TEXT NOT NULL);CREATE INDEX IF NOT EXISTS idx_sessions_title ON sessions(title);";

/// Open the database file. SQLite opens lazily: corruption only
/// surfaces on first access, which is why the integrity probe always
/// runs before any pragma or schema write.
fn open_raw(db_path: &Path) -> Result<Connection, IndexError> {
    Ok(Connection::open(db_path)?)
}

/// Switch an open database to WAL mode and ensure the schema. Only
/// called after the integrity probe has passed.
fn prepare(conn: &Connection) -> Result<(), IndexError> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.busy_timeout(Duration::from_millis(5_000))?;
    conn.execute_batch(SCHEMA_SQL)?;
    Ok(())
}

/// Run the integrity probe. Any probe failure — including a probe that
/// itself errors, as with a zeroed header — counts as corruption.
fn integrity_ok(conn: &Connection) -> bool {
    match conn.query_row(INTEGRITY_SQL, [], |row| row.get::<_, String>(0)) {
        Ok(answer) => answer == INTEGRITY_OK,
        Err(_) => false,
    }
}

/// Escape one user query term for a LIKE pattern with `ESCAPE '\'`.
/// The term is matched literally: `%`, `_`, and the escape character
/// itself lose their wildcard meaning.
pub fn escape_like(term: &str) -> String {
    let mut out = String::with_capacity(term.len() + 2);
    for ch in term.chars() {
        if ch == '\\' || ch == '%' || ch == '_' {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// Truncate a text field to [`MAX_FIELD_BYTES`] bytes on a UTF-8
/// boundary. Returns the (possibly truncated) text and whether it was
/// truncated.
fn truncate_field(text: &str) -> (String, bool) {
    if text.len() <= MAX_FIELD_BYTES {
        return (text.to_string(), false);
    }
    let cut = text.floor_char_boundary(MAX_FIELD_BYTES);
    (text[..cut].to_string(), true)
}

/// Fuzzy score tier of one query term against one lowercased field.
/// Whole-word beats substring beats subsequence; no match scores 0.
fn term_tier(term: &str, field: &str, whole: i64, sub: i64, seq: i64) -> i64 {
    if term.is_empty() || field.is_empty() {
        return 0;
    }
    if field
        .split(|c: char| !c.is_alphanumeric())
        .any(|w| w == term)
    {
        return whole;
    }
    if field.contains(term) {
        return sub;
    }
    let mut chars = field.chars();
    for tc in term.chars() {
        if chars.by_ref().find(|c| *c == tc).is_none() {
            return 0;
        }
    }
    seq
}

/// Deterministic fuzzy score of a session against the query terms.
/// Per term, the better of the title tier (300/150/30) and the
/// transcript tier (100/50/10) wins; the sum is the score. Pure
/// function of (terms, title, transcript): no maps, no randomness.
pub fn fuzzy_score(terms: &[String], title: &str, transcript: &str) -> i64 {
    let title_lc = title.to_lowercase();
    let transcript_lc = transcript.to_lowercase();
    let mut score: i64 = 0;
    for term in terms {
        let title_tier = term_tier(term, &title_lc, 300, 150, 30);
        let transcript_tier = term_tier(term, &transcript_lc, 100, 50, 10);
        score += title_tier.max(transcript_tier);
    }
    score
}

/// Split a raw query into at most [`MAX_QUERY_TERMS`] lowercased terms.
fn query_terms(query: &str) -> Result<Vec<String>, IndexError> {
    if query.len() > MAX_QUERY_LEN {
        return Err(IndexError::QueryTooLong);
    }
    Ok(query
        .split_whitespace()
        .take(MAX_QUERY_TERMS)
        .map(|t| t.to_lowercase())
        .collect())
}

/// Collect session files under `root`: recursive walk with an explicit
/// stack (no recursion), dot-directories skipped, results sorted for a
/// deterministic scan order. Only `*.session.json` files qualify.
fn collect_session_files(root: &Path) -> Result<Vec<PathBuf>, IndexError> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir)?;
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if entry.file_type()?.is_dir() {
                if !name.starts_with('.') {
                    stack.push(path);
                }
                continue;
            }
            if name.ends_with(SESSION_SUFFIX) {
                files.push(path);
                if files.len() > MAX_SCAN_FILES {
                    return Err(IndexError::TooManyFiles);
                }
            }
        }
    }
    files.sort();
    Ok(files)
}

/// Read (path -> (mtime, size)) for every row already indexed.
fn read_manifest(conn: &Connection) -> Result<HashMap<String, (i64, i64)>, IndexError> {
    let mut manifest = HashMap::new();
    let mut stmt = conn.prepare("SELECT path,mtime,size FROM sessions")?;
    let rows = stmt.query_map([], |row| {
        let path: String = row.get(0)?;
        let mtime: i64 = row.get(1)?;
        let size: i64 = row.get(2)?;
        Ok((path, (mtime, size)))
    })?;
    for row in rows {
        let (path, key) = row?;
        manifest.insert(path, key);
    }
    Ok(manifest)
}

/// File mtime as unix seconds.
fn file_mtime(path: &Path) -> Result<i64, IndexError> {
    let modified = std::fs::metadata(path)?.modified()?;
    let secs = modified
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs();
    Ok(secs as i64)
}

/// Parse one session file into (id, title, project, transcript,
/// truncated, started_at). Not-JSON is a typed Malformed skip.
fn parse_session(
    rel: &str,
    bytes: &[u8],
) -> Result<(String, String, String, String, bool, i64), IndexError> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| IndexError::Malformed {
            path: PathBuf::from(rel),
        })?;
    let get = |key: &str| {
        value
            .get(key)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    let id = {
        let raw = get("id");
        if raw.is_empty() { rel.to_string() } else { raw }
    };
    let (title, t_trunc) = truncate_field(&get("title"));
    let (project, p_trunc) = truncate_field(&get("project"));
    let (transcript, x_trunc) = truncate_field(&get("transcript"));
    let started_at = value
        .get("started_at")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    Ok((
        id,
        title,
        project,
        transcript,
        t_trunc || p_trunc || x_trunc,
        started_at,
    ))
}

/// Scan `root` into the SQLite index at `db_path`, incrementally:
/// files whose (mtime, size) match the manifest are not re-read.
/// Every read is recorded in `fs_log`; every SQL template in `sql_log`.
/// The whole scan commits in one transaction, stamped with a bumped
/// generation, so concurrent readers see a single consistent state.
///
/// Refuses to build on a corrupt database: returns
/// [`IndexError::Corrupt`] instead of mixing new rows into a broken
/// file. Use [`open_or_rebuild`] to quarantine-and-rebuild.
pub fn scan_and_index(
    root: &Path,
    db_path: &Path,
    fs_log: &mut FsLog,
    sql_log: &mut SqlLog,
) -> Result<ScanStats, IndexError> {
    let mut conn = open_raw(db_path)?;
    if db_path.exists() && !integrity_ok(&conn) {
        return Err(IndexError::Corrupt(format!(
            "refusing scan into corrupt index {}",
            db_path.display()
        )));
    }
    prepare(&conn)?;
    let manifest = read_manifest(&conn)?;
    let files = collect_session_files(root)?;
    let generation = next_generation(&conn);

    let tx = conn.transaction()?;
    let mut stats = ScanStats::new(generation);
    let mut seen: HashSet<String> = HashSet::with_capacity(files.len());
    let mut ctx = ScanCtx {
        fs_log: &mut *fs_log,
        sql_log: &mut *sql_log,
        stats: &mut stats,
        seen: &mut seen,
    };
    index_files(&tx, root, &files, &manifest, generation, &mut ctx)?;
    finalize_scan(&tx, &manifest, &seen, sql_log, generation, &mut stats)?;
    tx.commit()?;
    Ok(stats)
}

/// Read the current generation from the meta table and bump it for
/// the scan about to commit. Missing or unparsable means 0 → 1.
fn next_generation(conn: &Connection) -> u64 {
    conn.query_row("SELECT value FROM meta WHERE key='generation'", [], |row| {
        row.get::<_, String>(0)
    })
    .ok()
    .and_then(|v| v.parse::<u64>().ok())
    .unwrap_or(0)
    .saturating_add(1)
}

/// Mutable scan state threaded through indexing: what was seen,
/// what was read, and where the audit logs go.
struct ScanCtx<'a> {
    fs_log: &'a mut FsLog,
    sql_log: &'a mut SqlLog,
    stats: &'a mut ScanStats,
    seen: &'a mut HashSet<String>,
}

/// Upsert every changed file: read, parse, and write one row per
/// file through the single parameterized template. Unchanged files
/// (manifest hit) and malformed files are skipped, counted.
fn index_files(
    tx: &rusqlite::Transaction<'_>,
    root: &Path,
    files: &[PathBuf],
    manifest: &HashMap<String, (i64, i64)>,
    generation: u64,
    ctx: &mut ScanCtx<'_>,
) -> Result<(), IndexError> {
    for path in files {
        let rel = path
            .strip_prefix(root)
            .map_err(|_| {
                IndexError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "session file escaped scan root",
                ))
            })?
            .to_string_lossy()
            .to_string();
        ctx.seen.insert(rel.clone());
        let mtime = file_mtime(path)?;
        let size = std::fs::metadata(path)?.len() as i64;
        if manifest.get(&rel) == Some(&(mtime, size)) {
            continue; // Unchanged: not re-read.
        }
        ctx.fs_log.record(path);
        ctx.stats.files_read += 1;
        let bytes = std::fs::read(path)?;
        let parsed = match parse_session(&rel, &bytes) {
            Ok(p) => p,
            Err(IndexError::Malformed { .. }) => {
                ctx.stats.malformed_skipped += 1;
                continue;
            }
            Err(e) => return Err(e),
        };
        let (id, title, project, transcript, truncated, started_at) = parsed;
        ctx.sql_log.record(UPSERT_SQL);
        tx.execute(
            UPSERT_SQL,
            rusqlite::params![
                rel,
                id,
                title,
                project,
                transcript,
                i64::from(truncated),
                mtime,
                size,
                started_at,
                generation as i64
            ],
        )?;
        ctx.stats.rows_upserted += 1;
    }
    Ok(())
}

/// Delete rows for files that vanished, stamp every row with the new
/// generation, and persist the generation in the meta table.
fn finalize_scan(
    tx: &rusqlite::Transaction<'_>,
    manifest: &HashMap<String, (i64, i64)>,
    seen: &HashSet<String>,
    sql_log: &mut SqlLog,
    generation: u64,
    stats: &mut ScanStats,
) -> Result<(), IndexError> {
    for old in manifest.keys() {
        if !seen.contains(old) {
            sql_log.record(DELETE_SQL);
            tx.execute(DELETE_SQL, rusqlite::params![old])?;
            stats.rows_deleted += 1;
        }
    }
    sql_log.record(STAMP_GENERATION_SQL);
    tx.execute(STAMP_GENERATION_SQL, rusqlite::params![generation as i64])?;
    sql_log.record(META_SQL);
    tx.execute(META_SQL, rusqlite::params![generation.to_string()])?;
    Ok(())
}

/// An open, integrity-verified session index. One shared file serves
/// the TUI and the daemon; each opens its own connection in WAL mode.
pub struct SessionIndex {
    conn: Connection,
    query_log: Vec<String>,
}

impl SessionIndex {
    /// Open the index, refusing corrupt files with
    /// [`IndexError::Corrupt`] instead of serving partial data. The
    /// integrity probe runs before any pragma or write, so corruption
    /// can never surface as a misleading generic SQL error.
    pub fn open_strict(db_path: &Path) -> Result<Self, IndexError> {
        let conn = open_raw(db_path)?;
        if !integrity_ok(&conn) {
            return Err(IndexError::Corrupt(format!(
                "integrity check failed for {}",
                db_path.display()
            )));
        }
        prepare(&conn)?;
        Ok(Self {
            conn,
            query_log: Vec::new(),
        })
    }

    /// Ranked search. Every query term is LIKE-escaped and matched
    /// literally (terms are ANDed); candidates are then ranked by the
    /// deterministic [`fuzzy_score`] in Rust and ties broken by session
    /// id, so repeated runs produce byte-identical rankings. The
    /// escaped LIKE form is appended to the query log (data, not code).
    pub fn search(&mut self, query: &str, limit: usize) -> Result<Vec<Hit>, IndexError> {
        let terms = query_terms(query)?;
        let limit = limit.clamp(1, MAX_RESULTS);
        let (sql, patterns) = search_sql(&terms);
        self.query_log.push(format!(
            "terms={terms:?} like_patterns={patterns:?} escape='\\'"
        ));
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(patterns.iter()), |row| {
            let id: String = row.get(1)?;
            let title: String = row.get(2)?;
            let transcript: String = row.get(3)?;
            let generation: i64 = row.get(4)?;
            Ok((id, title, transcript, generation))
        })?;
        let mut hits = Vec::new();
        for row in rows {
            let (id, title, transcript, generation) = row?;
            let score = fuzzy_score(&terms, &title, &transcript);
            if terms.is_empty() || score > 0 {
                hits.push(Hit {
                    id,
                    title,
                    score,
                    generation: generation as u64,
                });
            }
        }
        // Total order: score descending, then id ascending. Stable and
        // deterministic — no HashMap iteration, no randomness.
        hits.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.id.cmp(&b.id)));
        hits.truncate(limit);
        Ok(hits)
    }

    /// Number of indexed sessions.
    pub fn count(&self) -> Result<usize, IndexError> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))?;
        Ok(n as usize)
    }

    /// Recorded mtime for one indexed session (relative path), if present.
    pub fn row_mtime(&self, rel: &str) -> Result<Option<i64>, IndexError> {
        let mut stmt = self
            .conn
            .prepare("SELECT mtime FROM sessions WHERE path=?1")?;
        let mut rows = stmt.query(rusqlite::params![rel])?;
        match rows.next()? {
            Some(row) => Ok(Some(row.get(0)?)),
            None => Ok(None),
        }
    }

    /// Transcript length and truncation flag for one indexed session.
    pub fn row_info(&self, rel: &str) -> Result<Option<(i64, bool)>, IndexError> {
        let mut stmt = self
            .conn
            .prepare("SELECT length(transcript),truncated FROM sessions WHERE path=?1")?;
        let mut rows = stmt.query(rusqlite::params![rel])?;
        match rows.next()? {
            Some(row) => {
                let len: i64 = row.get(0)?;
                let truncated: i64 = row.get(1)?;
                Ok(Some((len, truncated != 0)))
            }
            None => Ok(None),
        }
    }

    /// Title of one indexed session, for literal-storage checks.
    pub fn row_title(&self, rel: &str) -> Result<Option<String>, IndexError> {
        let mut stmt = self
            .conn
            .prepare("SELECT title FROM sessions WHERE path=?1")?;
        let mut rows = stmt.query(rusqlite::params![rel])?;
        match rows.next()? {
            Some(row) => Ok(Some(row.get(0)?)),
            None => Ok(None),
        }
    }

    /// Current rescan generation.
    pub fn generation(&self) -> Result<u64, IndexError> {
        let value: String =
            self.conn
                .query_row("SELECT value FROM meta WHERE key='generation'", [], |row| {
                    row.get(0)
                })?;
        value
            .parse::<u64>()
            .map_err(|_| IndexError::Corrupt("bad generation in meta".to_string()))
    }

    /// The escaped LIKE forms of every query run on this handle.
    pub fn query_log(&self) -> &[String] {
        &self.query_log
    }
}

/// Build the search SQL: one LIKE pair per term, ANDed, with
/// `ESCAPE '\'` so the bound pattern is matched literally. Only
/// placeholders are interpolated — term text travels exclusively in
/// bound parameters.
fn search_sql(terms: &[String]) -> (String, Vec<String>) {
    if terms.is_empty() {
        return (
            "SELECT path,id,title,transcript,generation FROM sessions".to_string(),
            Vec::new(),
        );
    }
    let mut sql = String::from("SELECT path,id,title,transcript,generation FROM sessions WHERE ");
    let mut patterns = Vec::with_capacity(terms.len());
    for (i, term) in terms.iter().enumerate() {
        if i > 0 {
            sql.push_str(" AND ");
        }
        let placeholder = format!("?{}", i + 1);
        sql.push_str(&format!(
            "(title LIKE {placeholder} ESCAPE '\\' OR transcript LIKE {placeholder} ESCAPE '\\')"
        ));
        patterns.push(format!("%{}%", escape_like(term)));
    }
    (sql, patterns)
}

/// Outcome of [`open_or_rebuild`].
pub struct Recovered {
    /// The usable index.
    pub index: SessionIndex,
    /// True when the index was rebuilt from a fresh scan.
    pub rebuilt: bool,
    /// Where the corrupt file was quarantined, if any.
    pub quarantined: Option<PathBuf>,
    /// Scan stats of the rebuild, if one happened.
    pub stats: Option<ScanStats>,
}

/// Open the index, or rebuild it when corrupt. On
/// [`IndexError::Corrupt`] the bad file is renamed aside (stale WAL
/// sidecars removed so the fresh database starts clean) and a full
/// scan rebuilds it; queries are never served from the corrupt file —
/// before recovery they error, after it they read the rebuilt index.
pub fn open_or_rebuild(
    root: &Path,
    db_path: &Path,
    fs_log: &mut FsLog,
    sql_log: &mut SqlLog,
) -> Result<Recovered, IndexError> {
    match SessionIndex::open_strict(db_path) {
        Ok(index) => Ok(Recovered {
            index,
            rebuilt: false,
            quarantined: None,
            stats: None,
        }),
        Err(IndexError::Corrupt(_)) => {
            let quarantined = quarantine_path(db_path);
            std::fs::rename(db_path, &quarantined)?;
            for suffix in ["-wal", "-shm", "-journal"] {
                let sidecar = PathBuf::from(format!("{}{suffix}", db_path.display()));
                let _ = std::fs::remove_file(sidecar);
            }
            let stats = scan_and_index(root, db_path, fs_log, sql_log)?;
            let index = SessionIndex::open_strict(db_path)?;
            Ok(Recovered {
                index,
                rebuilt: true,
                quarantined: Some(quarantined),
                stats: Some(stats),
            })
        }
        Err(other) => Err(other),
    }
}

/// Quarantine destination for a corrupt index: `<name>.corrupt-<n>`,
/// first free `n`.
fn quarantine_path(db_path: &Path) -> PathBuf {
    let base = db_path.display().to_string();
    let mut n: u64 = 0;
    loop {
        let candidate = PathBuf::from(format!("{base}.corrupt-{n}"));
        if !candidate.exists() {
            return candidate;
        }
        n += 1;
    }
}

/// The exact attribution header every Ghostex-adapted file must start
/// with (license hygiene gate).
pub const ATTRIBUTION_LINES: [&str; 3] = [
    "// Copyright (c) maddada",
    "// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500",
    "// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.",
];

/// True when `source` starts with exactly the attribution header.
pub fn attribution_ok(source: &str) -> bool {
    let mut lines = source.lines();
    ATTRIBUTION_LINES
        .iter()
        .all(|want| lines.next().is_some_and(|line| line == *want))
}

/// Scan `paths` for missing attribution; returns the offenders.
/// Empty means the license gate passes.
pub fn license_audit(paths: &[PathBuf]) -> Vec<PathBuf> {
    paths
        .iter()
        .filter(|p| {
            std::fs::read_to_string(p)
                .map(|src| !attribution_ok(&src))
                .unwrap_or(true)
        })
        .cloned()
        .collect()
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A fresh empty temp directory for one test case. Best-effort
/// cleanup is the caller's job.
pub fn fresh_temp_dir(tag: &str) -> PathBuf {
    let n = TEMP_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("phlow-wave30-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir must be creatable");
    dir
}

/// Write one scripted session fixture as `<stem>.session.json`.
pub fn write_session(dir: &Path, stem: &str, spec: &SessionSpec) -> PathBuf {
    let value = serde_json::json!({
        "id": spec.id,
        "title": spec.title,
        "project": spec.project,
        "transcript": spec.transcript,
        "started_at": spec.started_at,
    });
    let path = dir.join(format!("{stem}{SESSION_SUFFIX}"));
    std::fs::write(
        &path,
        serde_json::to_string(&value).expect("spec must serialize"),
    )
    .expect("session fixture must be writable");
    path
}

/// The shared 200-session ranking corpus: exactly one session carries
/// all three query words ("pairing", "brute", "force") as whole title
/// words; distractors carry subsets so the ranking is exercised.
/// Returns the target session's id.
pub fn pairing_corpus(dir: &Path) -> String {
    let target = SessionSpec::new(
        "target-pairing-brute-force",
        "pairing brute force",
        "weekly sync notes, nothing adversarial here",
    );
    write_session(dir, "target", &target);
    let vocab_a = [
        "pairing protocol notes",
        "pairing ceremony review",
        "device pairing flow",
    ];
    let vocab_b = [
        "brute force recovery",
        "brute-force ssh log",
        "force push safety",
    ];
    let filler = [
        "refactor the parser",
        "flaky test triage",
        "release checklist",
        "oncall handoff",
        "dependency audit",
        "tui layout pass",
    ];
    let mut n = 0;
    // Distractor transcripts carry all three query words as substrings
    // (so the AND prefilter keeps them) but never as whole title words
    // (so the title-exact target still outranks them).
    let distractor_transcript = "pairing review; brute force attempts seen in logs";
    for (i, title) in vocab_a.iter().chain(vocab_b.iter()).enumerate() {
        let spec = SessionSpec::new(&format!("distractor-{i:03}"), title, distractor_transcript);
        write_session(dir, &format!("d{i:03}"), &spec);
        n += 1;
    }
    let mut i = 0;
    while n < 199 {
        let spec = SessionSpec::new(
            &format!("filler-{i:04}"),
            filler[i % filler.len()],
            &format!("notes batch {i}"),
        );
        write_session(dir, &format!("f{i:04}"), &spec);
        i += 1;
        n += 1;
    }
    target.id
}
