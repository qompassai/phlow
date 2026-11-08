# task-88: A2A presence audit

**Kind:** rust · **Status:** pass (D-1059 redesign, 2026-11-07) · **Wave:** 86–90 · **Commits:** pending (phlow programme rung 4)

## ELI5

The original task-88 asked whether phlow's rust code could carry another agent's errors across the A2A boundary *typed* — code, message, and extra data preserved — while treating that extra data as untrusted, rejecting unknown task states instead of ignoring them, and never letting a remote agent's claims overwrite your own run's state. Back then the honest answer was: there is no rust A2A boundary at all, so the task reported the seam's absence and banked the product decision.

That decision has since been taken: `crates/phlow-a2a` exists — a registry of external agents, a bounded task store with a legal-transition state machine, a loopback-first JSON-RPC server, and a synchronous client. So task-88 is redesigned (D-1059) from an absence probe into a **presence audit**: instead of scanning source for words that must *not* be there, it drives the real crate and asserts the boundary *exists and behaves*.

## What this task attempts

- **Goal:** verify against the real `phlow-a2a` crate that (a) registration and the client/server handshake work end to end, (b) the task store round-trips state, history, and artifacts under its transition table, (c) unknown task states are rejected as protocol violations at the name layer, the decode path, and the wire, and (d) remote errors arrive typed (code/message/data preserved), attacker-controlled error data stays bounded and is treated as data only, and a fetched remote task never enters or mutates a local store.
- **Mechanism:** `src/tasks/task_88.rs` runs four cases against `phlow-a2a` (now a gauntlet dependency). Validation cases use the registry, a real loopback `A2aServer` with a trivial echo executor, the real client, and a real `TaskStore`. Adversarial cases use the crate's actual serde decode path, raw JSON-RPC over a socket (a `ListTasks` filter carrying an unknown state — a payload the typed client correctly refuses to construct), and a raw fake peer serving attacker-shaped and oversized error responses to the real client.
- **Success criterion:** all four cases pass against the crate as it stands. If the crate behaved otherwise, the case would fail and the failure would be the audit's finding — the audit is never weakened to force green, and this task does not change `phlow-a2a` behaviour.
- **Non-goals:** changing any `phlow-a2a` behaviour; the flow-gated `ExternalRegistry::send_task` path (the audit uses registration + `client_for`; send-gating is `phlow-a2a`'s own wire-test territory); diver's Lua A2A boundary (task-07's territory, unchanged).

## Cases

Two validation, two adversarial:

- **V1 `registry_and_handshake`:** register an `ExternalAgent` in an `ExternalRegistry` (unknown names fail with `RegistryError::UnknownAgent`); `client_for` builds a client; `fetch_card` returns the served card — name matches, primary URL is the server's bound URL (the server rewrites the ephemeral `:0` authority when serving), and the card does not claim push notifications; `send_message` completes a task end-to-end with the executor's echo artifact; `get_task` shows the terminal `TASK_STATE_COMPLETED`.
- **V2 `store_roundtrip`:** `TaskStore` create (starts `Submitted`, first message in history) → `append_message` → legal transitions → `add_artifact` → `get` reflects history (2), artifacts (1), and `Working` state; `cancel` reaches terminal `Canceled` with the cancellation flag set, and the terminal task then rejects messages and transitions; a second task walks the full `Submitted → Working → Completed` chain. The illegal `Submitted → Completed` skip is rejected both by `transition_valid` (false) and by `transition` returning `StoreError::InvalidTransition` — typed, never a silent no-op.
- **A1 `unknown_state_rejected`:** `TaskState::from_name` returns `None` for `TASK_STATE_BOGUS`, `bogus-state`, the empty string, and a near-miss spelling, while known proto and legacy spellings still parse; serde decoding a `TaskState` or a whole `Task` payload carrying `TASK_STATE_BOGUS` fails (the crate's actual decode path); at the wire, a raw `ListTasks` request filtered by `TASK_STATE_BOGUS` is answered with JSON-RPC error **-32602** and no result, while the same request with a known state returns a result (control).
- **A2 `error_shape_typed_bounded`:** the real server's unknown-task error reaches the client as `A2aError::Rpc` with code **-32001**, message ("task not found: …"), and data (the `TASK_NOT_FOUND` ErrorInfo) preserved; a fake peer's error carrying attacker-shaped `data` (an instruction-like payload) round-trips **verbatim** in the typed error's `data` field — carried as data, never interpreted; a fake peer's error response larger than the client's `RESPONSE_BYTES_MAX` (4,194,304 bytes) is refused as `A2aError::Transport` on the response cap instead of being delivered; and a remote task fetched from the server does not enter or mutate a separate local `TaskStore` (length unchanged, remote id absent, local task's state untouched) — remote claims stay remote assertions.

## What happened

All four cases pass against `phlow-a2a` as it stands (programme rung 4, 2026-11-07; gate: `cargo test -p phlow-gauntlet --test task_88` — 4 passed / 0 failed; see the rung report for the full gate record). No product findings: every behaviour the original design discipline asked for is present — typed errors with preserved shape, bounded and untrusted error data, unknown states rejected at every layer tested, and remote tasks kept as data only.

History: the previous absence-probe version of this task (four bounded source scans, honest `fail` at `"seam"`) is superseded by this redesign under D-1059. Its diver-side observation stands separately and is unchanged: task-07 found diver's Lua task state machine *ignores* unknown state strings rather than rejecting them — diver-owned, outside this task's crate.

## Full technical depth

The driver keeps the gauntlet contract (`ID`/`NAME`/`KIND`, `CASES`, `run_case(case) -> CaseReport`, `run(ctx) -> TaskOutcome`) and returns `Pass` only when every case passes; a failing case names itself in `where_` with its assertion failures in `how`. Notable mechanics:

- The server fixture follows `phlow-a2a`'s own wire-test pattern (`tests/wire.rs`): `A2aServer::start(ServerConfig::default(), card, executor)` on loopback with an ephemeral port, a card whose interface URL uses the `:0` authority, and a one-artifact echo executor implementing `server::AgentExecutor`.
- The wire-level unknown-state probe cannot go through the typed client — `ListTasksParams.status` is a `TaskState`, so the bogus string is unconstructible there (itself part of the discipline). The probe therefore POSTs raw JSON-RPC and asserts on the server's error envelope; the server decodes params via `serde_json::from_value`, whose `TaskState` deserializer rejects unknown variants, surfacing as -32602.
- The bounded-error probe relies on the client's read path, not a post-hoc size check: `read_capped` reads through `take(RESPONSE_BYTES_MAX + 1)`, so an oversized body is refused during the read and surfaces as a transport error. The fake peer is a one-shot `std::net` listener in the driver; it reads one request and answers with the crafted body, and tolerates the client closing early (that early close *is* the bound working).
- Executor failures are deliberately *not* used as the typed-error source: in this crate an executor error becomes a `Failed` task with the reason as a status message, not a JSON-RPC error. The typed-error assertions therefore use genuine protocol errors (unknown task, crafted peer errors), which is what `A2aError::Rpc` exists to carry.

## Sources

- `crates/phlow-a2a` — the crate under audit (`registry.rs`, `store.rs`, `server.rs`, `client.rs`, `types.rs`; its `tests/wire.rs` for fixture patterns).
- D-1059 — the redesign ruling (presence audit replacing the refuted absence assertions).
- `~/workspace/gauntlet-design-tasks-71-100.md` — the original task-88 design (Wave 86–90), whose discipline this audit carries forward.
