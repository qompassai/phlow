# task-108: cross-harness transfer (rust+lua, V)

**Kind:** rust+lua · **Status:** pass (replicates) · **Wave:** 106–110 · **Commits:** pending (wave 106-110)

## ELI5

A skill is just text. If the text is the real artifact, then loading it through a *different* program should give the exact same bytes — and the same score. This task evolves an F-bind skill in the Rust harness, freezes its exact text, loads it through Diver's **real** skill machinery (the actual `ai.mcp.skills` module running under headless Neovim), and checks the bytes match and both harnesses score it identically.

## What this task attempts

- **Goal:** prove the frozen skill is portable across harnesses: byte-exact procedure text through Diver's real load path, and matching scores in both harnesses.
- **Mechanism:** `src/tasks/task_108.rs` evolves an F-bind skill (3 seeds: 101, 102, 103), freezes body + slow-update markers + `final_protected`, and writes three variants per seed (s0, final, final+distractor) as `SKILL.md` files. `lua/gauntlet/task_108.lua` runs under headless Neovim against Diver's real API: `require('ai.mcp.skills')` → `setup({skills_dir})` → `scan()` → `get(name)` → `tool_def(skill).procedure` (note: `setup()` returns nil on success and raises on failure, so it is called via `pcall`; `scan()` returns `(loaded_count, errors)`, not a skill list; Diver scans recursively for files named `SKILL.md`). The procedure bytes are written back and the Rust side asserts byte-exactness.
- **Success criterion (pre-registered):** *replicates* = all nine procedure bodies byte-exact across harnesses AND both harnesses score above the no-skill baseline with matching scores; rank order preserved across variants.
- **Non-goals:** modifying Diver (read-only; SHA-verified).

## What happened

**Replicates** — the skill is byte-portable across harnesses:

- **V1:** all nine procedure bodies byte-exact (Rust bytes == Diver `tool_def(skill).procedure` bytes).
- **V2:** both harnesses score above baseline with matching scores. Baseline F-bind: **0.0000**. Seed 101: s0 **90 B / 0.0**, final **480 B / 1.0**, distractor **508 B / 1.0**. Seed 102: s0 **90 B / 0.0**, final **490 B / 1.0**, distractor **518 B / 1.0**. Seed 103: s0 **90 B / 0.0**, final **498 B / 1.0**, distractor **526 B / 1.0**. Rust and Diver scores matched on every variant.
- **A1 (adversarial to portability):** rank order preserved across variants in both harnesses (s0 < final = distractor) — the distractor bytes don't change the score, as expected.
- **A2:** an invalid skill name is rejected loudly (scan reports the error, get returns nil).

Diver probed: `c84352cc850d507df477706b9166b6541ebe9e1c` (main), read-only. Final evidence must run on primo against a SHA-verified read-only Diver copy; the local numbers above are development checks against the local Diver checkout, not final evidence.

## Full technical depth

The frozen artifact is body + slow-update markers + `final_protected` (not body alone — the markers and protected section are part of the skill). Variants use `<variants>/<name>/SKILL.md` plus `names.txt` because Diver scans recursively for `SKILL.md` filenames. The distractor variant appends a D_test-neutral line to test that byte differences outside the scored region don't perturb scores. Lua validated with `luac -p` per the repo's SKILLS.md Lua section (LuaJIT parse behavior; Diver's LuaLS strict settings apply to Diver's own code, not this driver).

## Primary evidence (scripted double + real Diver load path)

The Rust side is the scripted double; the Diver side is Diver's real skill machinery at the pinned SHA. Nothing here is presented as real-model evidence.

## Sources

- Diver `lua/ai/mcp/skills.lua` at `c84352cc850d507df477706b9166b6541ebe9e1c` (primary source for the load-path API)
- arXiv 2605.23904v2 §II (skill as frozen text artifact)
