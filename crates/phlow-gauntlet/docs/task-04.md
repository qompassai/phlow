# task-04: unknown adapter terminates invalid_adapter

**Kind:** nvim-lua · **Status:** fail (open) — driver behavior verified pass;
Rust integration blocked by a framework bug outside this task's scope ·
**Wave:** gauntlet-20 · **Commits:** phlow @ 7076c8a, diver @ c84352c
(uncommitted work; nothing pushed per task rules)

## ELI5

Diver (Matt's Neovim config) has an "agent harness": a small control plane
that starts and supervises agent runs. Each run names an *adapter* — the
protocol translator that will actually do the work (acp, a2a, mcp, phlow,
rose, herd). This task asks a simple safety question: what happens if you
ask the harness to start a run with an adapter name that doesn't exist,
like `definitely-not-an-adapter`? "Correct" looks like this: the harness
says no, marks the run failed, and leaves a diagnostic that names the bogus
adapter — no crash, no hang, no pretending it worked. Think of it as
testing that a bouncer actually checks IDs instead of waving everyone
through.

## What this task attempts

- **Goal:** verify, by direct observation, that `harness.run()` with a bogus
  adapter name fails the run with an `invalid_adapter:` diagnostic.
- **Mechanism:** `crates/phlow-gauntlet/lua/gauntlet/task_04.lua` drives
  diver's real harness (`lua/ai/harness/init.lua` → `supervisor.lua` →
  `registry.lua`) inside headless Neovim
  (`~/workspace/tools/neovim-nightly/bin/nvim --headless -l`); the Rust
  module `crates/phlow-gauntlet/src/tasks/task_04.rs` selects the scenario
  and forwards the driver's JSON verdict via
  `phlow_gauntlet::run_nvim_lua_driver_with_env`.
- **Success criterion:** the observed behavior matches the reported
  contract — `run()` returns `nil, "unknown adapter: <name>"`, the run ends
  in state `failed`, and the `run.finished` event carries the reason
  `"invalid_adapter: unknown adapter: <name>"` — with the exact diagnostic
  quoted in evidence.
- **Non-goals:** fixing or changing the harness, the gauntlet framework, or
  any file outside the four owned files; adapter negotiation when no
  adapter is named; the phlow Phase-6 transport itself.

## What happened

The harness behavior was verified **pass** by direct observation across all
four scenarios (first attempt; each run printed exactly one JSON verdict
line and exited 0):

- **default** (`adapter="definitely-not-an-adapter"`): `run()` returned
  `nil, "unknown adapter: definitely-not-an-adapter"`; the run ended in
  state `failed`; the `run.finished` event reason was exactly
  `"invalid_adapter: unknown adapter: definitely-not-an-adapter"`.
- **path-traversal** (`adapter="../evil"`): identical contract shape —
  `run()` returned `nil, "unknown adapter: ../evil"`, run `failed`, reason
  `"invalid_adapter: unknown adapter: ../evil"`. The parent directory
  listing was byte-identical before and after (48 entries both times): no
  filesystem write occurred, because adapter lookup is a plain registry
  table miss (`registry.adapters["../evil"]`), never a `require()` or path
  resolution.
- **empty** (`adapter=""`): clean validation error —
  `run()` returned `nil, "run spec.adapter must be a non-empty string when
  given"` and **zero** runs were created (validation happens in
  `types.validate_run_spec` before any run exists).
- **phlow-stub** (`adapter="phlow"`): does NOT behave like an unknown
  adapter. `run()` returned
  `nil, "phlow adapter start is Phase 6 work: use native ai.phlow API"`;
  the run still ended `failed`, but the `run.finished` reason was the raw
  start error with **no** `invalid_adapter:` prefix. Reason: `init.lua`'s
  `M.run` calls `supervisor.finish(..., 'failed', 'invalid_adapter: ' ..
  err)` after `launch()` already moved the run to `failed`, and the
  transition table forbids `failed → failed` — that second finish bounced
  off and was recorded as a `diagnostic.observed` event
  `{kind="invalid_transition", from="failed", to="failed"}` instead.

No panic, no hang, no silent success in any scenario.

The **Rust integration tests fail (0/4)** — not because of the task
behavior, but because of a pre-existing framework bug in
`crates/phlow-gauntlet/src/lib.rs` (outside this task's four files, left
unfixed per scope rules). See the next section.

## Where it went wrong

- **Stage:** `cargo test -p phlow-gauntlet --test task_04` — all four tests
  panic in the `run_scenario` → `run_nvim_lua_driver_with_env` spawn path
  before Neovim is ever launched.
- **Symptom:** every test reports
  `where: spawn, how: rejected driver script name 'task_04.lua' (want task-NN.lua)`.
- **Evidence:** `is_driver_script_name` in `src/lib.rs` reads:
  `bytes.len() == 12 && &bytes[0..5] == b"task-" && ... && &bytes[7..12] == b".lua"`.
  A standalone replication (same predicate, compiled with the repo's
  rustc) returns `false` for `"task-04.lua"`, `"task_01.lua"`,
  `"task-20.lua"` — all of them. The name `"task-04.lua"` is 11 bytes, so
  the length gate rejects it; worse, even a 12-byte input could never pass
  because a 5-byte slice (`bytes[7..12]`) can never equal the 4-byte literal
  `b".lua"`. The gate is unsatisfiable: **no** driver script name the
  framework's own docs describe (`task-NN.lua`) can ever spawn. This blocks
  every nvim-lua gauntlet task, not just task-04.
- **Root cause:** off-by-one in the name gate — the author counted 12
  where the real shape is 11 (`task-` + 2 digits + `.lua`), and sliced one
  byte too many for the suffix. Verified by the replication, not guessed.

## The fix — what changed and why

No fix was applied: `src/lib.rs` is outside this task's four owned files
(task rules), and the gauntlet program treats a documented failure as
success while a faked pass is the only real failure. What changed within
scope:

- **Changed:** `lua/gauntlet/task_04.lua` (created) — four-scenario driver;
  `src/tasks/task_04.rs` (rewrote stub) — `run`/`run_scenario` per the
  driver contract; `tests/task_04.rs` (created) — 2 validation + 2
  adversarial tests exactly as briefed; this doc.
- **Commit:** none (task rules: do not commit).
- **Why this shape:** the driver asserts the *exact* strings the harness
  produces (pinned by a pre-write headless probe), so any future harness
  change surfaces as a loud mismatch instead of a silent pass. The
  `phlow-stub` scenario asserts only structural facts (error returned, run
  failed, no `invalid_adapter:` assumption) and records the rest verbatim.
- **Proposed framework fix (for the lib.rs owner, not applied):** change
  the gate to `bytes.len() == 11 && &bytes[0..5] == b"task-" &&
  bytes[5].is_ascii_digit() && bytes[6].is_ascii_digit() &&
  &bytes[7..11] == b".lua"`. Then re-run
  `cargo test -p phlow-gauntlet --test task_04` — the four tests should go
  green with no test-code changes, since the driver is already verified
  end-to-end. **Source:** the function's own doc comment in `src/lib.rs`
  ("`script` must have the shape `task-NN.lua`"), which the implementation
  contradicts.

## Full technical depth

The call path, from the top, using diver @ c84352c:

1. `harness.run(spec)` (`lua/ai/harness/init.lua`): `supervisor.create`
   validates the spec via `types.validate_run_spec` (this is where `""`
   dies: `'run spec.adapter must be a non-empty string when given'` — no
   run object is ever allocated), then `supervisor.start_run(run.id,
   spec.adapter)`.
2. `start_run` moves the run `created → queued` (valid per the transition
   table), then `launch()` does `registry.get_adapter(sup.registry, name)`
   — a direct table index, no `require`, no filesystem touch. For a bogus
   name this returns nil → `launch` returns
   `nil, "unknown adapter: <name>"`. (Built-in adapters were already
   `require`d once at `setup()` time by `register_builtins`; the bogus
   name never reaches the loader.)
3. Back in `M.run`, the failure branch calls
   `supervisor.finish(run.id, 'failed', 'invalid_adapter: ' .. start_err)`.
   The run is still `queued`, and `queued → failed` is a legal edge, so
   the run lands in `failed` and the sink records `run.finished` with
   `reason = "invalid_adapter: unknown adapter: <name>"`. `M.run` itself
   returns the *unprefixed* `nil, "unknown adapter: <name>"` — the prefix
   lives only in the event payload, a distinction the driver's assertions
   encode.
4. For the `phlow` stub, `launch` finds the registered adapter and calls
   `start()`, which returns
   `nil, "phlow adapter start is Phase 6 work: use native ai.phlow API"`
   (`lua/ai/harness/adapters/phlow.lua`). `launch` then transitions the
   run `queued → failed` itself with that raw string as the reason and
   returns the error. `M.run`'s failure branch now calls `finish(...,
   'failed', 'invalid_adapter: ...')` on an *already-failed* run; the
   transition table has no `failed → failed` edge, so `transition`
   appends `diagnostic.observed {kind="invalid_transition"}` and returns
   an error that `M.run` discards. Net effect: run `failed`, finished
   reason is the raw start error, and there is a visible
   `invalid_transition` diagnostic — arguably a wart (the
   `invalid_adapter:` prefix is lost and a spurious diagnostic is emitted),
   but it is honest and observable rather than silent.

Budgets and bounds in the driver: one synchronous `harness.run` per
process (nothing async — no hang vector); `timeout_ms = 30000` on the
spec; evidence capped at 64 lines, newlines stripped so the verdict is
always exactly one JSON line; `pcall` around `main()` so a Lua error
still yields a `fail` verdict and exit code 0. The driver writes nothing
anywhere; the only filesystem assertion (parent-dir snapshot for
`path-traversal`) is read-only.

## Sources

- Primary: `~/workspace/repos/diver/lua/ai/harness/init.lua` (`M.run`,
  the `invalid_adapter:` prefix site); `supervisor.lua` (`launch`,
  `start_run`, `finish`, `TRANSITIONS` via `types.lua`); `registry.lua`
  (`get_adapter` plain table lookup, `register_builtins`);
  `types.lua` (`validate_run_spec`, transition table);
  `adapters/phlow.lua` (`start()` error string); `events.lua` (sink,
  `run.finished`/`diagnostic.observed` envelopes).
- Primary: `~/workspace/repos/phlow/crates/phlow-gauntlet/src/lib.rs`
  (`is_driver_script_name`, `run_nvim_lua_driver_with_env`,
  `parse_driver_verdict`); headless probe output and the four direct
  driver runs (evidence quoted above).
- Secondary: the task brief's reported contract ("bad adapter marks the
  run failed with an `invalid_adapter:` diagnostic") — confirmed by
  observation for unknown adapters, refined for the phlow stub.
