# The Run Contract — fleet reference (INDEX §2.1)

Status: reference implementation landed 2026-11-10 (PH-11) in
`phlow-checks::run_contract`, written down from phlow's existing
discipline — the check runner in `phlow-checks::runner` already runs
every check in its own process group and kills and reaps the whole
group on timeout. This document is the fleet contract other servers
adopt; their K1 test kits copy `crates/phlow-checks/tests/run_contract_k1.rs`.

These are file-and-process conventions, not a new server.

## 1. What is a run

Every operation that can outlive a tool call is a **run**. A run is a
directory under the server's state root (for phlow:
`~/.local/share/phlow/runs/` by convention; the module takes the root
as a parameter), named by its run id.

## 2. Contract fields

| Field | Rule |
|---|---|
| `run_id` | Unique; names the run's directory. `[A-Za-z0-9._-]`, starts alphanumeric, never contains `..`, ≤ 64 chars (`RUN_ID_CHARS_MAX`). Used exactly once: creating a second run with the same id fails. |
| `deadline` | Absolute Unix-ms timestamp in `run.json`, set at start, **never extended by the run itself**. |
| `heartbeat` | A file in the run directory whose content is the last heartbeat Unix-ms, rewritten atomically (sibling temp + rename) at the interval stated in `run.json`. Stale = age > 2× the interval (`HEARTBEAT_STALE_FACTOR`); a missing heartbeat is stale. A stale heartbeat means dead. |
| `process group` | Every child of the run is in the run's own process group (spawn with `process_group(0)`), and the group id is recorded in `run.json`, so one kill reaps the whole tree. A leader-only kill is a contract violation: grandchildren are exactly the orphans this contract exists to prevent. |
| `done-marker` | `done.json`, written **last, atomically**, with the exit state — `done`, `failed`, or `abandoned` — and the SHA-256 of every artifact the run claims (`artifacts: [{name, sha256}]`, ≤ 256). A run ends exactly once: writing a second marker fails. |

`run.json` shape (all fields required on read; unknown states and
malformed files are refused by name, never defaulted):

```json
{"run_id": "…", "started_unix_ms": 0, "deadline_unix_ms": 0,
 "heartbeat_interval_ms": 0, "pgid": 0}
```

## 3. Reaping rule

Anyone may reap a run whose deadline has passed or whose heartbeat is
stale: **kill the process group, then write an `abandoned`
done-marker** (empty artifact list unless the reaper can still hash
the artifacts). The decision table is pure and shared
(`decide_reap`): done-marker present → `AlreadyDone`; past deadline →
`DeadlinePassed`; heartbeat stale/missing → `HeartbeatStale`;
otherwise `Alive`. Deadline wins over staleness.

Reaping is a **single bounded pass** over the state root
(`reap_expired_runs`, ≤ 1024 run directories per pass). Directories
that do not parse as runs are named in the pass result's `skipped`
list, never silently dropped.

## 4. No waiter loops

No run is ever *waited on*. A consumer reads the done-marker or does
other work. A server never polls another server in a loop; to hand
work over it leaves a cue (INDEX §2.2; phlow's primitives are in
`phlow-checks::cue`) and returns. The reaper does not poll either: it
decides from files, kills once, and leaves — proving the group died
is the K1 kit's assertion job, not the reaper's loop.

## 5. K1 — the contract test kit

One per server. Phlow's kit (`tests/run_contract_k1.rs`) is the
template:

1. **Start a run** — a process tree (shell + two grandchildren) in its
   own process group, with a run record and a deadline.
2. **Kill the server** — drop the only handle without waiting: an
   orphaned group, a run directory, no done-marker.
3. **Assert** — one reaping pass at the deadline marks the run
   `abandoned`, and no process of its group survives (checked via
   `/proc` group membership, so a leader-only kill fails the kit).

A second test covers the heartbeat path: heartbeats stop, the run is
reaped `abandoned` long before its deadline.

## 6. Evidence

Why this contract exists, from the night it was written (INDEX §2.1):
twelve headless diagnostics producers counted alive at once; a GPU
queue held together by hand-written done-files; ~145 GB of agent
build output in one tree. Every one of those is a run nobody could
reap because nothing recorded its group or its deadline.
