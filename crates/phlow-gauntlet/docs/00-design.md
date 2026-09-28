# Phlow 20-Task Orchestration Gauntlet — Design

## Objective

Cue phlow (agent runtime, `qompassai/phlow`) to attempt 20 DIFFERENT difficult
tasks exercising **agent orchestration** with Matt's Neovim config (diver) in
the loop. Let it fail where it fails. Document every failure honestly with
evidence. Fix between iterations, citing what changed and why. Commit, push
(only primo-green trees, fast-forward, byte-verified), rebuild on primo, and
begin again until all 20 have played out. Document each discovery ELI5 +
cited with full technical depth.

## The 20 tasks

Kind `nvim-lua` = driven through headless Neovim running diver's actual
`lua/ai/harness` code. Kind `rust` = driven through phlow's Rust crates
(primarily `phlow-experiment`, plus `phlow-council`).

| # | ID | Name | Kind | Orchestration dimension under test |
|---|----|------|------|-------------------------------------|
| 1 | task-01 | fan-out/fan-in verdict aggregation | nvim-lua | 5 parallel harness runs (mixed adapters incl. mocks); supervisor aggregates verdicts into one |
| 2 | task-02 | budget exhaustion fails closed | nvim-lua | tiny step/token budget → run terminates `budget_exhausted`; no partial "success" verdict |
| 3 | task-03 | approval gate blocks unapproved tool use | nvim-lua | tool call needing approval with no approver → blocked; with approver → proceeds |
| 4 | task-04 | unknown adapter → invalid_adapter | nvim-lua | bogus adapter name → run terminates `invalid_adapter` (regression test of the fixed behavior) |
| 5 | task-05 | cancel/resume semantics | nvim-lua | cancel mid-run → `cancelled`; resume → completes; resume of a completed run → rejected |
| 6 | task-06 | ACP interop round-trip | nvim-lua | initialize/newSession/prompt through the ACP adapter against a mock ACP agent |
| 7 | task-07 | A2A task lifecycle | nvim-lua | message/send → tasks/get → tasks/cancel through the A2A adapter vs a mock JSON-RPC peer |
| 8 | task-08 | MCP stdio tool bridging | nvim-lua | tools/list → tools/call through the MCP adapter vs a mock stdio MCP server |
| 9 | task-09 | prompt injection via tool output | nvim-lua | tool returns "ignore previous instructions…"; output must stay data — no unapproved tool call follows (adversarial) |
| 10 | task-10 | malicious MCP tool description | nvim-lua | embedded instruction in a tool description is quoted/displayed, never executed (adversarial) |
| 11 | task-11 | self-approval rejected | rust | model-generated approval record cannot become `HumanApproval`; `PromotionGate` rejects it (adversarial) |
| 12 | task-12 | poisoned context compaction | rust | adversarial content in compacted context cannot resurface as instruction — via `phlow-agent::solpi::EvidenceReducer` (adversarial) |
| 13 | task-13 | scheduler overload | rust | submissions beyond `QUEUE_CAPACITY` → bounded rejection; scheduler state stays consistent |
| 14 | task-14 | evaluator budget exhaustion | rust | tiny budget → evaluation fails closed at the correct stage with a typed error |
| 15 | task-15 | lifecycle illegal transitions | rust | out-of-order `LifecycleEvent`s rejected; terminal states reject every event |
| 16 | task-16 | manifest validation | rust | malformed TOML suite/budget/promotion manifests → typed rejections; valid manifests parse |
| 17 | task-17 | nvim edit→check→fix loop | nvim-lua | harness-driven agent edits a Lua file, runs `luac` via job control, reads the error, fixes it, re-verifies clean |
| 18 | task-18 | worker crash recovery | nvim-lua | adapter run whose worker crashes → supervisor retries bounded times → `failed` verdict with cause chain |
| 19 | task-19 | council arbitration | rust | N agents return conflicting outputs → vote; majority wins; tie → `Revise` (the code's documented safe default — verify a tie can never yield `Keep`) |
| 20 | task-20 | deadline expiry / at-most-once | rust | expired deadline → `timed_out`; stale/late cancellation cannot resurrect or double-complete a run |

Each task's test module carries a 50/50 validation/adversarial case split.

## Driver architecture (new crate `phlow-gauntlet`)

- `crates/phlow-gauntlet/` — new workspace member.
- `src/lib.rs` — `TaskReport`, `TaskOutcome::{Pass,Fail}`, `Ctx` (paths to
  nvim binary, diver `lua/` dir, timeouts, budgets). Small, bounded.
- `src/tasks/mod.rs` + `task_01.rs` … `task_20.rs` — each module exposes
  `ID`, `NAME`, `KIND`, and `run(&Ctx) -> TaskOutcome`. Disjoint ownership:
  one worker per task file, no shared edits.
- `lua/gauntlet/task_NN.lua` — the Neovim-side driver for `nvim-lua` tasks.
  Run under `nvim --headless -l`; prints one JSON verdict line to stdout.
- `src/bin/gauntlet.rs` — CLI: `gauntlet list`, `gauntlet run <id>`,
  `gauntlet run-all --report-dir <dir>`.
- `tests/task_NN.rs` — one integration test file per task (validation +
  adversarial cases). The Rust test is the gate; for `nvim-lua` tasks it
  spawns the pinned headless nvim and asserts on the JSON verdict.
- `docs/gauntlet/task-NN.md` — per-task learning doc (ELI5 + cited + full
  depth): mechanism, what broke, why the fix works.

## Neovim-in-the-loop scoping (deliberate)

Headless nvim runs with diver's `lua/` directory on the runtimepath so
`require("ai.harness")` loads Matt's actual config code, but the full
`init.lua` is NOT bootstrapped (it would pull lazy.nvim plugin management
and network). The harness modules under test are byte-identical to his
config; the plugin layer is out of scope for orchestration testing. This
is documented, not hidden.

## Nix flake (Deliverable 1): `crates/phlow-gauntlet/flake.nix`

Matt's order: a flake that DIRECTS the 20 tasks — contained, reproducible,
sterile. Tiger Style Nix v1.0 (canonical guide) applies rigorously;
validation recipes: `nixfmt --check`, `statix check .`, `deadnix --fail .`,
`nix flake check`.

- Pinned inputs (`nixpkgs` nixos-25.05, `flake-utils`); `flake.lock` committed.
- `gauntletTasks`: single source of truth — 20 `{ id, name, kind, driver }`
  entries, with `assert lib.length == taskCountMax` (20, named constant).
- `apps.task-01` … `apps.task-20`: one `writeShellApplication` each,
  generated from the list by a bounded map; each runs its task driver inside
  the pinned environment and writes a JSON report.
- `apps.gauntlet`: runs all 20 in order, writes `report/gauntlet-report.json`.
- `apps.report`: pretty-prints the latest report.
- `devShells.default`: sterile env — pinned `cargo`, `rustc`, `neovim`,
  `lua`, `stylua`, `git`, `jq`. Documented: cargo needs network on first
  run for the crates.io registry (apps are not sandboxed); the toolchain
  itself is pinned.
- `checks`: `nix-lint` (nixfmt/statix/deadnix over the flake files) plus a
  structural check that all 20 apps exist. Task execution goes through
  `nix run` (documented boundary — checks stay hermetic).

Honesty note: no Nix evaluator exists in this sandbox, so `nix flake check`
and the task apps run on primo (which has Nix). Locally we run `deadnix`
and `statix` (available) on the Nix files; `nixfmt` is unavailable here, so
formatting is done by hand to the 2-space/100-col rule and flagged as
unverified until primo.

## Loop mechanics

- Work proceeds in waves of 5 tasks. One worker per task implements the
  driver + tests + doc draft and runs them locally, reporting outcomes
  honestly (pass or fail-with-evidence).
- The coordinator integrates: one commit per wave (disjoint paths), local
  gates (`cargo fmt --check`, `cargo clippy -p phlow-gauntlet -- -D warnings`,
  `cargo test -p phlow-gauntlet`), then primo full-workspace gates when SSH
  is reachable. Push to `main` ONLY the exact primo-green tree
  (fast-forward, never force, remote byte-verified). Rebuild on primo.
- If primo is unreachable at push time: commits stay local, bundles are
  kept, and the report says so — no pushing on local-only gates.
- Failures: documented in the task doc with evidence; fixes cite file +
  commit + rationale; the task re-runs next iteration.
- Progress reports every 10% (every 2 tasks) measured from real gate numbers.

## Fix loop (Matt's standing requirement, 2026-09-28)

For EVERY proposed fix — no exceptions:

1. **50/50 split.** Half the agents assigned to the fix are *validation*
   agents: they confirm the fix works, confirm no regression in neighboring
   behavior, and confirm the gates stay green. The other half are
   *adversarial* agents: they red-team the fix — try to break it, find the
   attack, the edge case, the regression the validators missed. Both sides
   report evidence, not opinions.
2. **Sourced winning choice.** The fix that lands must cite its authority at
   minimum: the primary source (upstream docs, protocol spec, paper, or the
   Tiger Style guide section) that makes this choice correct. "It felt
   right" is not a source.
3. **Idiomatic preference.** Among sourced options, prefer the one most
   idiomatic to the Karpathy principles (think before coding, simplicity
   first, surgical changes, goal-driven) fused with Tiger Style (safety >
   performance > DX, explicit contracts, bounded work, no hidden behavior).
4. **New conventions allowed — if validated.** If the work reveals a
   genuinely better pattern, it may become a new coding convention, but only
   when tests + gates prove it. Document the convention, its rationale, and
   its evidence alongside the fix — in the task doc AND in the repo's own
   `AGENTS.md` if it generalizes. Never establish a convention by assertion.

The per-task learning doc records all four: who validated, who red-teamed,
what the adversarial side found, which choice won and its source, and any
new convention with its evidence.

## Hard constraints (unchanged)

No autonomous self-modification. No self-approval or self-promotion. Human
promotion gates mandatory. Read-only defaults. Fail closed on missing
evidence / timeout / exhausted budget. Karpathy×Tiger lens: think before
coding, simplicity first, surgical changes, goal-driven with honest gate
reporting. Narrate the WHY — the decision process teaches.
