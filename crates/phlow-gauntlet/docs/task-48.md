# task-48: disk quota enforcement

**Kind:** rust · **Status:** fail (seam absent — no per-run quota, no `QuotaExceeded`, no cumulative write accounting; banked product decision) · **Wave:** 46–50 · **Commits:** pending (wave 46-50)

## ELI5

Give the agent a disk budget for each run — say, 100 MB. Every
time it writes a file, the budget ticks down. If a write would
push it over, the write fails *immediately* with a clear typed
error (`QuotaExceeded`), nothing is left half-written, and the
agent learns its exact remaining balance. The adversarial
scenarios are the nasty edges: a write that starts fine and
blows the budget mid-stream (fail the moment the cap is crossed,
keep the partial file from looking complete), and a zero budget
(flat refusal, no filesystem touch at all).

phlow has no such budget. There is no per-run quota, no
`QuotaExceeded` type, no cumulative write-byte accounting, and no
artifact writer that enforces a cap. The closest thing in the
tree is `phlow-workspace::FILE_BYTES_MAX` — a compile-time cap
on *reads* (how big a file may be when the agent looks at it),
which is the opposite direction: it bounds what comes in, not
what goes out, and it is not per-run. The design's weapons —
mid-write blowout, zero quota — have no target.

## What this task attempts

- **Goal:** locate the per-run disk quota in phlow's Rust crates
  and run the design's scenarios: mid-write quota blowout (typed
  `QuotaExceeded` the moment the cap is crossed, partial files
  not left behind as complete) and zero quota (immediate flat
  refusal).
- **Mechanism:** the `task_48.rs` driver probes the live working
  tree — a runtime tokenized vocabulary scan over every
  `crates/*/src/**/*.rs` for quota tokens (`quota`,
  `disk_quota`, `quota_bytes`, `quota_exceeded`, `bytes_written`,
  `per_run_quota`) and cumulative write-accounting tokens
  (`bytes_written`, `write_bytes`, `total_bytes_written`,
  `bytes_remaining`), plus a scan for any typed
  quota-exceeded-shaped error. Tokens are matched exactly
  (underscores preserved), so `FILE_BYTES_MAX` does not
  false-positive on `quota`.
- **Success criterion:** a per-run quota exists, mid-write
  blowout raises a typed refusal with partial-file cleanup, and
  zero quota refuses without touching the filesystem.
- **Non-goals:** building the quota. Whether artifact,
  transcript, and checkpoint writers should gain per-run quotas
  with partial-file cleanup is banked for Matt as a product
  decision, not auto-implemented.

## What happened

Fail at `"seam"` — on the first and only attempt, honestly.
There is no quota to blow past:

- `no_quota_tokens_in_sources` (V): the quota vocabulary scan
  returns zero hits across the workspace — no per-run quota, no
  `QuotaExceeded`, no settable quota on any writer. The
  `FILE_BYTES_MAX` read cap is the closest bound in the tree and
  it is a read cap, not a write quota.
- `write_paths_have_no_byte_accounting` (V): no writer tracks
  cumulative run bytes against a cap — a per-run quota would
  have nothing to debit.
- `mid_write_quota_exceeded_has_no_target` (A): no typed
  quota-exceeded-shaped error exists anywhere, so a mid-write
  blowout has no refusal to raise and no cleanup contract to
  test.
- `zero_quota_fast_fail_has_no_target` (A): with no settable
  quota, a zero-budget refusal has no API to call.

## The fix — what changed and why

Nothing changed: the seam is absent, so there is nothing to fix
without a product decision. The driver is new in this wave
(`src/tasks/task_48.rs`), plus four integration tests
(`tests/task_48.rs`). No production code was touched.

## Full technical depth

The probe's tokenizer preserves underscores (splits only on
non-alphanumeric characters that are not `_`), so snake_case
identifiers match exactly and `FILE_BYTES_MAX`-style constants
do not false-positive on the `quota` token. The probe's own
source file is path-excluded because the task NAME contains the
design vocabulary. Scans are bounded (1 MiB files, 50k files)
and read the live working tree, never a cached copy.

What a real implementation would need (banked for Matt, not
implemented here): a per-run write budget threaded through the
artifact/transcript/checkpoint writers, cumulative byte
accounting debited on every write, a typed `QuotaExceeded`
raised the moment a write would cross the cap, partial-file
cleanup or never-visible partial files, and a zero-quota fast
path that refuses before touching the filesystem. The read-side
`FILE_BYTES_MAX` precedent shows the codebase already accepts
compile-time byte caps — the write side has no equivalent.

## Sources

- Primary: the live working tree — `crates/*/src/**/*.rs`
  (vocabulary scans: zero quota hits, zero accounting hits,
  zero quota-exceeded types).
- Primary: `crates/phlow-workspace` (`FILE_BYTES_MAX`:
  compile-time *read* cap — the closest bound, wrong direction).
- Driver: `crates/phlow-gauntlet/src/tasks/task_48.rs`
  (bounded live-workspace probe).
- Tests: `crates/phlow-gauntlet/tests/task_48.rs` (2V/2A).
