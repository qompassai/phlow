# task-02: budget exhaustion fails closed

**Kind:** nvim-lua · **Status:** pass (with one framework bug documented, fix
proposed but out of scope — see below) · **Wave:** (not assigned in brief) ·
**Commits:** none (no-commit rule for this program)

## ELI5

An agent run gets an allowance — say, 2 turns to do its work. The worker
actually needs 5 turns. What should happen? The run must stop and be marked
as failed because it ran out of allowance. It must never pretend it
succeeded, and it must never come back to life as a success after the
allowance is gone. This task proves diver's agent harness does exactly that:
the allowance is a per-run budget, spending past it is rejected, the run
ends in the terminal `failed` state with the reason "budget exhausted", and
the harness's own verdict machine says the run did not succeed. A budget of
zero is rejected before the run is even created.

## What this task attempts

- **Goal:** prove a budget-exhausted harness run fails closed — terminal
  `failed`, `budget.exhausted` event, consumption ledger, non-success
  verdict — across three scenarios, with evidence.
- **Mechanism:** real code paths, no fakes in the budget logic —
  `crates/phlow-gauntlet/src/tasks/task_02.rs` (`run`/`run_scenario`)
  spawns `lua/gauntlet/task-02.lua` in headless Neovim
  (NVIM v0.13.0-dev-1721+g7dbca1c4e2); the driver builds a supervisor from
  diver's public harness modules (`lua/ai/harness/{events,registry,
  supervisor,budget,verdict}.lua`), launches a run with `budget =
  { turn = 2 }`, and spends turns through `supervisor.consume`, the same
  entry point real adapters use. `tests/task_02.rs` asserts on the
  driver's evidence (2 validation + 2 adversarial).
- **Success criterion:** default scenario — 5 turn-consumes against a
  2-turn budget ends with terminal state `failed`, a `budget.exhausted`
  event, ledger `limits.turn=2 used.turn=2`, and `verdict.evaluate` →
  `pass=false`; zero-budget scenario — `harness.run` returns nil +
  `"budget limit for turn must be a positive number"`; consume-after-
  exhaustion scenario — further consumes rejected, state stays `failed`.
- **Non-goals:** real protocol adapters (ACP/A2A/MCP/…); multi-kind
  budgets; wall-clock (`time_ms`) exhaustion; the harness policy engine.

## What happened

The task's own mechanism passed on the first attempt: all three driver
scenarios printed `pass` verdicts with full evidence on the first headless
run. The integration then surfaced two real defects *outside* the task's
four files — both documented below with byte-level evidence. With those
worked around in a `/tmp` sandbox copy only, the full gate set is green:
`cargo test -p phlow-gauntlet --test task_02` → 4/4 pass in 0.10 s;
`cargo clippy -p phlow-gauntlet --all-targets -- -D warnings` → zero
warnings; `rustfmt --check` on both Rust files → clean; `luac -p` on the
driver → clean.

## Where it went wrong

Two defects, both outside the task's four files, both blocking every
nvim-lua gauntlet task — not just this one.

**Defect A — `is_driver_script_name` rejects every name**
(`crates/phlow-gauntlet/src/lib.rs`):

```rust
fn is_driver_script_name(script: &str) -> bool {
    let bytes = script.as_bytes();
    bytes.len() == 12
        && &bytes[0..5] == b"task-"
        && bytes[5].is_ascii_digit()
        && bytes[6].is_ascii_digit()
        && &bytes[7..12] == b".lua"
}
```

`"task-02.lua"` is 11 bytes, so `len == 12` already fails — and for any
12-byte input, `&bytes[7..12]` is a 5-byte slice compared against the
4-byte literal `b".lua"`, which is false by length. The conjunction is
unsatisfiable: **no driver name can ever pass this gate**, so
`run_nvim_lua_driver_with_env` always fails at `"spawn"` with
`rejected driver script name 'task-02.lua' (want task-NN.lua)`.
Sibling workers' drivers (`task_01.lua`, `task_03.lua`, …) hit the same
wall. The intended check for the documented `task-NN.lua` shape is
`len == 11` with `&bytes[7..11] == b".lua"`. I did not fix the real repo
(out of scope — shared framework file, 19 siblings active); the fix was
applied only in the `/tmp` sandbox copy used for validation.

**Defect B — `print()` in `nvim --headless -l` writes to stderr.**
The framework parses the verdict from the child's **stdout**, but Lua's
`print()` under `nvim --headless -l` goes to stderr (verified empirically:
`print("via-print")` → stderr, `io.stdout:write(...)` → stdout on this
nvim build). The driver initially used `print()` and the verdict was
parsed as "no JSON verdict line on stdout". Fixed in the driver's `emit()`
(my file) by writing via `io.stdout:write(...)`.

Also observed while working: sibling workers concurrently edited
`src/tasks/task_01.rs` and `src/tasks/task_05.rs` with
`phlow_gauntlet::run_nvim_lua_driver_with_env` (unresolvable inside the
lib crate — must be `crate::`), briefly breaking whole-crate compilation.
Both were repaired by their owners during the session; I touched neither
file.

## The fix — what changed and why

- **Changed:** `lua/gauntlet/task-02.lua` — `emit()` now uses
  `io.stdout:write(vim.json.encode(verdict) .. '\n')` instead of `print()`.
  **Commit:** none (no-commit rule). **Why:** the framework contract
  requires the verdict on stdout; on this nvim build `print()` in `-l`
  scripts reaches only stderr. Alternatives rejected: writing the verdict
  to a file (contract says stdout; extra I/O outside the verdict channel
  adds surface), `vim.fn.writefile('/dev/stdout', …)` (less portable than
  `io.stdout`). **Source:** empirical probe on the pinned nvim binary
  (`NVIM v0.13.0-dev-1721+g7dbca1c4e2`), documented in the code comment.
  **Validation:** the 4 integration tests parse the verdict from stdout
  and pass. **Adversarial:** none beyond the stream probe; the failure
  mode (verdict on wrong stream) is now asserted by every test run.
- **Changed:** `tests/task_02.rs` — replaced a self-defeating negative
  assertion (`!"partial success"` matched the driver's own
  "no partial success" line) with a positive check for the
  `"verdict is NOT success"` evidence line. **Why:** the negative check
  could never pass alongside the driver's honest wording; the positive
  check asserts the same property without the substring trap.
- **Deliberately not changed:** `src/lib.rs` `is_driver_script_name`
  (Defect A). **Why:** shared framework file outside the task's 4-file
  scope, with 19 sibling workers active; the parent orchestrator must
  decide the fix since it changes every nvim-lua task's spawn path.
  Proposed one-line fix: `bytes.len() == 11` and
  `&bytes[7..11] == b".lua"`. Validated in sandbox only.

## Full technical depth

Budget shape → limiter mapping. A run spec's `budget` table flows through
`supervisor.create` (diver `lua/ai/harness/supervisor.lua`) into
`budget.new(spec.budget or DEFAULT_BUDGET_LIMITS)` (diver
`lua/ai/harness/budget.lua`). `budget.new` validates every entry:
unknown kinds rejected, and each limit must be a **positive** number —
`{ turn = 0 }` fails with `"budget limit for turn must be a positive
number"`, so `create` returns `nil, err` and `harness.run` propagates it
before any run object exists. That is the entire zero-budget path: no
adapter is ever consulted, no events are emitted.

Consumption. `supervisor.consume(sup, run_id, kind, amount)` delegates to
`budget.consume`, which checks `used + amount > limit` **without
mutating** on overflow and returns `false, "budget exhausted: turn"`.
On failure the supervisor appends a `budget.exhausted` event
(`{ kind = "turn" }`) to the sink and returns the error — so the third of
five 1-turn consumes against a 2-turn budget is rejected and the ledger
stays at `used.turn = 2`. Consumes 4 and 5 are rejected identically, each
emitting its own `budget.exhausted` event (observed seq 5, 6, 7).

Settlement. `supervisor.tick` → `drain_completions` scans new sink events;
on `budget.exhausted` it calls `finish(run_id, "failed", "budget
exhausted")`. Note the exact terminal state name: `types.lua` has no
`exhausted` terminal state — budget exhaustion is encoded as state
`failed` (a member of `TERMINAL_STATES`) with reason `"budget exhausted"`
plus the `budget.exhausted` event. The run's `run.finished` event carries
`{ state = "failed", reason = "budget exhausted" }`. A second settlement
attempt on later `budget.exhausted` events hits the transition table
(`failed → failed` is not an edge) and becomes a `diagnostic.observed`
`invalid_transition` event — the run cannot leave `failed`.

Verdict. The driver runs the harness's own `verdict.evaluate` (diver
`lua/ai/harness/verdict.lua`) with a `custom` acceptance item whose check
is `r.state == "completed"`. For the exhausted run this yields
`{ pass = false, checks = { { pass = false } } }` — the harness verdict
machinery itself reports non-success, so no partial "success" is
possible.

Post-exhaustion. Further `supervisor.consume` calls keep failing at the
budget layer (ledger frozen at 2/2); a further `tick` cannot transition
the run anywhere (see transition table above). The run stays `failed`
forever — never resurrected.

Adapter note (declared mock). The run launches through a local
`gauntlet_null` adapter (`probe`/`start`/`cancel`/`close`) registered on a
supervisor assembled from the public harness modules. It stands in for a
real protocol adapter because the budget path under test —
`supervisor.consume` → `budget.exhausted` → `tick` → `finish("failed")` —
never touches adapter behavior, and the six built-in adapters
(acp/a2a/mcp/phlow/rose/herd) are thin wrappers over live transports
unavailable in this sandbox.

## Sources

- Primary: `~/workspace/repos/diver/lua/ai/harness/budget.lua`
  (`M.new` positivity check, `M.consume` no-mutation-on-overflow);
  `supervisor.lua` (`M.create` budget wiring, `M.consume` event emission,
  `drain_completions` → `finish(..., "failed", "budget exhausted")`,
  `M.finish` outcome whitelist);
  `types.lua` (`TERMINAL_STATES`, `TRANSITIONS`, `BUDGET_KINDS`,
  `validate_run_spec`); `verdict.lua` (`M.evaluate`, custom-kind check
  semantics); `events.lua` (`new_sink`, `sink:events(run_id)`);
  `registry.lua` (`register_adapter` name pattern);
  `crates/phlow-gauntlet/src/lib.rs` (`is_driver_script_name`,
  `run_nvim_lua_driver_with_env`, `parse_driver_verdict`).
- Empirical: headless-nvim stream probe (print→stderr,
  io.stdout:write→stdout); full evidence transcripts from all three
  scenarios (in test output); `cargo test` 4/4 in 0.10 s (sandbox with
  Defect A worked around); `cargo clippy -- -D warnings` zero warnings;
  `rustfmt --check` clean on both Rust files; `luac -p` clean on driver.
- Secondary: none. Every behavior claim above was read from the harness
  source or observed in a run.
