# task-30: leader election under partition

**Kind:** nvim-lua · **Status:** fail (open) · **Wave:** 26–30 · **Commits:** pending (wave 26-30)

## ELI5

Leader election is how a group of workers picks exactly one boss. When
workers can fail or the network can split (a "partition": group A can't
talk to group B), the danger is two bosses — "split brain" — each
thinking it is in charge and issuing conflicting orders. A correct
election guarantees at most one leader at any time: workers vote or
compete, heartbeats detect a dead leader, and a partitioned minority
steps down instead of electing its own rival. "Correct" for this task
means the design's numbers hold: 3 workers elect exactly one leader, a
2-vs-1 partition keeps at most one leader, and a dead leader is
replaced within a bounded time.

## What this task attempts

- **Goal:** probe diver's real coordination modules for leader-election
  machinery — the design names the harness multi-worker coordination
  seam ("the herd adapter or supervisor may own it") — and show the
  election design holds under partition.
- **Mechanism:** `lua/gauntlet/task_30.lua`, a headless-Neovim recon
  probe that requires the real `ai.harness.supervisor`,
  `ai.harness.adapters.herd`, `ai.herd`, and `ai.herd.api` modules and
  reads their exported function tables for election APIs
  (elect/campaign/leader/heartbeat/quorum/partition). No network calls,
  no workers spawned — module-table inspection only.
- **Success criterion:** the design's election pass criteria (one
  leader; partition keeps at most one; bounded re-election).
- **Non-goals:** inventing an election. The design says an evidenced
  `where = "seam"` failure is the correct result when the design says an
  absent seam is valid.

## What happened

Fail, open seam — on the first and only attempt. None of diver's
coordination modules implements leader election. The supervisor manages
run lifecycles (spawn/create/finish/retry), the herd adapter translates
runs to remote workers, and `ai.herd`/`ai.herd.api` manage worker agent
processes (spawn/prompt/kill) — coordination of *tasks*, not election of
*leaders*. The probe:

- `probe_reports_seam_absence` (V): the probe completes against the
  real diver Lua tree and reports `where = "seam"` — "no leader
  election machinery".
- `probe_covers_all_design_named_modules` (V): the evidence shows all
  four design-named modules were inspected before concluding.
- `verdict_is_a_finding_not_a_probe_crash` (A): the `where` is neither
  "bootstrap" nor "lua-driver" — the probe ran to completion; a
  crashing probe must never masquerade as the seam finding.
- `zero_election_hits_recorded_explicitly` (A): the evidence records
  zero election-API hits explicitly and states the open design gap —
  the "at most one leader" invariant has no seam to assert against.

Fail-closed: if election APIs ever appear in these modules, the driver
reports `where = "recon"` (premise changed) instead of the seam
absence.

Diver-owned finding (flagged, never fixed on gauntlet authority): the
absence is in the diver repo's Lua modules. Adding election machinery
is diver product work, not gauntlet work.

## The fix — what changed and why

No fix — this is a documented design gap, never fixed under gauntlet
authority. The gauntlet-side work was making the absence check honest:

- **Changed:** `crates/phlow-gauntlet/lua/gauntlet/task_30.lua` (new)
  and `crates/phlow-gauntlet/src/tasks/task_30.rs` (new) — the probe
  inspects real module exports rather than asserting from memory, and
  distinguishes "seam absent" from "probe crashed" and "premise
  changed".
- **Why:** for a cross-repo (diver) seam, the worst failure is a probe
  that crashes and gets read as a finding. The three-way verdict
  (`seam` / `recon` / `bootstrap`+`lua-driver`) keeps those distinct.
- **Source:** diver `lua/ai/harness/supervisor.lua` (run lifecycle),
  `lua/ai/harness/adapters/herd.lua` (run↔worker translation),
  `lua/ai/herd/init.lua` + `lua/ai/herd/api.lua` (worker processes).
- **Validation agents:** the 2 validation tests
  (`probe_reports_seam_absence`,
  `probe_covers_all_design_named_modules`) assert the seam verdict and
  full module coverage.
- **Adversarial agents:** the 2 adversarial tests
  (`verdict_is_a_finding_not_a_probe_crash`,
  `zero_election_hits_recorded_explicitly`) rule out probe-crash
  masquerade and demand the zero-hit result be explicit.

## Full technical depth

The probe bootstraps the real harness (`require('ai.harness')` +
`setup({})` from `DIVER_LUA_DIR`), then `pcall(require, ...)` on each
of the four modules and scans their exported function names
(case-insensitive) for `elect`, `leader`, `campaign`, `ballot`,
`quorum`, `heartbeat`, `partition`. All four modules load; zero hits.
The supervisor's exports are run-lifecycle verbs (spawn/create/finish/
retry_run); the herd adapter's translate runs to remote herd workers;
`ai.herd`'s manage worker agent processes (spawn_agent/prompt_agent/
kill_agent/agents). None has a quorum rule, a heartbeat-based leader
detector, a term/epoch counter, or partition handling — the four
primitives a leader election needs. The "at most one leader" invariant
therefore has no seam to assert against: it is not violated, it is
unrepresentable.

What is missing for the design's election: a leader-election
primitive (campaign/vote/term), heartbeat-based failure detection with
a bounded re-election path, and partition handling that keeps the
minority side leaderless. The design gap: either diver's harness grows
election machinery for its multi-worker coordination or the
single-coordinator scope is documented as the intended boundary.

## Sources

- Primary: diver `lua/ai/harness/supervisor.lua`, `lua/ai/harness/adapters/herd.lua`,
  `lua/ai/herd/init.lua`, `lua/ai/herd/api.lua` (module export tables, inspected live).
- Driver: `crates/phlow-gauntlet/lua/gauntlet/task_30.lua` (recon probe).
- Driver: `crates/phlow-gauntlet/src/tasks/task_30.rs` (nvim-lua runner).
- Tests: `crates/phlow-gauntlet/tests/task_30.rs` (2V/2A).
