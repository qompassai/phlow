# task-140: rate limits and testing windows

**Kind:** rust · **Status:** pass · **Wave:** 24

## ELI5

A bug-bounty program says "only test us between 2am and 4am, and don't
hammer our servers — max 60 requests a minute." Those rules must be
built into the machine, not written in a doc someone might forget.
This task checks the scheduler obeys them structurally: at 3am it
launches tests; at 5am it holds the queue and launches nothing; if 4am
arrives while two tests are running, those two finish but nothing new
starts; and when the platform says "slow down" (HTTP 429), the machine
backs off — waiting 1 second, then 2, then 4, up to a 5-minute cap —
instead of retrying in a storm.

## What this task attempts

- **Goal:** no launches outside the testing window; platform 429s drive capped exponential backoff with no retry storm.
- **Mechanism:** `Scheduler::tick` consulting `TestingWindow` and the token-bucket `RateLimit` in `crates/phlow-gauntlet/src/bounty/sched.rs`; scripted 429s from `FakePlatform::fail_next_submit`; time via a shared cell clock (MOCK) the driver advances.
- **Success criterion:** V1 tick at 03:00 → all 5 targets launched; V2 tick at 05:00 → single `Hold { reason: "testing window closed" }`, queue intact; A1 window closes with 2 in flight → 0 launches after 04:00, queue intact; A2 attempt gaps exactly [1,2,4]s, attempts ≤ `limit × elapsed + burst`, backoff schedule caps at 300s.
- **Non-goals:** operator approval gating (tasks 134/135) — this is the program's time/rate envelope, independent of approval.

## What happened

Gating caught one driver bug before the final green run:
`FakePlatform::fail_next_submit` is a one-shot boolean, so calling it
three times up front armed only a single 429 — the arm now re-arms
immediately before each of the first three submissions (explicit
bounded refusal counter). Fixed; all four cases pass. V1: 03:00 tick launches 3 (K=3),
next tick launches 2 — 5/5 launched, zero holds. V2: 05:00 tick →
`[Hold { reason: "testing window closed" }]`, 0 launches, queue 5/5,
`in_flight` 0. A1: 2 launched at 03:59; ticks at 04:00 and 04:01 both
`Hold` with the window reason; 0 launches at/after 04:00; queue keeps
the 2 unlaunched targets; in-flight runs finished driver-side. A2:
attempts at t0, t0+1, t0+3, t0+7 — gaps exactly [1, 2, 4] seconds
(exponential); 4 attempts in 7s ≤ 60×7/60 + 5 = 12 (no storm);
`Backoff` actions observed between attempts; the cap unit check on a
fixed clock reads [1, 2, 4, 8, 16, 32, 64, 128, 256, 300, 300, 300] —
capped at 300s as documented.

## Full technical depth

Three structural gates run in `tick` order: (1) backoff — while
`now < backoff_until` the tick answers `Backoff { secs }` and touches
neither window nor queue; (2) window — `TestingWindow::allows` on
time-of-day (half-open `[start_min, end_min)`, midnight wrap
supported); (3) the token bucket — `Bucket::take` refills at
`limit/60` tokens/sec capped at `burst`. The 429 arm's driver protocol:
each `Launch` performs one scripted platform call; on 429 the target
is requeued and the phantom slot released via `note_run_finished`
(driver-level, documented in the case), then `note_platform_429`
doubles the backoff (1→2→4…, `min(×2, 300)`); on success
`note_platform_success` resets to 1s. The window is time-of-day in
UTC, matching how HackerOne/Bugcrowd program pages state testing
windows; 429 handling follows RFC 6585 §4 (the status code) with the
Retry-After concept from RFC 9110 §10.2.3 informing the backoff shape.

## Sources

- Primary: `crates/phlow-gauntlet/src/bounty/sched.rs` (`tick`, `Bucket`, `note_platform_429`); `crates/phlow-gauntlet/src/bounty/platform.rs` (`FakePlatform::submit`, `fail_next_submit`); RFC 6585 §4 (429 Too Many Requests); RFC 9110 §10.2.3 (Retry-After).
