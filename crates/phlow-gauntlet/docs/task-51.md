# task-51: byzantine worker detection

**Kind:** nvim-lua · **Status:** fail (open) · **Wave:** 51–55 · **Commits:** pending (wave 51-55)

## ELI5

Imagine you send the same question to five assistants and take the
answer most of them give. That is Byzantine fault tolerance: some
workers may lie or "equivocate" (give different answers to different
people), and the system still has to produce the right answer — the
liars get outvoted, and the system names which workers lied. "Correct"
for this task means the design's numbers hold: 5 workers, 1 lies, and
the verdict is still right AND the liar is identified.

## What this task attempts

- **Goal:** probe diver's real multi-worker fan-out consumers for a
  verdict-over-multiple-workers path — the design names the harness
  ("the fan-out consumer or orchestrator may own it") — and show the
  5-workers-1-liar scenario both produces the correct verdict and
  identifies the dissenter.
- **Mechanism:** `lua/gauntlet/task_51.lua`, a headless-Neovim recon
  probe that requires the real `ai.a2a.fanout`,
  `ai.a2a.orchestrator`, `ai.harness.supervisor`, and
  `ai.harness.verdict` modules and reads their exported function
  tables for aggregation APIs
  (quorum/vote/majority/byzantine/dissent/equivocate). No network
  calls, no workers spawned — module-table inspection only.
- **Success criterion:** the design's detection pass criteria
  (correct verdict over 5 workers with 1 liar; dissenter identified;
  equivocation caught).
- **Non-goals:** inventing a verdict. The design says an evidenced
  `where = "seam"` failure is the correct result when the design says
  an absent seam is valid.

## What happened

Fail, open seam — on the first and only attempt. None of diver's
modules computes a verdict over multiple workers' answers. The fan-out
consumer and orchestrator collect attributed per-worker results in
order; the supervisor runs lifecycles; the verdict module grades ONE
run's acceptance criteria. The probe:

- `probe_reports_seam_absence` (V): the probe completes against the
  real diver Lua tree and reports `where = "seam"` — "no
  quorum/voting rule over worker results; no dissenter is identified".
- `verifiers_grade_single_runs_not_worker_quorums` (V): the evidence
  shows `verdict.evaluate` grades one run and the supervisor never
  compares workers' answers.
- `verdict_is_a_finding_not_a_probe_crash` (A): the `where` is neither
  "bootstrap" nor "lua-driver" — the probe ran to completion; a
  crashing probe must never masquerade as the seam finding.
- `attribution_exists_but_no_verdict_reads_it` (A): per-worker
  attribution fields exist in the real result shapes (agent / lang /
  spec index) and zero aggregation-API hits were recorded — the
  equivocation gap is documented, not assumed.

Fail-closed: if aggregation APIs ever appear in these modules, the
driver reports `where = "recon"` (premise changed) instead of the seam
absence.

Diver-owned finding (flagged, never fixed on gauntlet authority): the
absence is in the diver repo's Lua modules. Whether diver needs
Byzantine-result arbitration for its fan-out/ensemble paths is a
diver product decision, banked for Matt — not gauntlet work.

## The fix — what changed and why

No fix — this is a documented design gap, never fixed under gauntlet
authority. The gauntlet-side work was making the absence check honest:

- **Changed:** `crates/phlow-gauntlet/lua/gauntlet/task_51.lua` (new)
  and `crates/phlow-gauntlet/src/tasks/task_51.rs` (new) — the probe
  inspects real module exports rather than asserting from memory, and
  distinguishes "seam absent" from "probe crashed" and "premise
  changed".
- **Why:** for a cross-repo (diver) seam, the worst failure is a probe
  that crashes and gets read as a finding. The three-way verdict
  (`seam` / `recon` / `bootstrap`+`lua-driver`) keeps those distinct.
- **Source:** diver `lua/ai/a2a/fanout.lua` + `orchestrator.lua`
  (result collection), `lua/ai/harness/supervisor.lua` (run
  lifecycle), `lua/ai/harness/verdict.lua` (per-run grading).
- **Validation agents:** the 2 validation tests
  (`probe_reports_seam_absence`,
  `verifiers_grade_single_runs_not_worker_quorums`) assert the seam
  verdict and the per-run nature of the verifier.
- **Adversarial agents:** the 2 adversarial tests
  (`verdict_is_a_finding_not_a_probe_crash`,
  `attribution_exists_but_no_verdict_reads_it`) rule out probe-crash
  masquerade and demand the zero-hit result be explicit.

## Full technical depth

The probe bootstraps the real harness (`require('ai.harness')` +
`setup({})` from `DIVER_LUA_DIR`), then `pcall(require, ...)` on each
of the four modules and scans their exported function names
(case-insensitive) for `quorum`, `vote`, `voting`, `majority`,
`byzantine`, `dissent`, `dissenter`, `equivocat`, `agreement`,
`consensus`, `aggregate`, `arbitrat`. All four modules load; zero
hits. The fan-out consumer and orchestrator expose result-collection
verbs (start/stream/fanout over attributed results); the supervisor
exposes run-lifecycle verbs; `ai.harness.verdict` exposes a single
`evaluate` that takes one run's acceptance criteria. None takes N
worker answers and returns a verdict — the primitive Byzantine
detection needs — and none returns a dissenter identity. Per-worker
attribution exists in the result shapes (agent/lang/spec index), so
equivocation would be visible in principle and detected by nothing in
practice. The "5 workers, 1 liar, correct verdict, liar named"
criterion is therefore unrepresentable, not violated.

What is missing for the design's detection: a verdict-over-workers
primitive (quorum/majority rule over attributed answers), an
equivocation check (same worker, two different answers), and
dissenter-identification output. The design gap: either diver's
harness grows Byzantine-result arbitration for fan-out/ensemble
work or the single-answer-per-run scope is documented as the intended
boundary.

## Sources

- Primary: diver `lua/ai/a2a/fanout.lua`, `lua/ai/a2a/orchestrator.lua`,
  `lua/ai/harness/supervisor.lua`, `lua/ai/harness/verdict.lua`
  (module export tables, inspected live).
- Driver: `crates/phlow-gauntlet/lua/gauntlet/task_51.lua` (recon probe).
- Driver: `crates/phlow-gauntlet/src/tasks/task_51.rs` (nvim-lua runner).
- Tests: `crates/phlow-gauntlet/tests/task_51.rs` (2V/2A).
