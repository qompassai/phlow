# task-60: break-glass procedure

**Kind:** nvim-lua · **Status:** fail (open) · **Wave:** 56–60 · **Commits:** pending (wave 56-60)

## ELI5

Normally the robot must ask a human before doing anything dangerous.
But imagine a real emergency — the pipes are bursting, the approvers
are asleep, and waiting means disaster. A "break-glass" procedure is
the fire alarm with a glass cover: you CAN smash it and act without
permission, BUT the alarm rings loudly when you do, you must write
down WHY within a few minutes, the permission melts away after one
use, and if you never explain yourself, the incident gets flagged
automatically. The danger the design names: if smashing the glass is
easy and quiet, everyone does it for convenience and the permission
system is dead. This task looked for diver's fire alarm — there isn't
one. No emergency path exists at all.

## What this task attempts

- **Goal:** probe diver's real `lua/ai` tree for an
  emergency-bypass (break-glass) path — reasoned invocation,
  time-boxed single-use grant, linked post-hoc justification with an
  N-minute deadline, auto-expiry with incident flagging on missing
  justification, fresh justification required for a second use — and,
  if none exists, document the design's question: "should one exist?"
- **Mechanism:** `lua/gauntlet/task_60.lua`, a headless-Neovim recon
  probe: the approval-path modules' export tables
  (`ai.harness.approval`, `ai.harness.supervisor`, `ai.harness`)
  scanned for bypass vocabulary (`break_glass`, `breakglass`,
  `emergency`, `bypass`, `override`), plus a BOUNDED source-text scan
  of the whole `lua/ai` tree for the SPECIFIC break-glass vocabulary
  (`break_glass`, `breakglass`, `break-glass`, `emergency` — the bare
  Lua keyword `break` is deliberately not a needle, or every loop in
  the tree would hit). The generic `bypass`/`override` words are
  polysemous across the tree (config overrides, cache bypasses,
  prompt-injection patterns) and are classified, never counted as
  machinery. Scenarios: `bypass-path-scan` (V),
  `approval-exits-closed` (V), `justification-artifacts-absent` (A),
  `fail-closed-recon` (A). No network calls, no workers spawned.
- **Success criterion:** the design's break-glass pass criteria —
  the driver reports `where = "recon"` (premise changed) if bypass
  machinery ever appears.
- **Non-goals:** answering "should one exist?" — that is the banked
  product decision. Inventing an emergency path under gauntlet
  authority would create the exact backdoor risk the design warns
  about.

## What happened

Fail, open seam — on the first and only attempt. No emergency-bypass
path exists. The approval state machine exits only via
`decide(approved|denied)` or `sweep_expired` — no bypass entry
point. The one "bypass" hit in the tree is `policy.lua`'s
ANTI-bypass contract: "No adapter, provider, or MCP server may
bypass this module" — the architecture's current stance is NO
bypass, and a break-glass path would have to be reconciled with that
contract. The `ai/security` "override" hits are a control sample:
prompt-injection/unicode-bidi scanner vocabulary
(instruction-override patterns, bidi_override controls), classified
unrelated, never counted — a genuine break-glass would carry
`break_glass`/`breakglass`/`break-glass`/`emergency` vocabulary,
which has zero hits anywhere. The driver:

- `probe_reports_absent_seam` (V): the bypass-path scan completes
  and reports `where = "seam"`, with evidence citing the
  anti-bypass contract and the `how` documenting the design
  question.
- `approval_exits_are_closed` (V): the state machine's only exits
  are `decide(approved|denied)` and `sweep_expired`; no
  bypass/override/emergency entry point exists.
- `verdict_is_a_finding_not_a_probe_crash` (A): the `where` is
  neither "bootstrap" nor "lua-driver" — the probe ran to
  completion; a crashing probe must never masquerade as the seam
  finding.
- `control_sample_classified_not_counted` (A): the `ai/security`
  "override" hits are classified UNRELATED (not counted as
  machinery), and the artifacts facet documents all three required
  artifacts absent — no justification record type, no single-use
  grant primitive, no auto-expiry with incident flagging.

Fail-closed: if bypass machinery ever appears, the driver reports
`where = "recon"` (premise changed) instead of the seam absence.

The documented finding (the design's question, banked for Matt):
should a break-glass procedure exist? The current architecture's
answer is effectively "no" — `policy.lua` forbids bypass outright —
and the design's own warning stands: the abnormal path's opposite
risk is the bypass becoming routine. With no bypass at all, that
risk is currently zero. Changing the answer is a product decision
with security review, not gauntlet work.

Diver-owned finding (flagged, never fixed on gauntlet authority):
the absence — and the decision — live in the diver repo.

## The fix — what changed and why

No fix — this is a documented design gap, never fixed under gauntlet
authority. The gauntlet-side work was making the absence check honest:

- **Changed:** `crates/phlow-gauntlet/lua/gauntlet/task_60.lua` (new)
  and `crates/phlow-gauntlet/src/tasks/task_60.rs` (new) — the probe
  scans real module exports and bounded source text rather than
  asserting from memory; the `ai/security` override hits are treated
  as a control sample (matches recorded, classified unrelated), so
  the zero-machinery finding is evidenced, not assumed; and the
  verdict distinguishes "seam absent" from "probe crashed" and
  "premise changed."
- **Why:** a naive scan would false-positive on the security
  scanner's "override" vocabulary and claim bypass machinery exists.
  The classified scan makes the absence evidenced — and recording
  the anti-bypass contract as architectural evidence keeps the
  finding precise: the absence is a stance, not an oversight-shaped
  hole.
- **Source:** diver `lua/ai/` tree (bounded text scan),
  `lua/ai/harness/policy.lua` (anti-bypass contract),
  `lua/ai/harness/approval.lua` (closed exits),
  `lua/ai/security/{scanner,mcp_vet,patterns}.lua` (control sample).
- **Validation agents:** the 2 validation tests
  (`probe_reports_absent_seam`, `approval_exits_are_closed`)
  assert the seam verdict and the closed exits.
- **Adversarial agents:** the 2 adversarial tests
  (`verdict_is_a_finding_not_a_probe_crash`,
  `control_sample_classified_not_counted`) rule out probe-crash
  masquerade and document the classified control sample plus the
  absent artifacts.

## Full technical depth

The probe bootstraps the real harness from `DIVER_LUA_DIR` (scratch
rtp shim, no diver file touched), requires each approval-path
module, and scans exported function names for bypass needles — zero
hits (no `break_glass`, `breakglass`, `emergency`, `bypass`, or
`override` exports). The bounded source-text scan walks up to 256
`.lua` files under `lua/ai` (bounded bytes per file) for the text
needles: zero hits for `break_glass`/`breakglass`/`break-glass`/
`emergency` across the whole tree. The generic `bypass`/`override`
hits are classified, not counted: the `ai/security` ones are the
control sample (prompt-injection pattern strings and the
bidi-override scanner), and the rest are polysemous unrelated senses
(config overrides, cache bypasses, error-path comments) — the
evidence records their count with bounded examples. The `bypass`
needle's only approval-path hit is `policy.lua`'s anti-bypass
contract — recorded as the architectural stance, never as machinery.

What is missing for the design's break-glass: an emergency-invocation
path carrying a reason, a single-use time-boxed grant, a justification
record linked to the bypassed execution with an N-minute post-hoc
deadline, auto-expiry that flags the incident when justification never
arrives, and a fresh-justification requirement for any second use.
The design gap is the question itself: the no-bypass contract is
doing real security work today (no routine-bypass risk exists), and
any emergency path must be designed so the bypass cannot become the
quiet default.

## Sources

- Primary: diver `lua/ai/` tree (bounded source-text scan),
  `lua/ai/harness/policy.lua` ("No adapter, provider, or MCP server
  may bypass this module"), `lua/ai/harness/approval.lua` (closed
  state-machine exits), `lua/ai/security/scanner.lua`,
  `lua/ai/security/mcp_vet.lua`, `lua/ai/security/patterns.lua`
  (control sample).
- Driver: `crates/phlow-gauntlet/lua/gauntlet/task_60.lua` (recon probe).
- Driver: `crates/phlow-gauntlet/src/tasks/task_60.rs` (nvim-lua runner).
- Tests: `crates/phlow-gauntlet/tests/task_60.rs` (2V/2A).
