# task-183: cancelled bridge reaping

**Kind:** rust · **Status:** pass · **Wave:** 29 · **Commits:** pending (wave 29)

## ELI5

Two nightmares for any program that spawns helper processes. First: you tell the helper "stop, I changed my mind" while it's in the middle of a slow job — the cancellation must actually reach it and the whole bridge must still shut down cleanly, no stuck thread, no orphaned browser. Second: you tell the browser to quit politely (SIGTERM) and it ignores you — the bridge must then escalate to the un-ignorable kill (SIGKILL) and still reap the process, so no zombie is left behind. This task proves both: mid-evaluation cancellation returns a typed `Cancelled` error and reaps within the 5-second budget, and a SIGTERM-ignoring browser still ends up dead and reaped.

## What this task attempts

- **Goal:** prove cancellation during a slow evaluate returns a typed error with full cleanup, and SIGTERM defiance escalates to SIGKILL with reaping.
- **Mechanism:** `crates/phlow-gauntlet/src/bridge.rs` (`SharedScriptedPort::evaluate`, cancel `Arc<AtomicBool>`, `terminate_child` with `SIGTERM_GRACE` 500ms → SIGKILL, reap within `BRIDGE_KILL_TIMEOUT` 5s) driven by `crates/phlow-gauntlet/src/tasks/task_183.rs`; assertions in `crates/phlow-gauntlet/tests/task_183.rs`.
- **Success criterion:** cancel set mid-30s-slow evaluate → `BridgeError::Cancelled` (not a hang, not `Ok`); browser process reaped well under 5s. A child that installs `SIG_IGN` for SIGTERM → SIGKILL escalation, `reaped=true`, `zombie=false`, zero remaining targets.
- **Non-goals:** cancelling a *real* in-flight websocket `Runtime.evaluate` (no websocket transport this wave); sub-500ms escalation (the grace period is deliberate).

## What happened

Pass after three fix iterations: 2/2, `test result: ok. 2 passed; 0 failed; finished in 0.60s`, stable across five consecutive runs (0.60–0.61s each). Two real concurrency defects were found and fixed — this task's adversarial cases earned their keep.

Wave-wide honesty note: the wave's first `cargo build -p phlow-gauntlet` failed with **23 errors and 1 warning** before any test ran; all were fixed before the first green gate.

## Where it went wrong

**Failure 1 — the cancel that couldn't fire.** *Stage:* `cancel_mid_evaluate_reaps`. *Symptom:* the test set the cancel flag while `evaluate` was spinning, but evaluation returned `Ok(Null)` after the full 30-second slow spin — the cancellation was never observed. *Root cause:* `SharedScriptedPort::evaluate` held the port's mutex for the entire slow spin; the cancel setter needed the same mutex to flip the flag. Classic lock-granularity deadlock-by-design: the flag could only be set after the thing it was supposed to interrupt had already finished.

**Failure 2 — the flaky SIGTERM readiness race.** *Stage:* `sigterm_ignored_escalates`. *Symptom:* intermittent failure — the Python child (which installs `SIG_IGN`) was killed before it installed the handler, so SIGTERM killed it "cleanly" and the escalation path was never exercised. *Root cause:* the parent slept a fixed 200ms after spawning, assuming the interpreter would be ready. Under load it wasn't. A fixed sleep is not synchronization.

## The fix — what changed and why

**Fix 1 — snapshot under a short lock, spin outside it** (`src/bridge.rs`, `SharedScriptedPort::evaluate`).
- **Changed:** the method now acquires the mutex only long enough to snapshot the `slow` flag and clone the cancel `Arc<AtomicBool>`, then releases the lock and runs `slow_spin` outside it, polling the atomic each iteration and returning the typed `BridgeError::Cancelled` on sight.
- **Why:** a cancellation flag behind a mutex that the cancellable work holds is a flag that can never be set. The atomic is lock-free by construction; snapshotting the slow flag keeps the mutex hold bounded to microseconds. The alternative — making the whole evaluate async — was rejected as out of scope for the wave's sync bridge design.
- **Source:** standard lock-granularity practice; the Tiger Style rule "retain cancellation" (`~/workspace/skills/tiger-style-rust/SKILL.md`).

**Fix 2 — bounded readiness handshake** (`src/tasks/task_183.rs`).
- **Changed:** the Python child now installs `SIG_IGN`, writes `ready\n`, flushes, then sleeps; the parent waits for the `ready` line up to `IGNORE_SIGTERM_READY_TIMEOUT`, and kills/reaps the child if readiness never arrives.
- **Why:** readiness is an event, not a duration. The handshake makes "the handler is installed" observable; the timeout keeps it bounded so a wedged child can't hang the test.
- **Source:** process-synchronization via pipe handshake (the same pattern the wave uses for the Chromium readiness probe, task 178).

- **Validation agents:** 2/2 green; five consecutive runs at 0.60–0.61s with zero flakes; `--lib` suite green.
- **Adversarial agents:** attempted to defeat the escalation by having the child ignore SIGTERM (escalated correctly), attempted to observe the cancel flag (now lock-free), and attempted to hang readiness (bounded by the timeout).

## Full technical depth

`terminate_child` implements the exact once-only ownership release: on `shutdown` or cancel it takes the child handle exactly once (a `taken` flag under the bridge mutex — double-take is impossible), sends SIGTERM, waits `SIGTERM_GRACE` (500ms) polling for exit, escalates to SIGKILL on survival, then reaps within `BRIDGE_KILL_TIMEOUT` (5s total). `Drop` on the bridge is a bounded best-effort `kill` + 200ms reap poll — best-effort because `Drop` cannot return errors or block indefinitely; normal shutdown and cancel remain exact once-only cleanup, and the doc comment on `Drop` states the best-effort semantics and the ignored-error rationale honestly.

The cyclic-value policy (rejection, task 181) and the newline-delimited framing (task 179) are unchanged here; cancellation composes with both because the cancel check sits in the traversal loop and the framing drain.

Ghostex lineage: the supervised-process-lifecycle concept is adapted from maddada/Ghostex @ `c91146607205ac49303d1bcfe2fd6f9a86741500`. Ghostex's CEF/GPUI implementation was not ported; this is a Tiger Style Rust re-implementation of the concept.

## Sources

- https://chromedevtools.github.io/devtools-protocol/ — official CDP docs (target lifecycle; the real-browser half of the wave's coverage)
- `crates/phlow-gauntlet/src/bridge.rs` — `SharedScriptedPort::evaluate`, `slow_spin`, `terminate_child`, `Bridge::shutdown`, `Drop`, `BridgeError::Cancelled`
- `crates/phlow-gauntlet/src/tasks/task_183.rs` — the two adversarial driver cases, readiness handshake
- `crates/phlow-gauntlet/tests/task_183.rs` — the integration assertions
- maddada/Ghostex @ `c91146607205ac49303d1bcfe2fd6f9a86741500` — adapted concept (CEF/GPUI implementation not ported)
