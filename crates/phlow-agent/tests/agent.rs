//! Tests for `phlow-agent`: conversation context, the SQLite FTS5 memory
//! store, and the thin orchestrator. Mirrors `flow/agent/` behavior.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use phlow_agent::{
    ConversationContext, DEFAULT_MAX_MESSAGES, MemoryError, MemoryStore, Orchestrator,
    RESPONSE_CHARS_MAX,
};
use phlow_llm::transport::LlmTransport;
use phlow_runtime::Runtime;

// ---------------------------------------------------------------------------
// Conversation context
// ---------------------------------------------------------------------------

#[test]
fn context_defaults_to_40_messages() {
    let context = ConversationContext::default();
    assert_eq!(context.max_messages(), DEFAULT_MAX_MESSAGES);
    assert_eq!(context.max_messages(), 40);
    assert!(context.is_empty());
    assert_eq!(context.len(), 0);
}

#[test]
fn context_appends_and_lists_in_order() {
    let mut context = ConversationContext::new(10);
    context.add_message("user", "hello");
    context.add_message("assistant", "hi there");
    let messages = context.messages();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, "user");
    assert_eq!(messages[0].content, "hello");
    assert_eq!(messages[1].role, "assistant");
    assert_eq!(messages[1].content, "hi there");
}

#[test]
fn context_evicts_oldest_when_full() {
    let mut context = ConversationContext::new(3);
    for index in 0..5 {
        context.add_message("user", &format!("message {index}"));
    }
    let messages = context.messages();
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0].content, "message 2");
    assert_eq!(messages[2].content, "message 4");
}

#[test]
fn context_clear_empties() {
    let mut context = ConversationContext::new(5);
    context.add_message("user", "hello");
    context.clear();
    assert!(context.is_empty());
    assert_eq!(context.len(), 0);
    assert!(context.messages().is_empty());
}

#[test]
fn context_zero_cap_discards_everything() {
    let mut context = ConversationContext::new(0);
    context.add_message("user", "hello");
    assert!(context.is_empty());
}

// ---------------------------------------------------------------------------
// Memory store
// ---------------------------------------------------------------------------

/// Scratch database that deletes its file on drop, keeping `/tmp` bounded.
struct ScratchDb {
    path: PathBuf,
}

impl ScratchDb {
    /// Unique path per test and per process (PID + counter: reruns never
    /// collide with leftovers).
    fn new(name: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        let path = std::env::temp_dir().join(format!("phlow-agent-test-{name}-{pid}-{id}.sqlite"));
        Self { path }
    }

    fn open(&self) -> MemoryStore {
        MemoryStore::open(&self.path).expect("memory store opens")
    }
}

impl Drop for ScratchDb {
    fn drop(&mut self) {
        // WAL mode leaves `-wal`/`-shm` sidecars next to the database.
        let _ = std::fs::remove_file(&self.path);
        for suffix in ["-wal", "-shm"] {
            let mut sidecar = self.path.clone().into_os_string();
            sidecar.push(suffix);
            let _ = std::fs::remove_file(sidecar);
        }
    }
}

fn open_store(name: &str) -> (ScratchDb, MemoryStore) {
    let db = ScratchDb::new(name);
    let store = db.open();
    (db, store)
}

#[test]
fn memory_store_and_recent_round_trip() {
    let (_db, store) = open_store("round-trip");
    store
        .store("what is the capital?", "Paris", &["geography"])
        .expect("store works");
    let entries = store.recent(10).expect("recent works");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].query, "what is the capital?");
    assert_eq!(entries[0].response, "Paris");
    // ISO-8601 UTC timestamp like Python's `utcnow().isoformat()`.
    assert!(
        entries[0].created_at.contains('T'),
        "unexpected timestamp: {}",
        entries[0].created_at
    );
    assert!(
        entries[0].created_at.len() >= 19,
        "timestamp too short: {}",
        entries[0].created_at
    );
}

#[test]
fn memory_recent_returns_newest_first() {
    let (_db, store) = open_store("recent-order");
    store.store("first", "1", &[]).expect("store works");
    store.store("second", "2", &[]).expect("store works");
    store.store("third", "3", &[]).expect("store works");
    let entries = store.recent(10).expect("recent works");
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].query, "third");
    assert_eq!(entries[2].query, "first");
}

#[test]
fn memory_recent_respects_limit() {
    let (_db, store) = open_store("recent-limit");
    for index in 0..5 {
        store
            .store(&format!("q{index}"), &format!("r{index}"), &[])
            .expect("store works");
    }
    let entries = store.recent(2).expect("recent works");
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].query, "q4");
}

#[test]
fn memory_search_finds_by_fts() {
    let (_db, store) = open_store("search");
    store
        .store("how do I bake sourdough bread?", "Use a starter.", &[])
        .expect("store works");
    store
        .store("what is the capital of France?", "Paris.", &[])
        .expect("store works");
    let hits = store.search("sourdough", 5);
    assert_eq!(hits.len(), 1);
    assert!(
        hits[0].starts_with("Q: how do I bake sourdough bread?\nA: Use a starter."),
        "unexpected hit: {}",
        hits[0]
    );
}

#[test]
fn memory_search_truncates_hit_fields() {
    let (_db, store) = open_store("search-truncate");
    let long_query = "q".repeat(500);
    let long_response = "r".repeat(500);
    store
        .store(&format!("uniqueword {long_query}"), &long_response, &[])
        .expect("store works");
    let hits = store.search("uniqueword", 5);
    assert_eq!(hits.len(), 1);
    let query_line = hits[0].strip_prefix("Q: ").expect("Q prefix");
    let query_shown = query_line.split('\n').next().expect("one line");
    assert_eq!(query_shown.chars().count(), 100);
    let answer_shown = hits[0].split("A: ").nth(1).expect("A part");
    assert_eq!(answer_shown.chars().count(), 200);
}

#[test]
fn memory_search_respects_top_k() {
    let (_db, store) = open_store("search-topk");
    for index in 0..5 {
        store
            .store(&format!("commonword entry {index}"), "body", &[])
            .expect("store works");
    }
    assert_eq!(store.search("commonword", 2).len(), 2);
}

#[test]
fn memory_search_never_fails() {
    // FTS5 syntax errors (unbalanced quotes) and storage errors degrade to
    // an empty list, like Python's `except Exception: return []`.
    let (_db, store) = open_store("search-errors");
    store.store("hello world", "hi", &[]).expect("store works");
    assert!(store.search("\"unbalanced", 5).is_empty());
}

#[test]
fn memory_search_oversized_term_returns_empty() {
    let (_db, store) = open_store("search-term-bound");
    assert!(store.search(&"x".repeat(10_000), 5).is_empty());
}

#[test]
fn memory_clear_empties_store_and_search() {
    let (_db, store) = open_store("clear");
    store.store("hello world", "hi", &[]).expect("store works");
    assert_eq!(store.search("hello", 5).len(), 1);
    store.clear().expect("clear works");
    assert!(store.recent(10).expect("recent works").is_empty());
    assert!(store.search("hello", 5).is_empty());
}

#[test]
fn memory_store_rejects_oversized_fields() {
    let (_db, store) = open_store("bounds");
    let too_long = "x".repeat(20_000);
    assert!(matches!(
        store.store(&too_long, "ok", &[]),
        Err(MemoryError::TooLong { field: "query", .. })
    ));
    assert!(matches!(
        store.store("ok", &"y".repeat(200_000), &[]),
        Err(MemoryError::TooLong {
            field: "response",
            ..
        })
    ));
    assert!(matches!(
        store.store("ok", "ok", &["t".repeat(100).as_str()]),
        Err(MemoryError::TooLong { field: "tag", .. })
    ));
    let many_tags: Vec<&str> = (0..100).map(|_| "tag").collect();
    assert!(matches!(
        store.store("ok", "ok", &many_tags),
        Err(MemoryError::TooManyTags { .. })
    ));
}

#[test]
fn memory_tags_persist_as_json() {
    let db = ScratchDb::new("tags-json");
    let path = db.path.clone();
    let store = MemoryStore::open(&path).expect("memory store opens");
    store
        .store("q", "r", &["alpha", "beta"])
        .expect("store works");
    drop(store);
    // Tags round-trip through the JSON column: read them back raw.
    let connection = rusqlite::Connection::open(&path).expect("reopen works");
    let tags: String = connection
        .query_row("SELECT tags FROM memories", [], |row| row.get(0))
        .expect("tags read");
    assert_eq!(tags, r#"["alpha","beta"]"#);
}

#[test]
fn memory_schema_matches_python() {
    // Table, FTS5 index, and insert trigger names mirror `flow/agent/memory.py`.
    let db = ScratchDb::new("schema");
    let path = db.path.clone();
    let store = MemoryStore::open(&path).expect("memory store opens");
    drop(store);
    let connection = rusqlite::Connection::open(&path).expect("reopen works");
    let mut names: Vec<String> = connection
        .prepare("SELECT name FROM sqlite_master WHERE type IN ('table', 'trigger') ORDER BY name")
        .expect("schema query prepares")
        .query_map([], |row| row.get(0))
        .expect("schema query runs")
        .filter_map(|name| name.ok())
        .collect();
    names.sort();
    assert!(names.contains(&"memories".to_owned()), "tables: {names:?}");
    assert!(
        names.contains(&"memories_fts".to_owned()),
        "tables: {names:?}"
    );
    assert!(
        names.contains(&"memories_ai".to_owned()),
        "tables: {names:?}"
    );
}

// ---------------------------------------------------------------------------
// Memory store: concurrency and crash safety
// ---------------------------------------------------------------------------

/// Crash-test writer's database path; set only in the spawned child.
const CRASH_DB_ENV: &str = "PHLOW_AGENT_CRASH_DB";
/// Rows the crash-test writer attempts before exiting on its own.
const CRASH_WRITER_ROWS_MAX: u32 = 20_000;
/// Rows the parent waits to see committed before killing the writer.
const CRASH_ROWS_BEFORE_KILL: i64 = 50;
/// Kill/recover cycles against one database file.
const CRASH_CYCLES: u32 = 5;

/// The response a crash-test row must carry: any other value is torn.
fn crash_response(query: &str) -> String {
    format!("crashbody {}", query.repeat(64))
}

#[test]
fn memory_open_enables_wal() {
    let db = ScratchDb::new("wal");
    drop(db.open());
    // journal_mode=WAL persists in the file header, so a fresh
    // connection observes it.
    let connection = rusqlite::Connection::open(&db.path).expect("reopen works");
    let mode: String = connection
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .expect("journal mode reads");
    assert_eq!(mode, "wal");
}

#[test]
fn memory_open_twice_is_idempotent() {
    let db = ScratchDb::new("idempotent");
    let first = db.open();
    first
        .store("kept query", "kept answer", &[])
        .expect("store works");
    // A second live handle re-runs the schema DDL while the first is open.
    let second = db.open();
    second
        .store("second query", "second answer", &[])
        .expect("store works");
    drop(first);
    drop(second);
    let reopened = db.open();
    assert_eq!(reopened.recent(10).expect("recent works").len(), 2);
    assert_eq!(reopened.search("kept", 5).len(), 1);
}

#[test]
fn memory_wal_rejects_in_memory_database() {
    // Adversarial: SQLite's in-memory database cannot do WAL; open must
    // refuse rather than silently run without concurrency safety.
    let result = MemoryStore::open(std::path::Path::new(":memory:"));
    assert!(
        matches!(result, Err(MemoryError::WalUnavailable { .. })),
        "unexpected: {:?}",
        result.err()
    );
}

#[test]
fn memory_concurrent_readers_and_writer_never_busy() {
    // Adversarial: one writer plus four readers on separate connections.
    // Every call must succeed; an escaped SQLITE_BUSY fails the test.
    // Near-maximal responses keep each commit (and its lock) long enough
    // that a rollback journal without a busy timeout reliably collides.
    const WRITES: u32 = 300;
    const READERS: usize = 4;
    const READS_PER_READER: u32 = 300;
    let db = ScratchDb::new("concurrent");
    drop(db.open());
    let path = db.path.clone();
    let response = "answer ".repeat(RESPONSE_CHARS_MAX / 8);
    std::thread::scope(|scope| {
        let writer = scope.spawn(|| {
            let store = MemoryStore::open(&path).expect("writer opens");
            for index in 0..WRITES {
                store
                    .store(&format!("sharedword {index}"), &response, &[])
                    .expect("write never sees SQLITE_BUSY");
            }
        });
        let readers: Vec<_> = (0..READERS)
            .map(|_| {
                scope.spawn(|| {
                    let store = MemoryStore::open(&path).expect("reader opens");
                    for _ in 0..READS_PER_READER {
                        store.recent(20).expect("read never sees SQLITE_BUSY");
                    }
                })
            })
            .collect();
        writer.join().expect("writer thread finishes");
        for reader in readers {
            reader.join().expect("reader thread finishes");
        }
    });
    let store = db.open();
    assert_eq!(
        store.recent(1_000).expect("recent works").len(),
        WRITES as usize
    );
    assert_eq!(store.search("sharedword", 100).len(), 100);
}

/// Child half of the crash test: writes rows until killed. Runs only when
/// spawned by [`memory_survives_kill_9_mid_insert`] (env var set).
#[test]
#[ignore = "helper process for memory_survives_kill_9_mid_insert"]
fn memory_crash_writer_child() {
    let Some(path) = std::env::var_os(CRASH_DB_ENV) else {
        return;
    };
    let store = MemoryStore::open(std::path::Path::new(&path)).expect("child opens");
    // PID-qualified: rows stay unique across kill/recover cycles.
    let pid = std::process::id();
    for index in 0..CRASH_WRITER_ROWS_MAX {
        let query = format!("crashrow{pid}x{index}");
        store
            .store(&query, &crash_response(&query), &["crash"])
            .expect("child store works");
    }
}

/// Count committed rows through a fresh connection.
fn committed_rows(path: &std::path::Path) -> i64 {
    let Ok(connection) = rusqlite::Connection::open(path) else {
        return 0;
    };
    let _ = connection.busy_timeout(std::time::Duration::from_secs(5));
    connection
        .query_row("SELECT count(*) FROM memories", [], |row| row.get(0))
        .unwrap_or(0)
}

/// Spawn the writer child, wait for committed rows, then SIGKILL it.
fn crash_writer_once(path: &std::path::Path) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let baseline = committed_rows(path);
    let mut child = std::process::Command::new(std::env::current_exe().expect("test exe"))
        .args([
            "--exact",
            "memory_crash_writer_child",
            "--ignored",
            "--nocapture",
        ])
        .env(CRASH_DB_ENV, path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("child spawns");
    while committed_rows(path) < baseline + CRASH_ROWS_BEFORE_KILL {
        assert!(std::time::Instant::now() < deadline, "writer child stalled");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    // `Child::kill` is SIGKILL on Unix: no destructors, no clean close.
    child.kill().expect("SIGKILL delivered");
    let status = child.wait().expect("child reaped");
    // A clean exit means the writer finished first: no crash was tested.
    assert_eq!(
        std::os::unix::process::ExitStatusExt::signal(&status),
        Some(9),
        "writer was not killed mid-run: {status}"
    );
}

#[test]
fn memory_survives_kill_9_mid_insert() {
    let db = ScratchDb::new("crash");
    drop(db.open());
    for _ in 0..CRASH_CYCLES {
        crash_writer_once(&db.path);
    }
    let connection = rusqlite::Connection::open(&db.path).expect("reopen works");
    let integrity: String = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .expect("integrity check runs");
    assert_eq!(integrity, "ok");
    // FTS5 external-content check: index agrees with `memories` exactly.
    connection
        .execute(
            "INSERT INTO memories_fts(memories_fts) VALUES('integrity-check')",
            [],
        )
        .expect("fts index matches content");
    drop(connection);
    let store = db.open();
    let entries = store.recent(1_000).expect("recent works");
    assert!(
        entries.len() >= (CRASH_ROWS_BEFORE_KILL as usize),
        "rows: {}",
        entries.len()
    );
    for entry in &entries {
        assert_eq!(
            entry.response,
            crash_response(&entry.query),
            "torn row surfaced"
        );
    }
    // Every surviving row is findable: no row committed without its index.
    let newest = &entries[0].query;
    assert_eq!(
        store.search(newest, 5).len(),
        1,
        "newest row missing from FTS"
    );
}

// ---------------------------------------------------------------------------
// Orchestrator
// ---------------------------------------------------------------------------

/// Minimal scripted LLM transport driving the runtime through the
/// orchestrator.
struct ScriptLlm {
    responses: std::collections::VecDeque<serde_json::Value>,
}

impl ScriptLlm {
    fn new(responses: Vec<serde_json::Value>) -> Self {
        Self {
            responses: responses.into(),
        }
    }

    fn chat_response(content: &str) -> serde_json::Value {
        serde_json::json!({
            "choices": [{"message": {"role": "assistant", "content": content}}]
        })
    }
}

impl LlmTransport for ScriptLlm {
    fn post_chat(
        &mut self,
        _base_url: &str,
        _payload: &serde_json::Map<String, serde_json::Value>,
        _timeout: std::time::Duration,
    ) -> Result<serde_json::Value, phlow_llm::LlmError> {
        self.responses
            .pop_front()
            .ok_or_else(|| phlow_llm::LlmError::Transport("script exhausted".to_owned()))
    }

    fn get_tags(
        &mut self,
        _base_url: &str,
        _timeout: std::time::Duration,
    ) -> Result<serde_json::Value, phlow_llm::LlmError> {
        Ok(serde_json::json!({"models": []}))
    }

    fn close(&mut self) {}
}

/// Editor transport that reports no editor (socket absent).
struct NoEditor;

impl phlow_editor::EditorTransport for NoEditor {
    fn exec(
        &mut self,
        _expression: &str,
        _args: &[serde_json::Value],
        _timeout: std::time::Duration,
    ) -> Result<serde_json::Value, phlow_editor::TransportError> {
        Err(phlow_editor::TransportError::Failed("no editor".to_owned()))
    }

    fn close(&mut self) {}
}

fn test_config(dir: &std::path::Path) -> phlow_config::FlowConfig {
    let toml =
        "[checks.smoke]\ncmd = [\"true\"]\nkind = \"lint\"\nrequired = true\nfiletypes = [\"*\"]\n";
    let path = dir.join("phlow.toml");
    std::fs::write(&path, toml).expect("config writes");
    phlow_config::load_config(&phlow_config::LoadOptions {
        config_path: Some(path),
        workspace: Some(dir.to_path_buf()),
        trusted: true,
        model: None,
    })
    .expect("config loads")
}

#[test]
fn orchestrator_delegates_run_to_runtime() {
    let dir = std::env::temp_dir().join("phlow-agent-orchestrator");
    std::fs::create_dir_all(&dir).expect("workspace");
    let config = test_config(&dir);
    let llm = ScriptLlm::new(vec![
        ScriptLlm::chat_response("Plan: nothing to do."),
        ScriptLlm::chat_response("Done. No changes needed."),
        ScriptLlm::chat_response(r#"{"approved": true, "summary": "ok", "issues": []}"#),
    ]);
    let runtime = Runtime::new(config, llm, NoEditor, None).expect("runtime builds");
    let mut orchestrator = Orchestrator::new(runtime);
    let report = orchestrator.run("do nothing");
    assert_eq!(report["status"], serde_json::json!("ok"));
    assert_eq!(report["verified"], serde_json::json!(true));
    orchestrator.close();
}
