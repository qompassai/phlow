# task-178: ephemeral bridge launch

**Kind:** rust · **Status:** pass · **Wave:** 29 · **Commits:** pending (wave 29)

## ELI5

Imagine hiring a temp worker for exactly one job: they show up, do the one thing you asked, and leave — no lingering in the hallway, no forgotten badge that still opens doors. The "bridge" is that temp worker: a browser process the agent spawns for one task. This task checks the whole lifecycle: the bridge launches exactly one browser tab, shakes hands with the agent (the MCP `initialize` handshake), and when the job is done, the tab closes and the browser process is fully gone — verified by counting processes on the machine, not by trusting a "we cleaned up" message.

## What this task attempts

- **Goal:** prove a bridge launches one target, completes MCP initialize, and leaves zero lingering targets or processes at task end.
- **Mechanism:** `crates/phlow-gauntlet/src/bridge.rs` (`Bridge::launch`, `Bridge::shutdown`, `mcp_initialize`, `terminate_child`) driven by `crates/phlow-gauntlet/src/tasks/task_178.rs`; assertions in `crates/phlow-gauntlet/tests/task_178.rs`.
- **Success criterion:** scripted cases show 1 `Target.created`, 1 live target, a successful initialize handshake, then `targets_closed == 1`, `reaped == true`, and an empty process census; the integration case shows the same 0 → 1 → 0 census against real Chromium.
- **Non-goals:** real `Runtime.evaluate` over a websocket (scripted + documented only); multi-target orchestration (task 182/184).

## What happened

Pass on the third iteration. The two scripted cases passed from the start (3/3 once the driver bug below was fixed). The real-Chromium integration case failed twice for two distinct, verified reasons before passing. Final evidence: `test result: ok. 3 passed; 0 failed; finished in 1.23s` against Chromium 153.0.8010.52 on primo, with the bridge-owned target observed live on the debugging port and then gone after shutdown (`targets_closed=1`, `reaped=true`, `census_clean=true`).

Wave-wide honesty note: the wave's first `cargo build -p phlow-gauntlet` failed with **23 errors and 1 warning** before any test ran; all were fixed before the first green gate. Nothing in this doc's pass claims predates that repair.

## Where it went wrong

Two failures, both in the real-Chromium integration case:

**Failure 1 — readiness probe.** *Stage:* bridge launch against real Chromium. *Symptom:* `bridge process spawn failed: debugging port <port> never answered`. *Evidence:* the exact Chromium command and flags worked when run by hand; a standalone Rust test using the same spawn arguments connected successfully; but the bridge's readiness probe — a raw TCP write of `GET /json/version HTTP/1.0\r\n\r\n` — got an immediate EOF (`read()` returning 0 bytes) on every attempt, while `curl http://127.0.0.1:<port>/json/version` succeeded. *Root cause:* Chromium 153's DevTools HTTP server drops bare `HTTP/1.0` requests without response. A request-shape probe confirmed: `HTTP/1.0` without Host → empty; `HTTP/1.0` with Host → empty; `HTTP/1.1` with `Host` + `Connection: close` → `HTTP/1.1 200 OK`. The probe was malformed, not the browser.

**Failure 2 — target census.** *Stage:* port-level 0 → 1 → 0 assertion. *Symptom:* `debugging port shows 3 targets, want 1`. *Evidence:* a debug run listed the port before/after the driver's "drain foreign targets" step: primo's Chromium spawns its own extension/service-worker/omnibox targets (Google Hangouts background page, omnibox popups, extension service workers); `/json/close` returned `Ok` for them but they persisted or respawned — "Target is closing" is asynchronous and extension targets are not ours to close. *Root cause:* the test tried to drain targets the bridge never owned. The environment's extension noise is not bridge state.

## The fix — what changed and why

**Fix 1 — valid HTTP/1.1 readiness probe** (`src/bridge.rs`, `chromium_debugging_ready`).
- **Changed:** the probe now sends `GET /json/version HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n` with a comment recording the verified requirement.
- **Why:** the DevTools server requires HTTP/1.1 with a Host header; anything less is silently dropped. The alternative — using the reqwest client for the probe — was rejected to keep the hot launch path dependency-free and bounded (raw TCP, 2s read timeout).
- **Source:** behavior verified empirically against Chromium 153.0.8010.52 on 2026-09-28 (request-shape matrix above); the CDP HTTP endpoints are documented at the official protocol page.
- **Citations:** https://chromedevtools.github.io/devtools-protocol/ (`/json/version`, `/json/list`, `/json/new`, `/json/close`).

**Fix 2 — scope the census to the bridge-owned target** (`src/tasks/task_178.rs`).
- **Changed:** deleted `drain_foreign_targets`; the case now asserts the bridge's own raw target id is present in `/json/list` after launch (the "1"), then after `shutdown` asserts `targets_closed == 1`, `reaped == true`, the port is unreachable, and no chromium PIDs remain in the census (the "0"). The `verify_target_live` helper was extracted to keep the case under 70 lines.
- **Why:** the bridge owns exactly one target; extension/service-worker targets belong to the user's browser profile, not the task. Asserting on the owned target is the honest 0 → 1 → 0; draining the world fights the environment and proves nothing about the bridge.
- **Source:** `/json/list` semantics from the official protocol page (lists page, worker, background-page, and other inspectable targets — i.e., more than the bridge's page is normal).

## Full technical depth

`Bridge::launch` spawns the child (`StubKind::Chromium` → real `/usr/bin/chromium --headless=new --no-sandbox --disable-gpu --remote-debugging-port={port} --user-data-dir={profile} about:blank`), polls the debugging port with the fixed HTTP/1.1 probe until `CHROMIUM_READY_TIMEOUT`, then creates one target via the port. `mcp_initialize` performs the MCP handshake over the newline-delimited stdio framing contract (request and response both cross the framing layer; the scripted responder answers with `protocolVersion 2024-11-05`). `shutdown` closes owned targets via `/json/close/{id}` (documented `GET`), then `terminate_child` sends SIGTERM, escalates to SIGKILL after `SIGTERM_GRACE` (500ms), and reaps within `BRIDGE_KILL_TIMEOUT` (5s). The census check reads the process table for command lines containing `remote-debugging-port`.

Separation of coverage, stated plainly: the scripted half proves protocol semantics (launch → 1 target → initialize → close → reap) deterministically; the real-Chromium half proves the HTTP discovery/lifecycle path (port readiness, `/json/list` presence of the owned target, close, process reaping) against Chromium 153. Typed `Runtime.evaluate` over a real websocket is **not** covered — the CDP docs describe it, the scripted port simulates its result shapes, but no test in this wave speaks the websocket transport.

Wave contracts, declared: all bridge↔agent traffic uses newline-delimited JSON framing (task 179); cyclic values are rejected with `BridgeError::CyclicValue`, never serialized (task 181).

Ghostex lineage: the ephemeral-launch concept is adapted from maddada/Ghostex @ `c91146607205ac49303d1bcfe2fd6f9a86741500`. Ghostex's CEF/GPUI implementation was not ported; this is a Tiger Style Rust re-implementation of the concept against CDP-over-HTTP.

## Sources

- https://chromedevtools.github.io/devtools-protocol/ — official CDP docs: `/json/version`, `/json/list`, `/json/new?{url}` (PUT, raw query decoded as the initial URL), `/json/close/{targetId}` (GET), `Runtime.evaluate` domain
- `crates/phlow-gauntlet/src/bridge.rs` — `Bridge::launch`, `Bridge::shutdown`, `shutdown_inner`, `terminate_child`, `chromium_debugging_ready`, `ChromiumPort`
- `crates/phlow-gauntlet/src/tasks/task_178.rs` — the three driver cases
- `crates/phlow-gauntlet/tests/task_178.rs` — the integration assertions
- maddada/Ghostex @ `c91146607205ac49303d1bcfe2fd6f9a86741500` — adapted concept (CEF/GPUI implementation not ported)
