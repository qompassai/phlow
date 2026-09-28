# task-59: approval scope binding

**Kind:** nvim-lua · **Status:** fail (open) · **Wave:** 56–60 · **Commits:** pending (wave 56-60)

## ELI5

The human approves the robot to "read file A." The approval should
be a TICKET for exactly that job — not a blank hall pass the robot
can reuse to "write file B," or to "read file A" with different,
sneakier instructions attached. Binding means the approval paper
itself says WHAT it approved (the tool, the arguments, the files),
and whoever finally does the work CHECKS the paper against the real
job before acting. Diver's approval paper DOES say what it approved
— the binding is written down. But there is nobody at the door
checking the paper: the harness ships no executor that verifies the
binding, so the "ticket reuse is rejected" proof the design demands
cannot be demonstrated.

## What this task attempts

- **Goal:** probe the two-part scope-binding seam — (1) the approval
  RECORD binds the exact action + arguments (tool/argv/paths/
  endpoints), and (2) the EXECUTOR verifies the binding, proven by
  the design's replay rejections (approval for "read file A"
  presented for "write file B" must be rejected; approval for X with
  args (1,2) replayed for X with args (1,3) must be rejected).
- **Mechanism:** `lua/gauntlet/task_59.lua`, a headless-Neovim probe
  against the real diver Lua tree with two facets. The record facet is
  BEHAVIORAL: real `ai.harness.approval.request` calls on a real
  queue, then `approval.get` reads the record back and asserts the
  fields are stored verbatim. The executor facet is a recon scan:
  the approval-path modules' export tables
  (`ai.harness`, `ai.harness.approval`, `ai.harness.supervisor`,
  `ai.harness.policy`, `ai.harness.store`) plus a bounded
  source-text scan of the harness tree for `require(
  'ai.harness.approval')` consumers and executor vocabulary.
  Scenarios: `record-binds-action` (V), `binding-fields-verbatim`
  (V), `no-executor-verifies` (A), `replay-uncheckable` (A). No
  network calls, no workers spawned.
- **Success criterion:** the design's binding pass criteria — the
  driver reports `where = "recon"` (premise changed) if executor
  verification ever appears.
- **Non-goals:** inventing a test-side executor and claiming the
  product verifies. That would test the driver, not the product —
  the exact dishonesty this probe refuses.

## What happened

Fail at a HALF-absent seam — on the first and only attempt. The
record structure binds the action: a request for `fs.write` with
`argv = { 'write', '--force' }`, `paths`, and `endpoints` comes back
from `approval.get` with all four fields stored verbatim (element
counts preserved — no lossy normalization). The structure half is
real. But no executor exists to verify it: the only approval
consumer in the harness is `supervisor.tick` → `sweep_expired`
(expiry, not execution), and `policy.decide` / `approval.request`
have zero callers inside the repo — task-03's declared gap,
re-verified by this probe's consumer scan. The driver:

- `probe_reports_half_absent_seam` (V): the `record-binds-action`
  probe completes and reports `where = "seam"`, with evidence
  showing the record carries the exact tool/argv/paths/endpoints —
  the present half — and the `how` naming the absent executor.
- `binding_fields_stored_verbatim` (V): the binding-fields facet —
  argv/paths stored verbatim, so a replay check would have the full
  action shape to compare against, if an executor existed.
- `verdict_is_a_finding_not_a_probe_crash` (A): the `where` is
  neither "bootstrap" nor "lua-driver" — the probe ran to
  completion; a crashing probe must never masquerade as the seam
  finding.
- `no_executor_consumes_approval_records` (A): the executor hunt —
  the consumer scan lists only `supervisor.tick → sweep_expired`
  (expiry), and the replay facet documents both replay scenarios as
  uncheckable: with no executor, nothing can reject approval-for-X
  presented for action Y.

Fail-closed: if executor-side binding verification ever appears, the
driver reports `where = "recon"` (premise changed) instead of the
seam absence. The probe deliberately does NOT invent a test-side
executor and claim the product verifies — that would make the
replay rejections a test of the driver's own code.

Diver-owned finding (flagged, never fixed on gauntlet authority):
the missing executor is in diver's architecture. Whether the
harness should grow a real tool executor that verifies approval
bindings — or approvals stay advisory records — is a diver product
decision, banked for Matt.

Distinct from task-40 (TOCTOU): task-40 is about the binding going
STALE between approval and execution (state change in the window).
Task-59 is about the binding being UNCHECKED at execution — there is
no execution to race against.

## The fix — what changed and why

No fix — this is a documented design gap, never fixed under gauntlet
authority. The gauntlet-side work was keeping the verdict honest
about a half-present seam:

- **Changed:** `crates/phlow-gauntlet/lua/gauntlet/task_59.lua` (new)
  and `crates/phlow-gauntlet/src/tasks/task_59.rs` (new) — the probe
  asserts the record structure behaviorally (real request, real
  read-back) instead of assuming it, and scans for the executor
  rather than assuming its absence; the verdict distinguishes
  "structure present, verifier absent" from a fully-absent seam.
- **Why:** reporting the whole task as "binding absent" would be
  false — the record really does bind the action fields, and a
  future executor would inherit that structure. Reporting it as
  "binding present" would be worse — unverified bindings are not
  bindings. The half-absent verdict is the precise truth.
- **Source:** diver `lua/ai/harness/approval.lua` (record fields:
  `tool`, `argv`, `paths`, `endpoints` stored verbatim) and the
  harness tree (`supervisor.tick → sweep_expired` as the only
  approval consumer; zero executor-shaped consumers).
- **Validation agents:** the 2 validation tests
  (`probe_reports_half_absent_seam`,
  `binding_fields_stored_verbatim`) assert the seam verdict and the
  verbatim binding fields.
- **Adversarial agents:** the 2 adversarial tests
  (`verdict_is_a_finding_not_a_probe_crash`,
  `no_executor_consumes_approval_records`) rule out probe-crash
  masquerade and document the uncheckable replays.

## Full technical depth

The record facet issues real `approval.request` calls with a
synthetic run id on a real `approval.new()` queue, then reads the
record back with `approval.get` and asserts `tool`, `argv`, `paths`,
and `endpoints` round-trip exactly — including element counts (a
3-element argv and 2-element paths list survive verbatim, so no
lossy normalization weakens the binding). The executor facet
requires each approval-path module and scans its exported function
names for executor needles (`execut`, `invoke`, `perform`,
`run_tool`, `authorize`) — zero hits across all five modules —
then runs a bounded source-text scan of `lua/ai/harness` for
`require('ai.harness.approval')`: the only consumer is
`supervisor.lua`, which uses the queue for expiry
(`tick → sweep_expired`), not execution. Executor-vocabulary text
hits are classified, not counted: none consume an approval record.

What is missing for the design's scope binding: an executor that
takes an approval id/record and the actual action, compares the
record's bound tool/argv/paths/endpoints against the action being
executed, and rejects mismatches — the component that would make
the two replay scenarios rejectable. The record structure is already
exactly what such an executor would need; the verifier is the absent
half.

## Sources

- Primary: diver `lua/ai/harness/approval.lua` (record structure —
  verbatim `tool`/`argv`/`paths`/`endpoints`), `lua/ai/harness/
  supervisor.lua` (sole consumer: expiry), `lua/ai/harness/
  init.lua`, `lua/ai/harness/policy.lua`, `lua/ai/harness/store.lua`
  (zero executor-shaped exports).
- Driver: `crates/phlow-gauntlet/lua/gauntlet/task_59.lua`
  (two-facet probe).
- Driver: `crates/phlow-gauntlet/src/tasks/task_59.rs` (nvim-lua runner).
- Tests: `crates/phlow-gauntlet/tests/task_59.rs` (2V/2A).
