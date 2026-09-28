# task-05: cancel/resume semantics

**Kind:** nvim-lua · **Status:** fail (open) — task logic proven green via the
driver; the Rust spawn path is dead due to a framework bug in
`is_driver_script_name` (see below) · **Wave:** not named in task brief ·
**Commits:** none (rules forbid committing)

## ELI5

The diver agent harness runs "runs" — units of agent work with a lifecycle:
created → queued → running → completed/failed/cancelled/timed_out. This task
checks the lifecycle's integrity rules the way you'd check a bank ledger:
cancelling a run mid-flight must stamp it `cancelled` and record *why*; you
must be able to resume a cancelled run so it finishes exactly once (never
twice, never brought back from the dead); and asking to resume an already
finished run, or cancel a finished or nonexistent run, must be a clean
"no" that changes nothing. A fake slow worker stands in for a real AI
adapter so the timing is deterministic.

## What this task attempts

- **Goal:** prove cancel/resume lifecycle integrity of diver's `ai.harness`
  end to end: cancel mid-run with reason → resume → completes exactly once;
  resume-of-completed, cancel-of-terminal, and cancel-of-unknown are clean
  rejections.
- **Mechanism:** `crates/phlow-gauntlet/src/tasks/task_05.rs`
  (`run`/`run_scenario` → `phlow_gauntlet::run_nvim_lua_driver_with_env`)
  spawns headless nvim on `lua/gauntlet/task_05.lua`, which drives
  `harness.run/cancel/resume` against diver's
  `lua/ai/harness/{init,supervisor,types,registry,adapter,events}.lua` and
  prints one JSON verdict.
- **Success criterion:** `cancelled` with reason `gauntlet-test` observed;
  resumed run reaches `completed` exactly once (two `run.finished` events
  total: cancelled then completed); resume-of-completed refused with
  `invalid transition completed -> queued`.
- **Non-goals:** real protocol adapters (herd/acp/a2a/mcp/phlow/rose) are not
  exercised; parent/child run trees are not exercised; retry/backoff is not
  exercised.

## What happened

Iteration 1. The Lua driver was built, run directly through headless nvim,
and **all four scenarios pass with the verdict on stdout and exit 0**:

- `default`: `run()` → `running`; `cancel(run_id, "gauntlet-test")` →
  `cancelled`; `run.finished` carries `reason=gauntlet-test`; `resume()` →
  attempt 2 → `completed`; `run.finished` events total=2, completed=1
  (exactly one completion, no resurrection).
- `resume-completed`: run completes on attempt 1; `resume()` rejected with
  `invalid transition completed -> queued`; state stays `completed`.
- `cancel-terminal`: `cancel()` of the completed run rejected with
  `run is already terminal: completed`; state unchanged.
- `cancel-unknown`: `cancel("run-0000-bogus-id")` rejected with
  `unknown run: run-0000-bogus-id`; zero runs exist afterwards.

The Rust integration tests (`cargo test -p phlow-gauntlet --test task_05`,
2 validation + 2 adversarial) all fail — not in the task logic, but at the
framework's spawn gate: `rejected driver script name 'task_05.lua' (want
task-NN.lua)`. Root cause is a bug in the framework's
`is_driver_script_name` (next section). The task code follows the mandated
driver contract exactly; the framework cannot honor it for *any* task.

## Where it went wrong

- **Stage:** spawn — `run_nvim_lua_driver_with_env` → `is_driver_script_name`
  (`crates/phlow-gauntlet/src/lib.rs:322`).
- **Symptom:** every test panics with
  `task-05 scenario '<s>' failed at 'spawn': rejected driver script name 'task_05.lua' (want task-NN.lua)`.
  Test result: `0 passed; 4 failed`.
- **Evidence:** the check as committed:
  ```rust
  bytes.len() == 12
      && &bytes[0..5] == b"task-"
      && bytes[5].is_ascii_digit()
      && bytes[6].is_ascii_digit()
      && &bytes[7..12] == b".lua"
  ```
  `bytes[7..12]` is a 5-byte slice; `b".lua"` is 4 bytes. Slice equality
  requires equal lengths, so the final conjunct is false for **every**
  input. Additionally `len("task-05.lua") == 11 != 12`, and the brief's
  mandated `"task_05.lua"` fails the `task-` prefix too. Replicated logic in
  Python over `task_05.lua`, `task-05.lua`, `task-5.lua`, `task-123.lua`,
  `task-05X.lua`: all rejected, and no 12-byte string with a 5-byte tail can
  ever match `".lua"`. The entire nvim-lua driver path is dead code; all 20
  tasks are affected, not just task-05.
- **Root cause:** off-by-one in the framework's name validator (length 12 vs
  11, tail slice `[7..12]` vs `[7..11]`), verified — not guessed.

## The fix — what changed and why

No fix was applied: the bug is in `src/lib.rs`, which is outside the four
files this task may touch, and the rules forbid committing. What *was*
changed in this task's own files, per iteration:

1. **Driver verdict stream** (`lua/gauntlet/task_05.lua`): first version used
   `print()` for the verdict; the verdict appeared on stderr and the Rust
   parser (stdout-only) would never see it. Root cause: in
   `nvim --headless -l`, Lua `print()` routes to stderr (verified with a
   probe script: `print` → stderr, `io.stdout:write` → stdout). Changed to
   `io.stdout:write(vim.json.encode(verdict) .. '\n')`.
   **Source:** the probe run itself (primary evidence, quoted above).
2. **Rust call path** (`src/tasks/task_05.rs`): brief wrote
   `phlow_gauntlet::run_nvim_lua_driver_with_env(...)`; inside the crate that
   name does not resolve (`E0433`). Changed to
   `crate::run_nvim_lua_driver_with_env(...)`.
   **Source:** compiler error E0433.

**Proposed framework fix (for the coordinator, not applied):**
```rust
fn is_driver_script_name(script: &str) -> bool {
    let bytes = script.as_bytes();
    bytes.len() == 11
        && &bytes[0..5] == b"task-"
        && bytes[5].is_ascii_digit()
        && bytes[6].is_ascii_digit()
        && &bytes[7..11] == b".lua"
}
```
Note this accepts `task-05.lua` but still rejects the brief's literal
`"task_05.lua"` (underscore); the brief's driver contract and the
framework's validator disagree on the separator and should be reconciled.

**Validation:** `luac -p` clean on the driver; all four driver scenarios pass
directly via headless nvim (verdicts quoted in "What happened");
`cargo clippy -p phlow-gauntlet --all-targets -- -D warnings` zero warnings;
`cargo fmt` clean on both Rust files I own (the crate's pre-existing files
`src/bin/gauntlet.rs`, `src/lib.rs`, `src/tasks/mod.rs` already fail
`cargo fmt --check` — pre-existing drift, untouched).
**Adversarial:** the two adversarial tests exist but cannot execute past the
spawn gate; their scenarios were executed directly and the rejection
evidence is quoted above.

## Full technical depth

The harness lifecycle is a hierarchical state machine
(`diver/lua/ai/harness/types.lua:TRANSITIONS`): `completed` has no outgoing
edges, while `failed`/`cancelled`/`timed_out`/`interrupted` may go back to
`queued`. Terminal states emit `run.finished` exactly once per attempt
(`supervisor.lua:transition`, guarded by `run._terminal_emitted`).
`supervisor.cancel` is cooperative: bump the generation (stale adapter
callbacks are dropped), `pcall(adapter.cancel, handle, reason)`, then
transition to `cancelled` with the reason in the `run.finished` payload.
`supervisor.resume` re-queues a *terminal* run: attempt+1, generation+1,
reset the terminal guard, transition to `queued`, and re-`launch` through
the same adapter.

The driver's fake adapter (`gauntlet_slow`, registered programmatically via
`registry.register_adapter` into `harness._state.registry` — test
introspection, declared here as the one non-public seam used) honors the
adapter contract (`probe`/`start`/`cancel`/`close`). Its `start` reads
`run.extensions.gauntlet.complete_on_attempt`: attempt 1 only appends a
`diagnostic.observed` (stays `running`, so cancel is genuinely mid-flight);
later attempts append `model.completed` with `outcome = "completed"`, which
the supervisor's `tick()` drains into `finish(run, 'completed')`. The driver
pumps `supervisor.tick` itself in `wait_state` (bounded 15 s) since headless
nvim runs no harness timer.

Observed harness wart (flagged, not fixed — different repo, read-only
here): `supervisor.resume` mutates `run.attempt`, `run.generation`,
`run._terminal_emitted`, and `run.handle` *before* the `queued` transition,
so a rejected resume of a completed run still leaves the run mutated
(attempt 1→2, terminal guard reset, handle dropped). State stays
`completed` and the error is correct, but a future code path that
transitioned out of `completed` would double-emit `run.finished`. Worth a
look by whoever owns diver's harness: validate before mutating.

Timing: the `default` scenario runs in ~1.3 s wall clock (1.2 s deliberate
mid-flight pause); the other scenarios in <1 s. Work dirs:
`$TMPDIR/gauntlet-task-05-<scenario>/task-05`; the driver writes nothing.

## Sources

- Primary: `~/workspace/repos/diver/lua/ai/harness/{init,supervisor,types,registry,adapter,events}.lua`
  (state machine, cancel/resume mechanics, sink API); herd adapter
  `adapters/herd.lua` (spawns real workers — rejected as the slow worker for
  determinism).
- Primary: `~/workspace/repos/phlow/crates/phlow-gauntlet/src/lib.rs:322-329`
  (`is_driver_script_name` — the failing gate), `:232-318`
  (`run_nvim_lua_driver_with_env` contract).
- Primary evidence: direct headless-nvim runs of `lua/gauntlet/task_05.lua`
  (four scenario verdicts quoted above); the `print`→stderr probe;
  `cargo test` output (`0 passed; 4 failed` at `spawn`).
- Secondary: task brief's verified facts (harness API shape, repo SHAs) —
  taken as given, not re-derived.
