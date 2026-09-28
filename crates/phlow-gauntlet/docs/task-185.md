# task-185: no persistent MCP config

**Kind:** rust · **Status:** pass · **Wave:** 29 · **Commits:** pending (wave 29)

## ELI5

The temp worker must not leave anything behind — no config file edited, no "helpful" server entry registered, no cache that grows every time they visit. This task treats a fake MCP config directory as the canary: snapshot it, run the bridge, snapshot it again, and demand the two snapshots be byte-identical. Then it checks the server registry is clean after the run (no bridge entry left registered), and runs the whole thing ten times in a row to prove nothing accumulates — ten runs must leave exactly as much behind as zero runs.

## What this task attempts

- **Goal:** prove the bridge writes no persistent MCP configuration: config dir byte-identical across a run, registry clean afterward, ten sequential runs accumulate nothing.
- **Mechanism:** `crates/phlow-gauntlet/src/bridge.rs` (no config-file writes in any launch/shutdown path) driven by `crates/phlow-gauntlet/src/tasks/task_185.rs` (`fixture_config_dir`, dir snapshot hashing, registry scan); assertions in `crates/phlow-gauntlet/tests/task_185.rs`.
- **Success criterion:** `diff_empty=true` with `files=2` hashed before/after a run; `registry_entries=1` before (the pre-existing entry), `bridge_entry=false` and `registry_entries=0` new entries after; ten sequential runs → `runs=10`, `diff_empty=true`.
- **Non-goals:** the browser profile dir (Chromium's own `--user-data-dir` is ephemeral by design and removed at shutdown — task 178's census covers process cleanup); OS-level temp files outside the fixture.

## What happened

Pass on the second iteration: 3/3, `test result: ok. 3 passed; 0 failed; finished in 0.09s`. The first iteration failed all three cases from the shared driver-reporting bug (task 179's fix). No persistence defect was found; the bridge writes no config on any path.

Wave-wide honesty note: the wave's first `cargo build -p phlow-gauntlet` failed with **23 errors and 1 warning** before any test ran; all were fixed before the first green gate.

## Where it went wrong

The only failure was the cross-task reporting bug documented in task-179: every `finish_case` computed `report.passed` but never assigned `report.failures`, so all three cases reported failure with empty evidence. The no-persistence property held on its first real run. Kept per the template's honesty requirement.

## The fix — what changed and why

- **Changed:** `src/tasks/task_178.rs`–`task_185.rs` — `finish_case` now assigns `report.failures = failures` alongside the verdict (shared with task 179's fix).
- **Why:** see task-179. Recorded here because this task's first run was its victim.
- **Source:** internal `CaseReport` contract.
- **Validation agents:** 3/3 green after the reporting fix; no bridge code changed.
- **Adversarial agents:** the ten-sequential-runs case *is* the adversarial angle — a single clean run could hide a slow leak (one file per run); ten runs with `diff_empty=true` and `registry_entries=0` bound the accumulation to zero, not merely "small".

## Full technical depth

The fixture builds a fake MCP config surface: a config file plus a `servers/` directory (`fixture_config_dir`, two files → `files=2`). The case hashes both files before launch and after full shutdown (`config_dir_byte_identical`), so *any* write — content change, permission change reflected in a re-hash, added or removed file — flips `diff_empty` to false. The registry case plants one pre-existing server entry, runs the bridge, then scans for entries: it asserts the original entry is untouched (`registry_entries=1` total) and no bridge entry exists (`bridge_entry=false`). The ten-run case repeats launch→shutdown ten times against the same fixture and asserts `runs=10` with the post-run snapshot still byte-identical — accumulation has nowhere to hide across runs.

This is a property of the bridge's code paths, not of luck: no function in `bridge.rs` opens a config file for writing. The ephemeral browser profile (`--user-data-dir` under a temp dir, removed at shutdown) is the only filesystem footprint, and task 178's process census plus the temp-dir cleanup cover its removal.

The newline-delimited framing (task 179) is relevant here as the negative: all bridge↔agent communication crosses the stdio pipe as framed messages, so there is no channel *through which* a config write could be smuggled — the bridge's only outputs are pipe bytes and the browser process it owns. The cyclic-value policy (rejection, task 181) is declared for the wave: cyclic values are refused with `BridgeError::CyclicValue`, never serialized.

Ghostex lineage: the leave-no-trace bridge concept is adapted from maddada/Ghostex @ `c91146607205ac49303d1bcfe2fd6f9a86741500`. Ghostex's CEF/GPUI implementation was not ported; this is a Tiger Style Rust re-implementation of the concept.

## Sources

- `crates/phlow-gauntlet/src/bridge.rs` — launch/shutdown paths (no config writes)
- `crates/phlow-gauntlet/src/tasks/task_185.rs` — the three driver cases, `fixture_config_dir`, snapshot hashing
- `crates/phlow-gauntlet/tests/task_185.rs` — the integration assertions
- maddada/Ghostex @ `c91146607205ac49303d1bcfe2fd6f9a86741500` — adapted concept (CEF/GPUI implementation not ported)
