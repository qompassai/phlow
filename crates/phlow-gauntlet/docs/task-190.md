# task-190: corrupt index rebuild

**Kind:** rust · **Status:** pass · **Wave:** 30 · **Commits:** pending (wave 30)

## ELI5

Databases live in files, and files can rot — a crash mid-write can
leave the index file with a smashed header or cut off halfway
through. The engine must never do the worst thing: open a damaged
file and quietly serve half-true answers. Instead, every open
starts with a health check (`PRAGMA integrity_check`); if the file
fails, the engine reports a typed "corrupt" error, moves the bad
file aside (quarantine, so it can't be mistaken for good), and
rebuilds the index from scratch by re-reading the session files.
This task smashes a 20-session index two ways — zeroing the first
4 KiB and truncating the file mid-table — and proves both paths
detect, quarantine, rebuild, and serve correct queries after.

## What this task attempts

- **Goal:** prove a corrupt index fails closed with a typed error,
  quarantines, rebuilds from a fresh scan, and is queryable after.
- **Mechanism:** `session_find.rs` (`SessionIndex::open_raw`,
  integrity probe, `open_or_rebuild`, quarantine);
  driver `src/tasks/task_190.rs`.
- **Success criterion:** both corruption modes → `open_strict`
  returns `Err(IndexError::Corrupt(_))` (never `Ok`, never another
  error kind); quarantine file exists and the live path is gone;
  rebuild yields 20/20 rows; all 20 sessions queryable by token;
  rebuild scan completes within 10× the clean-scan time.
- **Non-goals:** concurrent access during rebuild (task 192),
  hostile session *content* (task 189).

## What happened

Pass on the first executable gate run (the wave's shared compile blockers were fixed before any test executed). Integration tests 2/2:
`zeroed_header_rebuilds` and `truncated_midtable_rebuilds` — both
show the typed `IndexError::Corrupt`, quarantine, 20/20 rows, and
20/20 queryable after rebuild. ~0.01s on primo.

## Where it went wrong

One genuine pre-gate bug in the engine, found by reading the open
path before writing the driver:

- **Stage:** `SessionIndex::open_strict` construction.
- **Symptom (anticipated, then verified by test):** the first draft
  configured WAL mode and created the schema *before* running the
  integrity check. On a corrupt file, SQLite can surface the damage
  as a generic SQL error during setup — or, worse, the WAL replay
  can mask the corruption — so the typed `Corrupt` path would not
  reliably fire.
- **Root cause:** ordering. The integrity probe must run on the raw
  file before any setup that could alter or reinterpret it.

(The pre-gate E0521 lifetime fix is documented in task-186.md.)

## The fix — what changed and why

- **Changed:** `src/session_find.rs`: split the open path into
  `open_raw` (opens the connection, runs `PRAGMA integrity_check`
  on the untouched file) → on failure return
  `IndexError::Corrupt`; on success `prepare` (WAL mode, busy
  timeout, schema creation). `open_or_rebuild` quarantines on
  `Corrupt` and re-scans into a fresh file.
- **Commit:** pending (wave 30).
- **Why:** check-before-touch is the only ordering that guarantees
  the typed error. Alternatives — catching generic SQL errors and
  reclassifying — would mislabel real SQL bugs as corruption.
  Deleting the corrupt file instead of quarantining was rejected:
  quarantine preserves evidence and prevents the "file vanished"
  mystery.
- **Source:** SQLite `PRAGMA integrity_check` documentation (returns
  one row `ok` when healthy, error text otherwise); rusqlite
  connection/pragma APIs.
- **Validation agents:** `cargo test -p phlow-gauntlet --test task_190`
  on primo → 2/2 pass; both corruption modes show `Corrupt`, not a
  generic `Sql` error.
- **Adversarial agents:** the two corruption modes are the
  adversarial half — header smash (the most common real-world
  damage) and mid-table truncation (the case most likely to pass a
  naive header-only check, since page 1 can survive while later
  pages are gone).
- **Citations:** SQLite `PRAGMA integrity_check`
  (https://www.sqlite.org/pragma.html#pragma_integrity_check);
  rusqlite 0.37.0 docs (https://docs.rs/rusqlite/0.37.0/rusqlite/).

## Full technical depth

`open_raw` opens the SQLite file with no setup at all and runs
`PRAGMA integrity_check`, requiring the single-row `ok` answer.
Any deviation — an error, or rows of corruption text — becomes
`IndexError::Corrupt(String)` carrying SQLite's own diagnostic.
Only after `ok` does `prepare` run `PRAGMA journal_mode=WAL`,
`busy_timeout`, and the `CREATE TABLE IF NOT EXISTS` schema.

`open_or_rebuild(sessions_root, db_path)` calls `open_strict`;
on `Corrupt` it renames the file to `<name>.corrupt-<unix_ts>`
(same directory, so no cross-filesystem move), then runs
`scan_and_index` into the original path — a genuinely fresh scan,
not a repair — and returns the recovered handle plus a
`Recovered { rebuilt: true, quarantined: Some(path), stats }`.

The driver corrupts two ways: (1) zero the first 4096 bytes
(SQLite's page 1 holds the 100-byte header: magic, page size,
schema cookie — zeroing it guarantees `integrity_check` fails);
(2) truncate the file to 60% of its size (page 1 intact, later
btree pages gone — defeats any header-only check). After damaging
the main file, the driver removes any `-wal`/`-shm`/`-journal`
sidecars so SQLite cannot replay an intact WAL over the damage
and hide it — a subtlety that would otherwise make the test pass
for the wrong reason.

The timing bound (rebuild ≤ 10× clean scan) is deliberately loose:
it exists to catch pathological rebuild loops, not to benchmark.
For a 20-file corpus both scans are sub-millisecond; the bound is
checked, not stressed.

## Sources

- Primary: `crates/phlow-gauntlet/src/session_find.rs`
  (`SessionIndex::open_raw`, `open_strict`, `open_or_rebuild`,
  `quarantine_path`, `IndexError::Corrupt`);
  `crates/phlow-gauntlet/src/tasks/task_190.rs`;
  `crates/phlow-gauntlet/tests/task_190.rs`.
- Secondary: SQLite `PRAGMA integrity_check` docs
  (https://www.sqlite.org/pragma.html#pragma_integrity_check);
  SQLite file format (page-1 header layout).
