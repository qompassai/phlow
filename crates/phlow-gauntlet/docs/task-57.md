# task-57: escalation chains

**Kind:** nvim-lua · **Status:** fail (open) · **Wave:** 56–60 · **Commits:** pending (wave 56-60)

## ELI5

When the robot asks a human for permission and the human isn't
available, the request shouldn't just die — it should travel UP a
chain: ask the team lead, then the manager, then the boss, each with a
timer. If the whole chain runs out of people, the request must fail
in a clearly logged way (not vanish), and nobody may skip steps or
jump straight to the boss. This task looked for that chain in diver —
and found there is no chain at all: approvals live in a flat queue,
and the design's "escalation chains" concept has no seam to attach to.

## What this task attempts

- **Goal:** probe the real diver approval path for escalation-chain
  machinery — a routing table of approver levels, a next-approver
  API, delegation, chain-order configuration, a chain-exhaustion rule
  — per the design's pass criteria (ordered L1→L2 chains, exhaustion
  fails closed with a logged reason, no skipped steps).
- **Mechanism:** `lua/gauntlet/task_57.lua`, a headless-Neovim recon
  probe that requires the real approval-path modules
  (`ai.harness.approval`, `ai.harness.supervisor`, `ai.harness`)
  and the public harness API (`setup`/`run`/`cancel`/`resume`/
  `version`), scanning exported function names for routing
  vocabulary (route/delegate/escalat/approver/chain/fallback). A
  bounded source-text scan of the harness tree looks for routing
  consumers; hits are classified rather than counted — e.g.
  `adapter.lua`'s "delegates" refers to native start calls and
  `store.lua`'s fallback language concerns content-store hashing, not
  approval fallback. Scenarios: `routing-surface` (V),
  `chain-order-config` (V), `exhaustion-undefined` (A),
  `fail-closed-recon` (A). No network calls, no workers spawned.
- **Success criterion:** the design's chain pass criteria — the probe
  reports `where = "recon"` (premise changed) if any routing
  machinery ever appears.
- **Non-goals:** inventing an approver-chain design. The design says
  an evidenced `where = "seam"` failure is the correct result when
  the seam is absent.

## What happened

Fail, open seam — on the first and only attempt. The approval system
is a flat queue, not a chain. `ai.harness.approval` exports queue
operations (`request`/`decide`/`get`/`pending`/`sweep_expired`); the
public harness API is `setup`/`run`/`cancel`/`resume`/`version` —
there is no approver-level configuration, no next-approver or
delegation API, and no chain-exhaustion rule. The driver:

- `probe_reports_seam_absence` (V): the probe completes against the
  real diver Lua tree and reports `where = "seam"` — "no
  escalation-chain routing machinery."
- `chain_order_has_no_configuration_surface` (V): no
  chain-order/routing configuration exists in the harness API or the
  approval module exports — the chain has no surface to configure.
- `verdict_is_a_finding_not_a_probe_crash` (A): the `where` is
  neither "bootstrap" nor "lua-driver" — the probe ran to
  completion; a crashing probe must never masquerade as the seam
  finding.
- `exhaustion_rule_is_undefined` (A): with no chain there is no
  exhaustion rule to fail closed — the design's "exhaustion fails
  closed with a logged reason" is undefined, and the fail-closed
  facet records zero routing hits explicitly.

Fail-closed: if routing machinery ever appears, the driver reports
`where = "recon"` (premise changed) instead of the seam absence.

Diver-owned finding (flagged, never fixed on gauntlet authority):
the absence is in the diver repo's Lua modules. Whether diver needs
L1→L2 escalation chains at all — the single-operator approval path
may be the intended scope — is a diver product decision, banked for
Matt — not gauntlet work.

Distinct from task-56: the timeout path there is the only
"escalation" that exists — expiry to denied. There is no chain to
escalate ALONG; the request goes nowhere when the approver is
silent.

## The fix — what changed and why

No fix — this is a documented design gap, never fixed under gauntlet
authority. The gauntlet-side work was making the absence check honest:

- **Changed:** `crates/phlow-gauntlet/lua/gauntlet/task_57.lua` (new)
  and `crates/phlow-gauntlet/src/tasks/task_57.rs` (new) — the probe
  scans real module exports and bounded source text rather than
  asserting from memory, and classifies vocabulary hits (`adapter.lua`
  "delegates", `store.lua` fallback) as unrelated rather than
  counting them.
- **Why:** a naive text scan would false-positive on unrelated
  vocabulary and claim routing machinery exists. The classified scan
  makes the zero-routing finding evidenced, not assumed, and
  distinguishes "seam absent" from "probe crashed" and "premise
  changed."
- **Source:** diver `lua/ai/harness/approval.lua` (flat queue),
  `lua/ai/harness/init.lua` (public API surface),
  `lua/ai/harness/supervisor.lua` (no routing),
  `lua/ai/harness/adapter.lua` + `store.lua` (classified
  vocabulary hits).
- **Validation agents:** the 2 validation tests
  (`probe_reports_seam_absence`,
  `chain_order_has_no_configuration_surface`) assert the seam
  verdict and the absence of a configuration surface.
- **Adversarial agents:** the 2 adversarial tests
  (`verdict_is_a_finding_not_a_probe_crash`,
  `exhaustion_rule_is_undefined`) rule out probe-crash masquerade
  and document the undefined exhaustion rule.

## Full technical depth

The probe bootstraps the real harness from `DIVER_LUA_DIR` (scratch
rtp shim, no diver file touched), then `pcall(require, ...)` on each
approval-path module, scanning exported function names
(case-insensitive) for routing needles. The approval module's
exports are queue verbs only; the harness root's public API is
`setup`/`run`/`cancel`/`resume`/`version` — a run-lifecycle API, not
an approval-routing one. The bounded source scan (`lua/ai/harness/`)
finds zero routing consumers: the two near-miss vocabulary hits are
`adapter.lua`'s "delegates" (native start calls — adapter plumbing)
and `store.lua`'s "fallback" (content-store hash fallback — storage
plumbing), both classified in the evidence. No routing table, no
approver levels, no delegation, no exhaustion rule.

What is missing for the design's chains: an approver-level routing
table (L1→L2 order configurable), a next-approver/delegate API, a
per-level timer, and a chain-exhaustion rule that fails closed with a
logged reason. The design gap: either diver's approval path grows
escalation routing, or the single-flat-queue scope is documented as
the intended boundary — approvals expire to deny (task-56) instead of
traveling anywhere.

## Sources

- Primary: diver `lua/ai/harness/approval.lua` (flat queue exports),
  `lua/ai/harness/init.lua` (public harness API),
  `lua/ai/harness/supervisor.lua`, `lua/ai/harness/adapter.lua`,
  `lua/ai/harness/store.lua` (classified vocabulary hits).
- Driver: `crates/phlow-gauntlet/lua/gauntlet/task_57.lua` (recon probe).
- Driver: `crates/phlow-gauntlet/src/tasks/task_57.rs` (nvim-lua runner).
- Tests: `crates/phlow-gauntlet/tests/task_57.rs` (2V/2A).
