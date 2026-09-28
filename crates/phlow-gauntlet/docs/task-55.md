# task-55: dissent escalation

**Kind:** nvim-lua · **Status:** fail (open) · **Wave:** 51–55 · **Commits:** pending (wave 51-55)

## ELI5

Ask two different AI models the same question. If they agree, you can
trust the answer more; if they disagree, you should NOT pick one
silently — you escalate to a human, showing both answers, and do
nothing until the human decides. And if one model keeps being wrong,
you raise an alarm about that model and stop asking it until someone
reviews it ("quarantine"). The rule is: a dissented answer never gets
acted on without human review.

## What this task attempts

- **Goal:** probe diver's real multi-model answer path for comparison
  machinery — the design names the model-adapter seam ("the harness
  adapter or a comparison layer may own it") — and show disagreement
  escalates to a human with both answers quoted, never a silent
  majority.
- **Mechanism:** `lua/gauntlet/task_55.lua`, a headless-Neovim recon
  probe that requires the six real harness adapters (rose, phlow,
  a2a, herd, mcp, acp — the modules that return answers to prompts)
  and `ai.harness.verdict`, reading their exported function tables
  for comparison APIs (compare/dissent/escalat/quarantine/agreement).
  `ai.security` is probed as a CONTROL SAMPLE: it genuinely exports
  `quarantine`, so its needle matches are recorded as evidence and
  explicitly classified unrelated (file-scanning escalation, not
  model-answer dissent) — never counted as comparison-path hits.
  Without this separation the probe would false-positive into
  `where = "recon"`. No network calls, no workers spawned —
  module-table inspection only.
- **Success criterion:** the design's escalation pass criteria
  (human escalation on disagreement with both answers quoted;
  dissent-rate alert + quarantine for a systematically-wrong model).
- **Non-goals:** inventing a comparison path. The design says an
  evidenced `where = "seam"` failure is the correct result when the
  design says an absent seam is valid.

## What happened

Fail, open seam — on the first and only attempt. No multi-model
answer comparison path exists. The adapters start workers and return
handles; nothing feeds two models' answers into a comparator; the
verdict module grades ONE run; escalation/quarantine in
`ai.security` concern suspicious FILES (composite security verdicts;
quarantine moves a file to an isolated directory), not model answers.
The probe:

- `probe_reports_seam_absence` (V): the probe completes against the
  real diver Lua tree and reports `where = "seam"` — "no multi-model
  answer comparison path".
- `security_escalation_is_file_scanning_not_model_dissent` (V): the
  evidence shows `verdict.evaluate` grades one run, and `ai.security`
  matches the needles but is explicitly documented as a control
  sample — file-scanning escalation, unrelated to model dissent.
- `verdict_is_a_finding_not_a_probe_crash` (A): the `where` is neither
  "bootstrap" nor "lua-driver" — the probe ran to completion; a
  crashing probe must never masquerade as the seam finding.
- `escalation_record_counter_and_threshold_all_absent` (A): no
  escalation record quoting both answers, no dissent-rate counter
  per model, no quarantine-threshold named constant — the fail-closed
  facet records zero comparison-API hits explicitly.

Fail-closed: if comparison APIs ever appear on these paths, the
driver reports `where = "recon"` (premise changed) instead of the
seam absence. The "no action on a dissented answer without human
review" rule holds vacuously — no dissent is ever detected — which is
exactly the failure mode the design forbids: silent non-detection,
not silent majority.

Diver-owned finding (flagged, never fixed on gauntlet authority): the
absence is in the diver repo's Lua modules. Whether diver needs
human escalation/quarantine for heterogeneous-model disagreement is a
diver product decision, banked for Matt — not gauntlet work.

Distinct from task-19: the council there votes and a tie resolves to
the safe default (Revise). This design refuses to resolve
machine-side at all — and there is no machine-side path to refuse
with.

## The fix — what changed and why

No fix — this is a documented design gap, never fixed under gauntlet
authority. The gauntlet-side work was making the absence check honest:

- **Changed:** `crates/phlow-gauntlet/lua/gauntlet/task_55.lua` (new)
  and `crates/phlow-gauntlet/src/tasks/task_55.rs` (new) — the probe
  inspects real module exports rather than asserting from memory, and
  distinguishes "seam absent" from "probe crashed" and "premise
  changed".
- **Why:** `ai.security` genuinely exports escalation/quarantine
  functions, so a naive needle scan would false-positive. The probe
  treats it as a control sample — matches recorded, classified as
  unrelated — so the zero-comparison-hit finding is evidenced, not
  assumed.
- **Source:** diver `lua/ai/harness/adapters/*.lua` (the six real
  adapters), `lua/ai/harness/verdict.lua` (per-run grading),
  `lua/ai/security/init.lua` (file-scanning control sample).
- **Validation agents:** the 2 validation tests
  (`probe_reports_seam_absence`,
  `security_escalation_is_file_scanning_not_model_dissent`) assert
  the seam verdict and the control-sample classification.
- **Adversarial agents:** the 2 adversarial tests
  (`verdict_is_a_finding_not_a_probe_crash`,
  `escalation_record_counter_and_threshold_all_absent`) rule out
  probe-crash masquerade and document all three absent artifacts.

## Full technical depth

The probe bootstraps the real harness (`require('ai.harness')` +
`setup({})` from `DIVER_LUA_DIR`), then `pcall(require, ...)` on each
of the six adapter modules plus `ai.harness.verdict` and
`ai.security`, scanning exported function names (case-insensitive)
for `compare`, `compar`, `dissent`, `dissensus`, `escalat`, `quarantin`,
`agreement`, `agree`, `adjudicat`, `verdict`. The adapter modules
export start/stop/prompt/handle verbs — they produce answers, they
never compare two of them. `ai.harness.verdict` exports `evaluate`
over one run's acceptance criteria: no second model, no comparison.
`ai.security` exports escalation/quarantine verbs, but for
suspicious FILES — its quarantine moves a file to an isolated
directory and its escalation sends file-scan alerts; neither takes a
model answer, and there is no dissent-rate counter or named
quarantine threshold constant anywhere in the answer path. No module
takes two models' answers and returns a comparison; no escalation
record quoting both answers exists; no per-model dissent-rate
counter exists. The design's "no action without human review" has no
detection to gate on.

What is missing for the design's escalation: a multi-model
comparison primitive (same prompt, N model answers, agree/disagree
output), an escalation record quoting both answers for the human,
a per-model dissent-rate counter with a named quarantine threshold,
and the review gate that blocks action on dissented answers. The
design gap: either diver's harness grows a comparison layer for
heterogeneous models or the single-model-answer scope is documented
as the intended boundary.

## Sources

- Primary: diver `lua/ai/harness/adapters/{rose,phlow,a2a,herd,mcp,acp}.lua`,
  `lua/ai/harness/verdict.lua`, `lua/ai/security/init.lua` (module
  export tables, inspected live).
- Driver: `crates/phlow-gauntlet/lua/gauntlet/task_55.lua` (recon probe).
- Driver: `crates/phlow-gauntlet/src/tasks/task_55.rs` (nvim-lua runner).
- Tests: `crates/phlow-gauntlet/tests/task_55.rs` (2V/2A).
