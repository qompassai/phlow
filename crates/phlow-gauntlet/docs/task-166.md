# task-166: pure transition function

**Kind:** rust (validation) · **Status:** pass · **Wave:** 27 · **Commit:** pending (wave 27)

## ELI5

Imagine a recipe book for a kitchen. You hand the book two things: what's on the counter right now (the state) and one new order ticket (the event). The book always gives back the same two things: the new counter layout (the new state) and a written prep list (the effects) — it never cooks anything itself. Task 166 checks two properties of that book. First, run a 200-ticket dinner service twice from scratch: the final counter and both prep lists must be byte-for-byte identical, and the service must have been real work (25 dishes, 200 tickets — not an empty kitchen). Second, prove the book *cannot* cook: scan its pages for any mention of ovens, phones, or clocks (file, network, and time APIs) and require zero hits.

## What this task attempts

- **Goal:** verify the reducer is pure and deterministic: same (state, event) always yields the same (state, effects), with effects recorded and never executed.
- **Mechanism:** `crates/phlow-gauntlet/src/state_machine.rs` — `reduce(&MachineState, &Event) -> Result<(MachineState, Vec<Effect>), ReduceError>`; `MachineState::canonical_bytes` and `canonical_effect_bytes` for byte comparison. Driver: `crates/phlow-gauntlet/src/tasks/task_166.rs` (`replay_once`, `replay_report`, `scripted_session`); integration tests in `crates/phlow-gauntlet/tests/task_166.rs`.
- **Success criterion:** V1 — 200-event scripted session replayed twice; final states and concatenated effect lists byte-identical; 25 tasks, version 200, non-empty effect list. V2 — source scan of `src/state_machine.rs` finds zero of the 12 forbidden I/O tokens.
- **Non-goals:** executing effects (task 167); hostile input at the boundary (task 168); the interpreter's transport behavior (task 169).

## What happened

PASS, all gates green on primo:

- **V1:** 200 events × 2 replays; `states_byte_identical=true`, `effects_byte_identical=true`; final state 25 tasks, version 200 (one per event), effect list non-empty. Reported metrics: `"events": 200, "replays": 2, "final_tasks": 25, "final_version": 200`.
- **V2:** static scan over `src/state_machine.rs` — 0 hits across all 12 tokens (`std::fs`, `std::net`, `std::io`, `std::time`, `std::process`, `std::thread`, `std::env`, `SystemTime`, `Instant`, `TcpStream`, `Command::`, `std::os::`); `reduce` confirmed defined.
- Integration tests: 2/2 pass (`replay_deterministic_200_events`, `reducer_imports_no_io`). Full gate suite: `cargo build`, `cargo test --lib` (64/64), `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` (0 warnings) — all clean.

## The fix — what changed and why

- **Changed:** `crates/phlow-gauntlet/src/tasks/task_166.rs` (+ `task_167.rs`, `task_168.rs`, `task_169.rs`) — added the missing `report.failures = failures;` assignment after `report.passed = failures.is_empty();`.
- **Commit:** pending (wave 27).
- **Why:** while debugging task-167's count failure, the instrumented run printed `failures.len()=1` locally but `report.failures=[]` — the drivers set `passed` from the local failure list but never copied the list into the report, so any future failure would surface as `passed=false` with an empty diagnosis. The assignment makes the verdict self-describing. For 166 itself the cases already passed, so this was preventive hardening of the shared driver pattern, applied uniformly.
- **Source:** the crate's own `CaseReport` contract (`src/skillopt/driver.rs`: "`failures`: Failing assertion details, empty when `passed`") — the field exists precisely to carry the diagnosis.
- **Validation agents:** worker re-ran the full gate suite on primo after the change: 10/10 integration tests, 64/64 lib tests, fmt and clippy clean — no regressions.
- **Adversarial agents:** none beyond the gate suite; the change is a pure reporting fix (no behavior under test altered).
- **Citations:** `crates/phlow-gauntlet/src/skillopt/driver.rs` (`CaseReport::pass`/`fail` definitions).

## Full technical depth

The purity claim has two halves, and the task tests them independently because they fail independently. Determinism (V1) is behavioral: `reduce` takes `&MachineState` and `&Event` and returns owned values; nothing inside reads a clock, a file, or a global — so replaying the same 200-event script (25 tasks × 8 events: spawn, start, progress 25, progress 50, block, unblock, progress 100, complete/fail) from `MachineState::default()` must produce identical bytes twice. The comparison is on `canonical_bytes` — a deterministic serialization, not `Debug` output — so byte-equality is a real claim about the state, not formatting. Non-triviality guards (25 tasks, version 200, non-empty effects) exist because byte-equality of two empty runs would prove nothing; version increments once per accepted event, so version 200 also proves every event was transition-legal.

Zero-I/O (V2) is structural, not behavioral: even a pure-looking function could smuggle `std::fs::read` behind a branch the tests never take. The scan reads the module source at compile time (`include_str!`) and fails on any of 12 tokens covering filesystem, network, process, environment, OS, and clock facilities. The mock interpreter lives in the same module and is itself in-memory only, so scanning the whole file is sound — a hit anywhere fails the case. This is deliberately crude (a comment mentioning `std::fs` would trip it), because a false positive is cheap (rename the comment) and a false negative would void the guarantee.

The Ghostex lineage: `gx-core`'s `core.rs` is organized as "events in, state plus effects out" — a pure core that returns effects for a separate interpreter to execute. The adaptation keeps exactly that split and nothing else: the domain is a small agent-task lifecycle (Queued → Running → Blocked/Done/Failed), not Ghostex's tabs/sidebar model. The per-event `reduce_*` functions and the shared `transition` guard (find task → require state → move state) keep every function under the 70-line Tiger Style ceiling without speculative abstraction.

## Sources

- `~/workspace/scratch/ghostex/packages/gx-core/src/core.rs` — the adapted pattern: pure core returning state plus effects; interpreter separate (primary source, Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500, MIT)
- `~/workspace/ghostex-recon/adaptation-map.md` — what was adapted (state-machine pattern) vs. what was not (tabs/sidebar/FocusSession domain)
- `crates/phlow-gauntlet/src/state_machine.rs` — `reduce`, `MachineState::canonical_bytes`, `canonical_effect_bytes`, `MAX_EFFECTS_PER_EVENT`
- `crates/phlow-gauntlet/src/tasks/task_166.rs`, `crates/phlow-gauntlet/tests/task_166.rs`
- `~/workspace/gauntlet-design-tasks-151-200.md` — task-166 design (Wave 27)
