# task-131: scheduled scope refresh

**Kind:** rust · **Status:** pass · **Wave:** 23 · **Commits:** <worktree commit on gate>

## ELI5

A bug-bounty program's scope changes over time — targets get added and
dropped. The agent can't work from a stale list, so it polls the scope
feed on a timer and files each new snapshot as a new version. This task
checks the front door of the cycle: polls fire on schedule, unchanged
polls don't create fake new versions, a broken poll doesn't clobber the
good scope, and a big clock jump doesn't cause a storm of catch-up
polls. Think of it like a newspaper subscription: today's edition
arrives once, yesterday's isn't reprinted, a torn copy doesn't replace
the good one, and if your clock was wrong you get one catch-up edition —
not sixty.

## What this task attempts

- **Goal:** verify the poll timer → `ScopeStore.file()` seam end to end
  with scripted doubles.
- **Mechanism:** driver-local `poller_tick` in
  `crates/phlow-gauntlet/src/tasks/task_131.rs` drives
  `bounty::feed::ScriptedFeed` (MOCK) and `bounty::clock::ManualClock`
  (MOCK) into `bounty::store::ScopeStore::file()`.
- **Success criterion:** 3 scripted polls (v1, v1, v2) yield
  latest-versions `[1,1,2]` and version counts `[1,1,2]`;
  `fetched_at` monotonic; malformed poll → typed
  `FeedError::Malformed` with scope retained at v1; +3600s clock jump
  → exactly 1 catch-up poll, gapless versions.
- **Non-goals:** real bbscope wire compatibility (the feed is
  "bbscope-style", not "bbscope-compatible"); diffing between
  snapshots (task 132); acting on removals (task 133).

## What happened

Pass on the second attempt. The first compile failed: `task_131.rs`
used the `Clock` trait without importing it. The four cases (all
passing after the import fix):

- `versioned_polls`: 3 scripted polls at 60s cadence → latest versions
  `[1,1,2]`, version counts `[1,1,2]`. The unchanged middle poll hit
  `ScopeStore::file`'s idempotent path (`Ok(false)`) — nothing new
  filed.
- `fetched_at_monotonic`: filed versions `(1, t0)` and `(2, t0+120)`,
  monotonic, each stamped with its poll tick time.
- `malformed_poll_retains_scope`: poll 2 raised
  `FeedError::Malformed` (typed, logged); `latest().version` stayed 1,
  version count stayed 1; the next poll's repeat of v1 was idempotent.
- `clock_jump_single_catchup`: after a +3600s jump, exactly 1 poll
  fired (bar: ≤2), versions `[1,2]` gapless, and the timer recovered
  to the 60s cadence on the next tick.

## The fix — what changed and why

One fix iteration: added the missing `use ...::Clock` import to
`src/tasks/task_131.rs` (the driver calls `Clock` methods on the
manual clock), then ran `cargo fmt` on the new files. No test logic
changed — the four cases passed on the next run.

## Full technical depth

The poller is a driver-local timer loop, not a scaffold type (the
scaffold's `sched.rs` holds the *probe* scheduler; no `ScopePoller`
exists in the landed scaffold — the design doc's scaffold list names
one, but `src/bounty/sched.rs` contains only `Scheduler`; drivers own
the poll loop). Key design decision: `poller_tick` reschedules the
next poll from `now`, not from the old due time. The naive
`next_due += interval` formulation would fire 60 catch-up polls after
a +3600s jump (a poll storm); rescheduling from `now` fires exactly
one catch-up and skips the missed ones deliberately. That is the
catch-up bound: fail-open timer (a failing poll can't wedge the
schedule), fail-closed data (a failing poll never files anything, so
the old scope is retained).

`ScopeStore::file` enforces the versioning contract structurally:
same version → `Ok(false)` (idempotent, no new entry); lower
version → `Err(ScopeStoreError::StaleVersion)` (replay protection,
exercised by task 150's feed). Malformed feed data never reaches
`file` — `feed.poll()` returns `Err(FeedError::Malformed)` first,
and the tick logs it and moves on.

## Sources

- Primary: sw33tlie/bbscope repo docs — `website/README.md` and
  `docs/src/web/self-hosting.md`: `--poll-interval` default **6
  hours** between background poll cycles (0 disables); the site
  exposes a scope-changes feed and per-program change timelines, i.e.
  changes are tracked across polls. Our driver's 60s cadence is a
  test acceleration, not a claim about bbscope's config; no wire
  compatibility is claimed.
- Scaffold: `src/bounty/feed.rs` (`ScriptedFeed`, `FeedError`),
  `src/bounty/store.rs` (`ScopeStore::file`), `src/bounty/clock.rs`
  (`ManualClock`).
