# task-186: full index build + incremental rescan

**Kind:** rust · **Status:** pass · **Wave:** 30 · **Commits:** pending (wave 30)

## ELI5

phlow keeps a folder of past session files — one JSON file per session,
with a title, a project name, and a transcript. Searching them by
re-reading every file on each keystroke would be slow, so the engine
reads each file once and copies the important fields into a SQLite
database (the "index"). After that, searches ask the database, not the
files. The clever part: the engine remembers each file's size and
modification time, so on the next scan it only re-reads files that
actually changed. This task proves two things: a 500-file folder gets
fully indexed and every session is findable, and a rescan after
changing 4 files reads exactly those 4 — not the other 496.

## What this task attempts

- **Goal:** prove the scan-once index builds completely and rescans
  incrementally by `(mtime, size)`.
- **Mechanism:** `crates/phlow-gauntlet/src/session_find.rs`
  (`scan_and_index`, `SessionIndex::open_strict`, `SessionIndex::search`,
  `FsLog` read counter); driver
  `crates/phlow-gauntlet/src/tasks/task_186.rs`.
- **Success criterion:** 500 synthetic sessions → 500 rows, all 500
  queryable by unique token, mtimes recorded; then +3 files / +1
  modification → rescan reads exactly 4 files per the `FsLog` counter.
- **Non-goals:** ranking quality (task 187), foreign trees (task 188),
  adversarial input (tasks 189–191), concurrency (task 192).

## What happened

Pass on the first executable gate run (the wave's shared compile blockers were fixed before any test executed). Integration tests 2/2:
`full_build_queryable` (500 files → 500 rows, 500/500 queried OK,
5/5 mtimes match) and `incremental_rescan_four_reads` (rescan read
exactly 4 files: the 3 new plus the 1 modified; rows 503). The
evidence names the 4 re-read files from the fs-access log, proving
the other 499 were skipped. Total run time ~0.2s on primo.

## Where it went wrong

Two pre-gate compile failures, both in the driver scaffolding, not
the engine:

- **Stage:** `cargo build -p phlow-gauntlet`.
- **Symptom:** `error[E0521]: borrowed data escapes outside of function`
  — `CaseReport::pass` takes `case: &'static str` (see
  `src/skillopt/driver.rs:255`), but `run_case(case: &str)` forwarded
  a short-lived borrow.
- **Evidence:** build log on primo, repeated for every new task module.
- **Root cause:** the driver signature did not match the
  `CaseReport::pass` contract; `&str` borrows from the caller cannot
  satisfy `'static`.

A second, wave-level failure hit before any of this compiled:

- **Stage:** dependency resolution (`cargo build`).
- **Symptom:** `error: failed to select a version for libsqlite3-sys`
  — "package libsqlite3-sys v0.35.0 ... links to the native library
  sqlite3, but it conflicts with a previous package which links to
  sqlite3 as well".
- **Root cause:** the first attempt pinned `rusqlite 0.40` (latest on
  crates.io), but the workspace already contains
  `phlow-self-improve` with `rusqlite = "=0.37.0"`. Cargo's `links =
  "sqlite3"` rule forbids two copies of the native library in one
  graph.

## The fix — what changed and why

- **Changed:** `crates/phlow-gauntlet/src/tasks/task_186.rs` (and all
  wave-30 drivers + tests): `run_case(case: &str)` →
  `run_case(case: &'static str)`; test helpers
  `check_case(case: &str)` → `check_case(case: &'static str)`.
- **Commit:** pending (wave 30).
- **Why:** the `'static` bound is the honest contract — case names are
  string literals. Alternatives (leaking the string, changing
  `CaseReport`) would fight the existing scaffold for no benefit.
- **Source:** `src/skillopt/driver.rs:255`
  (`pub fn pass(case: &'static str, ...)`), plus the task-131 driver
  exemplar which uses the same shape.
- **Validation agents:** `cargo test -p phlow-gauntlet --test task_186`
  on primo → 2/2 pass.
- **Adversarial agents:** none beyond the gate suite; the fix is a
  signature change, reviewed by re-reading every `run_case` call site
  (7 drivers + 7 tests + `mod.rs` dispatch) for lifetime misuse.
- **Citations:** rustc E0521 diagnostic; Cargo reference on `links`
  (https://doc.rust-lang.org/cargo/reference/resolver.html#links).

- **Changed:** `crates/phlow-gauntlet/Cargo.toml`:
  `rusqlite = "0.40"` → `rusqlite = { version = "=0.37.0", features =
  ["bundled"] }`, with a comment recording the pin rationale.
- **Why:** workspace uniformity beats newest-version: one
  `libsqlite3-sys` in the graph, same as `phlow-self-improve`
  (`crates/phlow-self-improve/Cargo.toml:13`). `bundled` keeps the
  no-system-sqlite property. The engine uses only long-stable
  rusqlite APIs (`open`, `pragma_update`, `execute_batch`,
  `transaction`, `params_from_iter`), so 0.37.0 loses nothing.
- **Source:** `crates/phlow-self-improve/Cargo.toml:9-13`
  ("same pinned version as phlow-agent's memory store"); Cargo's
  `links` conflict error text.

## Full technical depth

The corpus is 500 synthetic `*.session.json` files, each with a
unique token in its transcript. `scan_and_index` walks the session
root (skipping dot-directories and non-`*.session.json` files),
reads each file once through an `FsLog` (a scripted double that
counts real reads), and upserts `(id, title, project, transcript,
mtime, size, truncated)` into SQLite. The manifest of `(mtime,
size)` per file is read from the `meta` table before the scan, so
unchanged files skip the read entirely — the `continue` happens
before `fs_log.record`. Every write goes through one parameterized
template (`INSERT ... ON CONFLICT(path) DO UPDATE`), so no session
content is ever interpolated into SQL.

The rescan path re-opens the same database, re-reads the manifest,
and compares `(mtime, size)` per file: the 3 new files are absent
from the manifest (read), the modified file's size differs (read),
the other 496 match (skipped). The generation counter bumps once
per rescan inside the same transaction, so readers see exactly one
state change. The driver asserts `FsLog.reads().len() == 4` and
that the read paths are exactly the 4 expected files — a file that
was re-read without being in the changed set would fail the test.

Queryability is proven by searching each of the 500 unique tokens
and requiring the owning session to be a hit. mtime correctness is
spot-checked on 5 files against `std::fs::metadata`.

## Sources

- Primary: `crates/phlow-gauntlet/src/session_find.rs`
  (`scan_and_index`, `SessionIndex`, `FsLog`, `SqlLog`);
  `crates/phlow-gauntlet/src/tasks/task_186.rs`;
  `crates/phlow-gauntlet/tests/task_186.rs`;
  `crates/phlow-gauntlet/src/skillopt/driver.rs:255`;
  `crates/phlow-self-improve/Cargo.toml:9-13`.
- Secondary: Ghostex `packages/find/src/index.rs` (the "scan once"
  pattern this adapts); Cargo `links` reference.
