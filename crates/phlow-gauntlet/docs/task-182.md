# task-182: bridge navigation allowlist

**Kind:** rust · **Status:** pass · **Wave:** 29 · **Commits:** pending (wave 29)

## ELI5

If the temp worker is only allowed to visit one office, you don't let them wander the building — and you definitely don't let them open a new office for themselves. The bridge gets an allowlist: a short list of web addresses it's permitted to visit. This task attacks that allowlist four ways: asking for a website that's not on the list, asking for a `file://` address (which could read the agent's own files), trying to pop open a second window (which would be a second tab nobody approved), and asking for ten tabs at once (which must be refused with a declared bound, not silently honored). Every attack must be refused with a specific typed error — not a crash, not a silent yes.

## What this task attempts

- **Goal:** prove navigation is confined to the allowlist: off-list hosts denied, `file://` denied, `window.open` cannot create a second target, target counts stay within a declared bound.
- **Mechanism:** `crates/phlow-gauntlet/src/bridge.rs` (`NavigateGuard` / allowlist check in target creation and evaluate paths) driven by `crates/phlow-gauntlet/src/tasks/task_182.rs`; assertions in `crates/phlow-gauntlet/tests/task_182.rs`.
- **Success criterion:** off-allowlist host → typed `NavigationDenied`; `file://` URL → typed `NavigationDenied`; `window.open` attempt → bridge still holds exactly one target; ten-tab request → refused at the declared `MAX_TARGETS` bound with a typed error.
- **Non-goals:** content filtering inside an allowed page (allowlist is about *where*, not *what*); real-browser enforcement of the same policy (scripted fixtures per the CDP target-lifecycle semantics).

## What happened

Pass on the second iteration: 4/4, `test result: ok. 4 passed; 0 failed; finished in 0.02s`. The first iteration failed all four cases from the shared driver-reporting bug (task 179's fix — verdict computed, evidence channel empty). No allowlist defect was ever found; the guard was correct from the start.

Wave-wide honesty note: the wave's first `cargo build -p phlow-gauntlet` failed with **23 errors and 1 warning** before any test ran; all were fixed before the first green gate.

## Where it went wrong

The only failure was the cross-task reporting bug documented in task-179: every `finish_case` computed `report.passed` but never assigned `report.failures`, so all four cases reported failure with empty evidence. The allowlist logic itself passed on its first real run. This section is kept because the template requires it and because the failure is honest: a test harness that cannot show its evidence is a defect, even when the product is fine.

## The fix — what changed and why

- **Changed:** `src/tasks/task_178.rs`–`task_185.rs` — `finish_case` now assigns `report.failures = failures` alongside the verdict (shared with task 179's fix).
- **Why:** see task-179. The fix is identical; it is recorded here because this task's first run was its victim.
- **Source:** internal `CaseReport` contract.
- **Validation agents:** 4/4 green after the reporting fix; no allowlist code changed.
- **Adversarial agents:** the four attacks *are* the adversarial coverage — off-list host, `file://`, `window.open`, ten-tab burst — each asserting a typed refusal and, for `window.open`, that the bridge still owns exactly one target afterward.

## Full technical depth

The allowlist is enforced at two choke points, both in `bridge.rs`: target creation (the `file://` denial and off-host denial fire before any navigation is attempted — the URL's host/scheme is validated against the allowlist with always-on checks, never string-prefix matching) and the evaluate path (a `window.open` expression is refused with `NavigationDenied` rather than evaluated, because evaluation would hand the page a primitive the bridge cannot take back). The multi-target bound is declared as a named constant (`MAX_TARGETS`); the ten-tab request is refused with the typed error naming the bound, so the limit is observable, not implicit.

The `file://` denial deserves emphasis: allowing local-file navigation would let a compromised page read agent-side files through the browser's file access. The guard denies by scheme before the host check, so `file://` is refused even if an allowlist entry somehow matched.

All fixtures are scripted CDP target-lifecycle shapes per https://chromedevtools.github.io/devtools-protocol/ (`Target.created`/`Target.destroyed`, `/json/new`, `/json/close`). The real-browser half of the wave (tasks 178/180) proves the HTTP transport works; the allowlist logic is transport-independent, so scripted fixtures are the honest coverage here.

Wave contracts, declared: all bridge↔agent traffic uses newline-delimited JSON framing (task 179); cyclic values are rejected with `BridgeError::CyclicValue`, never serialized (task 181).

Ghostex lineage: the navigation-confinement concept is adapted from maddada/Ghostex @ `c91146607205ac49303d1bcfe2fd6f9a86741500`. Ghostex's CEF/GPUI implementation was not ported; this is a Tiger Style Rust re-implementation of the concept.

## Sources

- https://chromedevtools.github.io/devtools-protocol/ — official CDP docs: Target domain lifecycle events, `/json/new`, `/json/close`
- `crates/phlow-gauntlet/src/bridge.rs` — allowlist enforcement, `MAX_TARGETS`, `BridgeError::NavigationDenied`
- `crates/phlow-gauntlet/src/tasks/task_182.rs` — the four adversarial driver cases
- `crates/phlow-gauntlet/tests/task_182.rs` — the integration assertions
- maddada/Ghostex @ `c91146607205ac49303d1bcfe2fd6f9a86741500` — adapted concept (CEF/GPUI implementation not ported)
