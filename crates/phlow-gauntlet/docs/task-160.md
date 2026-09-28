# task-160: subscribe-on-reconnect

**Kind:** rust · **Status:** pass · **Wave:** 26 ·
**Commits:** single wave commit `gauntlet: wave 26 drivers, tests, docs`

## ELI5

When your call drops and you redial, you shouldn't have to re-tell the
other person everything you were listening to — the phone should
remember your subscriptions. This task proves the client treats
subscriptions as *its own* memory, not the connection's: after a drop
and reconnect it re-sends `subscribe(a)` and `subscribe(b)` exactly
once each — never zero times (lost interest), never twice (duplicate
noise). It also proves you can add a new subscription *while* the line
is down, and it lands correctly once the connection is back.

## What this task attempts

- **Goal:** after any reconnect the client's subscription set is
  re-established exactly once per topic — no drops, no duplicates —
  including topics added during the outage.
- **Mechanism:** `SubscriptionSet` in
  `crates/phlow-gauntlet/src/daemon_client.rs`, driven by
  `crates/phlow-gauntlet/src/tasks/task_160.rs` against a
  `DaemonFixture` (real loopback TCP) that records every subscribe
  frame; `ManualClock` for reconnect timing.
- **Success criterion:** post-reconnect subscription set equals the
  pre-drop set exactly; the daemon-side subscribe count is 1 per
  topic; events flow on all topics after resubscribe.
- **Non-goals:** the reconnect ladder itself (task 159), event/state
  convergence (task 161).

## What happened

Both driver cases pass on the final gates: `cargo test -p
phlow-gauntlet --test task_160` → 2 passed, 0 failed; lib tests 64/64;
fmt and clippy clean. V1: subscribed to `{a, b}`, dropped the link,
reconnected — the fixture's subscribe log shows exactly
`["a", "b"]` (one frame each, order-stable), and a post-reconnect
event on each topic is observed by the client. V2: `subscribe(c)`
issued while the engine is parked mid-outage → after reconnect the
established set is exactly `{a, b, c}` with daemon-side counts of 1
per topic. No duplicate frames, no missing topics.

## Where it went wrong

- **Stage:** verdict plumbing, shared with task 159's second bug.
- **Symptom:** `report.passed == false` with an empty `failures` vec.
- **Evidence:** same as task 159 — `CaseReport::pass` built the report
  but the local `failures` were never assigned into it.
- **Root cause:** missing `report.failures = failures` before
  recomputing `passed`. Fixed once, in every driver, via the same
  pattern (task 160 keeps the inline tail since it was already ≤70
  lines).

No behavioral bug was found in `SubscriptionSet` itself: the
first green run of the resubscribe assertions held.

## The fix — what changed and why

- **Changed:** `crates/phlow-gauntlet/src/tasks/task_160.rs` —
  assigns `report.failures = failures; report.passed =
  report.failures.is_empty();` before returning.
- **Commit:** single wave commit `gauntlet: wave 26 drivers, tests,
  docs`.
- **Why:** a verdict that can never be honest is worse than a failing
  test — it teaches the wrong confidence. The fix makes `passed` a
  pure function of the collected failures.
- **Source:** the `CaseReport` contract in
  `crates/phlow-gauntlet/src/skillopt/driver.rs` (a report's `passed`
  flag must agree with its `failures` list).
- **Validation agents:** the subagent author ran the full gate set on
  primo after the fix: targeted 2/2, lib 64/64, fmt clean, clippy 0
  warnings.
- **Adversarial agents:** same author — attempted to force a duplicate
  subscribe by reconnecting twice in a row without a drop, and a lost
  subscribe by subscribing during the outage then dropping *again*
  before reconnect; the set semantics (idempotent add, snapshot on
  reconnect) held in both probes.
- **Citations:** Ghostex subscribe-on-reconnect behavior
  (`packages/gx-client/src/worker.rs` @ c911466).

## Full technical depth

`SubscriptionSet` is client-held state: `insert`/`remove` mutate a
local set (`MAX_SUBSCRIPTIONS = 256` bounds it), and the reconnect
path replays the *entire set* as fresh subscribe frames on the new
connection — the daemon never has to remember. The fixture
(`DaemonFixture::start`) binds loopback TCP, speaks the 4-byte
big-endian length + JSON frame protocol, checks the credential *before*
parsing the op, and appends every `subscribe` topic to a shared audit
log the driver reads after the run.

V1's flow: spawn fixture → client subscribes a, b (fixture log:
`[a, b]`) → scripted drop → engine reconnects on the ladder →
reconnect hook replays the set → assert fixture log is exactly
`[a, b, a, b]`-as-multiset with per-topic count 1 post-reconnect →
publish one event per topic → client observes both. V2's flow: same,
but `subscribe(c)` lands while `engine.parked()` is false and the link
is down; the set becomes `{a, b, c}`; after reconnect the fixture log
shows exactly one post-reconnect frame per topic. Deterministic time
comes from `ManualClock`; the fixture's TCP half is real sockets on
loopback (MOCK = scripted daemon behavior, not a fake socket).

## Sources

- Primary: Ghostex `packages/gx-client/src/worker.rs`,
  `packages/gx-client/src/socket.rs` @ c91146607205ac49303d1bcfe2fd6f9a86741500
  (subscribe-on-reconnect doctrine).
- Primary: `crates/phlow-gauntlet/src/daemon_client.rs`
  (`SubscriptionSet`, `DaemonFixture`, `MAX_SUBSCRIPTIONS`).
- Secondary: wave 26 design notes
  (`~/workspace/gauntlet-design-tasks-151-200.md`, task-160 section).
