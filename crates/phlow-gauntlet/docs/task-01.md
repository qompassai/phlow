# task-01: fan-out/fan-in verdict aggregation

**Kind:** nvim-lua · **Status:** pass · **Wave:** gauntlet-1 ·
**Commits:** none — working tree only (do-not-commit rule)

## ELI5

Imagine you are a foreman with five workers. You hand all five a job at the
same time (fan-out), wait until every single one reports back done, failed,
or walked off (fan-in), and then you write one summary: "the crew succeeded"
only if all five finished the job. This task proves that diver's agent
harness (`lua/ai/harness`) can do exactly that: launch 5 runs in parallel,
watch them with a deadline, lose none, and combine their final verdicts into
one correct aggregate. The "workers" are fakes — declared test doubles —
because what is under test is the harness's run lifecycle, not any real AI.

## What this task attempts

- **Goal:** launch 5 parallel harness runs, observe all 5 reaching terminal
  states, and aggregate their verdicts into one correct summary, losing no run.
- **Mechanism:** `crates/phlow-gauntlet/src/tasks/task_01.rs` spawns headless
  Neovim on `lua/gauntlet/task_01.lua`, which registers a fake `gauntlet`
  adapter at runtime via `registry.register_adapter`, drives
  `harness.run` / `harness.cancel` / `supervisor.tick` /
  `supervisor.finish`, and reads terminal states from the sink's
  `run.finished` events (`events.lua`).
- **Success criterion:** 5 runs launched, 5 terminal states observed, 5
  `run.finished` events in the sink, aggregate verdict correct per the rule
  (pass iff all 5 completed). The driver's `pass` means *the machinery was
  correct*; the aggregate pass/fail is data inside the evidence.
- **Non-goals:** real model workers (no LLM is invoked); the `herd` adapter;
  resume/retry paths; verdict *graders* (`verdict.lua` acceptance checks are
  not exercised — aggregation of run states is).

## What happened

Pass, first real attempt at the task logic. All three scenarios behave per
spec (observed verdicts, abbreviated):

- `default`: 5/5 terminal, `completed=3 failed=1 cancelled=1`,
  `aggregate=fail` — correct, since not all 5 completed.
- `all-succeed`: 5/5 terminal, `completed=5`, `aggregate=pass`.
- `adapter-raises`: 5/5 terminal, `completed=4 failed=1`, `aggregate=fail`;
  the raised run was finished as failed by the driver and counted, not lost.
- Unknown scenario (`bogus`, probed manually): driver verdict `fail` with
  `where=scenario`, exit code still 0.

Rust gates: `cargo fmt --check` clean, `cargo clippy --all-targets -D
warnings` zero warnings, `cargo test -p phlow-gauntlet --test task_01` 4/4
green (2 validation + 2 adversarial). Lua: `luac -p` clean, ≤100 cols.

## Where it went wrong

Two iterations were needed; one was my bug, two were environmental.

- **Stage:** Rust test harness, first `cargo test` run.
- **Symptom (mine):** all 4 tests failed with
  `driver reported fail at 'verdict': no JSON verdict line on stdout`, while
  the verdict JSON was visible on **stderr**.
- **Evidence:** framework's `parse_driver_verdict` scans only stdout;
  `nvim --headless -l` sends Lua `print()` to stderr (verified:
  `print("via-print")` → stderr, `io.stdout:write("via-stdout\n")` →
  stdout, same binary, same flags).
- **Root cause:** my driver's `emit()` used `print()`. Under
  `nvim --headless -l`, `print` writes to stderr, so the framework never saw
  the verdict line.
- **Environmental (not mine, fixed by others mid-session):**
  (a) `is_driver_script_name` in `src/lib.rs` had an off-by-one
  (`len == 12`, `bytes[7..12]`) rejecting the contract-mandated
  `task_01.lua`; the scaffold owner fixed it in-tree (now `task_NN.lua`,
  underscore, plus unit tests). (b) `src/tasks/task_05.rs` (parallel
  worker's file) had the same `phlow_gauntlet::`-vs-`crate::` compile error
  mine initially had; that worker fixed their own file. I touched neither.

## The fix — what changed and why

- **Changed:** `lua/gauntlet/task_01.lua`, `emit()`: replaced
  `print(vim.json.encode(verdict))` with
  `io.stdout:write(vim.json.encode(verdict) .. '\n')` + `io.stdout:flush()`,
  with a comment explaining why.
- **Commit:** none (working tree only).
- **Why:** the framework contract (`run_nvim_lua_driver`, `src/lib.rs`)
  parses the verdict exclusively from the child's stdout; `print()` does
  not reach stdout under `nvim --headless -l`. Writing the file descriptor
  directly is the minimal change that satisfies the contract, and it keeps
  the "exactly one JSON line on stdout, always exit 0" guarantee.
  Alternatives rejected: writing the verdict to a file (contract says
  stdout); using `vim.fn.writefile('/dev/stdout', …)` (less portable,
  same effect).
- **Source:** primary — `src/lib.rs` `parse_driver_verdict` (reads
  `out.stdout` only); empirical — the `/dev/stdin` probe above showing
  `print` → stderr under the exact flags the framework uses
  (`--headless -l`).
- **Validation agents:** `cargo test -p phlow-gauntlet --test task_01`
  (4/4 green after the fix); manual `-l` runs of all three scenarios plus a
  bogus scenario, checking the single JSON line lands on stdout and the
  exit code is 0.
- **Adversarial agents:** the two adversarial tests —
  `adapter_start_raises_still_terminates_and_counts` (start() raises;
  run still terminates failed and is counted) and
  `cancel_during_fanout_yields_cancelled_and_aggregate_fail`
  (mid-run cancel → cancelled state, aggregate fail). Both green.
- **New convention:** under `nvim --headless -l`, never use `print()` for
  machine-readable driver output — always `io.stdout:write()` + flush.
  Rationale: `print` goes to stderr in this mode (verified), and every
  gauntlet Lua driver speaks to the framework through stdout. Evidence:
  this task's fix and the 4 green tests.
- **Citations:** `~/workspace/repos/phlow/crates/phlow-gauntlet/src/lib.rs`
  (`run_nvim_lua_driver_with_env`, `parse_driver_verdict`);
  `~/workspace/repos/diver/lua/ai/harness/{init,supervisor,registry,adapter,types,events}.lua`.

## Full technical depth

**Adapter choice (option a: fake, runtime-registered).** `registry.lua`
exposes `register_adapter(registry, name, adapter)` publicly: it validates
the name against `^[a-z][a-z0-9_]*$` and requires `probe`/`start`/`cancel`/
`close` functions — nothing else. The driver reaches the live registry at
`harness._state.registry` (populated by `setup()`) and registers a
`gauntlet` test double. The herd adapter (option b) was rejected: its
`start()` requires `ai.herd.spawn_agent` — real worker processes with
process ownership and remote topology — which adds nondeterminism, real
subprocess side effects, and a dependency on herd's runtime state, while
testing nothing extra about the harness lifecycle. The fake drives the
*real* path: `registry → supervisor.launch → transition → tick →
drain_completions → finish`; only the worker side is faked, and the fake is
declared in the driver and in the evidence (`adapter=gauntlet (fake,
runtime-registered test double)`).

**Per-run behavior** is selected via `spec.extensions.gauntlet.behavior`
(`supervisor.create` stores `extensions` on the run, so `start(run, sink)`
can read it): `succeed` appends `model.completed{outcome='completed'}`;
`fail` appends `model.completed{outcome='failed', error=…}`;
`pending` appends nothing (stays `running` until the driver cancels it);
`raise` records the run id in `FAKE.last_raised_run_id` and then calls
`error()`.

**The raise path — verified, not assumed.** `supervisor.launch()` calls
`chosen.start(run, sup.sink)` with *no* `pcall`, and `harness.run()`
doesn't wrap `start_run` either — so a raising `start()` propagates as a
Lua error out of `harness.run()`, leaving the run stranded in `queued`
(no `run.finished`, no id returned to the caller). The driver therefore
wraps each launch in `pcall(harness.run, spec)`; on a raise it reads the
id the fake recorded *before* raising and calls
`supervisor.finish(sup, id, 'failed', reason)` — `queued → failed` is a
legal edge in `types.TRANSITIONS` — so the run emits exactly one
`run.finished` and is counted. This is the mechanism the
`adapter-raises` scenario proves: 5 terminal, 5 finished events, 0 lost.

**Polling.** Headless `-l` runs no timer, so the driver pumps
`supervisor.tick(sup, types.now_ns())` itself in a bounded loop
(`POLL_TIMEOUT_MS = 15000`, 25 ms `vim.wait` steps — `vim.wait` verified
working headless). `tick` → `drain_completions` converts each new
`model.completed` event into `supervisor.finish`, which moves
`running → completed|failed` and emits `run.finished` exactly once per
attempt (guarded by `run._terminal_emitted`). In `default`, after the
first tick the driver calls `harness.cancel()` on the `pending` run
(`supervisor.cancel`: generation bump, `pcall(adapter.cancel, …)`,
`running → cancelled`). The loop exits when `types.is_terminal` holds for
all 5 run states; otherwise the driver fails honestly at `fan-out` with
the deadline.

**Fan-in.** For each run id the driver reads `sup.sink:events(id)`
(`events.lua` — note: there is no `sink.lua`; the sink constructor is
`events.new_sink()`, wired in `init.setup`), counts `run.finished`
events, and takes the last event's `payload.state`/`payload.reason`,
cross-checked against `supervisor.get(sup, id).state`. Aggregate rule:
pass iff `finished_events == 5 and lost == 0 and completed == 5`. A run is
"lost" if it has ≠1 finished event or its state is missing/non-terminal.

**Verdict semantics.** The JSON verdict's `outcome` reports whether the
*orchestration machinery* behaved correctly (5 launched, 5 terminal, 5
finished events, correct aggregate per the rule). The aggregate itself
(`aggregate=pass|fail` plus per-state counts) is evidence. This is why the
`default` scenario — 3 completed, 1 failed, 1 cancelled — yields driver
`pass` with `aggregate=fail`: the correct answer to "did all 5 succeed?"
is no, and producing that answer without losing a run is the task.

**Failure handling in the driver.** Every stage returns structured errors;
`main()` wraps `run_attempt()` in `xpcall` so even an unexpected Lua
error becomes a `fail` verdict line (with `where`/`how`) and the process
still exits 0 — the verdict carries the outcome, never the exit code.
Evidence is capped at 64 lines. Nothing is written outside
`GAUNTLET_WORK_DIR` (in practice the driver writes nothing to disk at
all); the diver repo is only read.

## Sources

- Primary: `~/workspace/repos/diver/lua/ai/harness/init.lua` (`setup`,
  `run`, `cancel`, `_state` shape); `supervisor.lua` (`launch` — unguarded
  `chosen.start`, `create`, `start_run`, `cancel`, `finish`, `tick`,
  `drain_completions`, `get`); `registry.lua` (`register_adapter`,
  `register_builtins`); `adapter.lua` (adapter contract, `probe`);
  `types.lua` (`RUN_STATES`, `TERMINAL_STATES`, `TRANSITIONS`,
  `is_terminal`, `validate_run_spec`); `events.lua` (`new_sink`,
  `append`, `events`, `count` — the sink the brief called `sink.lua`,
  which does not exist); `adapters/herd.lua` (`start` requires
  `ai.herd.spawn_agent` — the reason option (b) was rejected).
- Framework: `~/workspace/repos/phlow/crates/phlow-gauntlet/src/lib.rs`
  (`run_nvim_lua_driver_with_env`, `parse_driver_verdict`,
  `is_driver_script_name`); `src/tasks/task_01.rs`, `tests/task_01.rs`,
  `lua/gauntlet/task_01.lua` (this task's files).
- Secondary: none — every behavior claim above was read from the source
  or observed in a run.
