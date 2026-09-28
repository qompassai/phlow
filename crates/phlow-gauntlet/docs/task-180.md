# task-180: DevTools HTTP round trip

**Kind:** rust · **Status:** pass · **Wave:** 29 · **Commits:** pending (wave 29)

## ELI5

Besides the note-passing pipe, the browser also runs a tiny web server (the "debugging port") that answers questions like "what tabs are open?" and "open this page." This task checks both halves of talking to a page: the scripted half asks a fake browser to evaluate JavaScript and checks the answer comes back correctly typed (a page title becomes text, a nested object becomes text you can parse back byte-identically, `undefined` becomes a proper Null — never a crash). The real half opens a real page in real Chromium and watches the exact title appear on the debugging port. What it does *not* do is the live websocket conversation for `Runtime.evaluate` — that needs a websocket client this wave doesn't build, so the real-browser half proves HTTP discovery, navigation, and title observation only.

## What this task attempts

- **Goal:** prove typed `Runtime.evaluate` result mapping on the scripted port and real-browser HTTP lifecycle (open → navigate → observe title → close) on primo's Chromium.
- **Mechanism:** `crates/phlow-gauntlet/src/bridge.rs` (`ChromiumPort`, `ScriptedPort::evaluate`, `ingest_remote`, `new_target_url`) driven by `crates/phlow-gauntlet/src/tasks/task_180.rs`; assertions in `crates/phlow-gauntlet/tests/task_180.rs`.
- **Success criterion:** scripted `evaluate("document.title")` → typed title text; nested object → MCP text that parses back to the source value with the bigint `9007199254740993` intact via `unserializableValue`; unknown expression → typed Null; real Chromium opens a `file://` page and the port reports the exact title `Wave 29 Test Page`, then the browser is reaped with a clean census.
- **Non-goals:** real websocket `Runtime.evaluate` (no websocket transport in this wave — stated, not snuck); internet-dependent navigation (the fixture is a local file).

## What happened

Pass on the third iteration: 4/4, `test result: ok. 4 passed; 0 failed; finished in 1.43s`. Two failures preceded it: the shared driver-reporting bug (task 179's fix) and a CDP HTTP contract bug (below) that silently navigated to the wrong page.

Wave-wide honesty note: the wave's first `cargo build -p phlow-gauntlet` failed with **23 errors and 1 warning** before any test ran; all were fixed before the first green gate.

## Where it went wrong

- **Stage:** real-Chromium title round trip.
- **Symptom:** the title assertion failed — the target stayed on `about:blank` instead of navigating to the fixture page.
- **Evidence:** a probe against real Chromium 153 showed the driver requested `/json/new?url=<encoded>` (via reqwest `.query(&[("url", url)])`) and the browser opened `about:blank`. The official protocol page states `/json/new?{url}` takes the **raw query component**, URL-decoded, as the initial URL — `?url=<value>` is not the contract, and invalid input silently falls back to `about:blank`.
- **Root cause:** invented URL shape. The driver guessed `?url=`; the documented contract is the raw encoded query. A second, smaller defect rode along: the driver used `PUT /json/close/{id}` while the docs specify `GET`.

## The fix — what changed and why

- **Changed:** `src/bridge.rs` — added `new_target_url(base, url)` which builds the documented raw encoded query using `percent-encoding` (added as a direct locked dependency, `=2.3.2`, already in `Cargo.lock` transitively); both real target-creation paths use it. Changed `/json/close` from `PUT` to the documented `GET`. The real integration now writes a local HTML fixture and navigates to `file://…` (no network), after discovering that `data:` URLs are rejected for top-frame navigation.
- **Why:** the raw-query form is the documented contract; `?url=` silently degrades to `about:blank`, which is the worst kind of wrong (no error, wrong page). `file://` keeps the real-browser half hermetic.
- **Source:** https://chromedevtools.github.io/devtools-protocol/ — "`/json/new?{url}` … puts the given url … Note that the URL must be URL-encoded… uses PUT"; "`/json/close/{targetId}` … GET".
- **Citations:** the official protocol page above; behavior re-verified live against Chromium 153.0.8010.52 on 2026-09-28.

## Full technical depth

The scripted half drives `ScriptedPort::evaluate` with canned CDP `Runtime.evaluate` result objects (`cdp_string`, `cdp_object`): `{"type": "string", "value": …}` → `McpContent::Text`; `{"type": "object", "value": …}` → text via the iterative `bound_value` walk (depth cap 32, string prefix-truncation at 65,536 bytes, cyclic rejection); bigints arrive as `{"unserializableValue": "9007199254740993"}` and survive the round trip exactly (the test asserts the digit string in the evidence); `{"type": "undefined"}` → `McpContent::Null`.

The real half spawns headless Chromium (extracted helper `spawn_headless_chromium`, keeping the case under 70 lines), waits for a target on `/json/list`, writes `title.html` into the temp profile dir, opens it via the corrected `/json/new?<raw-encoded-file-url>` (PUT), polls until the port reports the exact title, closes the target (`GET /json/close/{id}`), kills and reaps the browser, and asserts no PID lingers.

Coverage separation, stated plainly: scripted = typed `Runtime.evaluate` semantics per the CDP docs; real = HTTP discovery, target lifecycle, navigation, and title observation. No test in this wave opens a websocket to the browser; real `Runtime.evaluate` over the wire is **not** claimed.

Wave contracts, declared: evaluation results cross the stdio pipe as newline-delimited JSON (task 179); cyclic values are rejected with `BridgeError::CyclicValue`, never serialized (task 181).

Ghostex lineage: the DevTools-driven page concept is adapted from maddada/Ghostex @ `c91146607205ac49303d1bcfe2fd6f9a86741500`. Ghostex's CEF/GPUI implementation was not ported; this is a Tiger Style Rust re-implementation of the concept against CDP-over-HTTP.

## Sources

- https://chromedevtools.github.io/devtools-protocol/ — official CDP docs: `/json/new` (PUT, raw query), `/json/close` (GET), `/json/list`, `Runtime.evaluate` result shapes (`type`, `value`, `unserializableValue`)
- `crates/phlow-gauntlet/src/bridge.rs` — `ChromiumPort`, `new_target_url`, `ScriptedPort::evaluate`, `ingest_remote`, `bound_value`
- `crates/phlow-gauntlet/src/tasks/task_180.rs` — the four driver cases
- `crates/phlow-gauntlet/tests/task_180.rs` — the integration assertions
- maddada/Ghostex @ `c91146607205ac49303d1bcfe2fd6f9a86741500` — adapted concept (CEF/GPUI implementation not ported)
