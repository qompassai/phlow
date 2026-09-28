# task-37: confused deputy

**Kind:** nvim-lua · **Status:** fail (seam absent — no requester-chain concept in the authorization check; diver-owned finding, flagged not fixed) · **Wave:** 36–40 · **Commits:** pending (wave 36-40)

## ELI5

A security guard checks badges at a vault door. The rule: "anyone
with a level-5 badge can open the vault." Alice has level 5. Bob has
level 1 — he can only read the bulletin board. Bob asks Alice,
"could you open the vault for me?" and she does — the guard saw
Alice's badge, so it's fine. That's the *confused deputy* problem:
the guard checked *who knocked*, but never asked *who the knocking
was for*.

The design asks for the guard to check the whole story: if Alice is
opening the vault *because Bob asked her to* (a request chain:
Bob → Alice → vault), the guard should see the chain and deny it —
and the denial should *name the broken chain* (the `on-behalf-of`
field). But diver's policy checker (`ai.harness.policy`) only looks
at the immediate request: what tool, what risk, what path. The
request has no "on behalf of" field at all, no chain field, no
delegation record — and any extra provenance field a caller attaches
is silently ignored. So the deputy-caused invocation decides
byte-identically to a direct, properly-approved invocation: the guard
can't see a chain that doesn't exist, and a denial that "names the
broken chain" is impossible when there is no chain to name.

## What this task attempts

- **Goal:** drive the real policy check with three mock tools:
  privileged tool B (restricted), low-privilege tool A, and tool C —
  default: direct invocation of B succeeds; then A's invocation of B
  (invoked as part of A's execution — deputy-caused) succeeds; then
  A's call to C which calls B (two-hop laundering) succeeds — and
  each deputy-caused decision must be DENIED, naming the broken chain.
- **Mechanism:** the `task_37.lua` driver in headless Neovim against
  the REAL diver Lua tree — a real `policy.new()` with the three mock
  tools registered, real `policy.decide` calls on a real
  `AiHarnessToolRequest`. No mocks of the policy itself.
- **Success criterion:** deputy-caused invocations are denied, and
  each denial names the broken chain (`on-behalf-of`).
- **Non-goals:** fixing diver. Diver-owned findings stay flagged,
  never fixed on gauntlet authority.

## What happened

Fail at `"seam"` — on the first and only attempt, honestly. The
guard sees only the knock:

- `direct_invocation_approved` (V): direct invocation of the
  privileged tool `vault.export` decides `approval` — the real policy
  check was exercised, not a mock.
- `deputy_caused_indistinguishable` (A): the same request with a
  provenance field (`caused_by = 'notes.read'`, i.e. invoked as part
  of A's execution) decides `approval` — byte-identical to the
  direct decision. `rule_matches` consults only risk/tool/path/
  endpoint; the extra provenance field is silently ignored.
- `two_hop_laundering_invisible` (A): A→C→B laundering decides
  identically — there is no chain to inspect, so laundering depth is
  invisible too.
- `no_chain_to_name` (A): `AiHarnessToolRequest` has no
  requester-chain / delegation / on-behalf-of field at all, so the
  design's "denial names the broken chain" cannot be built — there is
  no chain to name.

A source-scan note is cited in the evidence: the supervisor stores
`opts.policy` but never calls `policy.decide` on tool execution — so
even the immediate-request check is not currently wired into the
execution path (flagged, not fixed on gauntlet authority).

## The fix — what changed and why

No product fix was made — diver-owned, flagged not fixed. The
gauntlet-side work was an honest probe:

- **Changed:** `crates/phlow-gauntlet/lua/gauntlet/task_37.lua` (new) —
  drives the real `policy.new`/`decide` with three mock tools,
  direct vs deputy-caused vs two-hop scenarios; fail-closed
  (`where = "recon"` if the seam ever changes shape).
- **Changed:** `crates/phlow-gauntlet/src/tasks/task_37.rs` (new) —
  thin `nvim-lua` shim, mirroring `task_30.rs`.
- **Why:** a chain-aware denial needs a chain. The probe proves the
  request carries no chain and the checker consults no chain — so the
  honest verdict is seam-absent, not a faked pass on the
  immediate-request approval.
- **Source:** `~/workspace/repos/diver/lua/ai/harness/policy.lua`
  (`M.new`, `M.decide`, `rule_matches` — risk/tool/path/endpoint
  only), `~/workspace/repos/diver/lua/ai/harness/types.lua`
  (`AiHarnessToolRequest` — no chain field).
- **Validation agents:** the 2 validation tests
  (`probe_reports_seam_absence`, `probe_exercises_the_real_policy_check`)
  pin the `fail`-at-`seam` verdict and prove the real policy check
  was exercised before concluding.
- **Adversarial agents:** the 2 adversarial tests
  (`verdict_is_a_finding_not_a_probe_crash`,
  `deputy_caused_invocation_is_indistinguishable`) rule out a crashing
  probe masquerading as the finding and pin the byte-identical
  decisions / two-hop invisibility evidence.

## Full technical depth

`policy.new({ tools = {...} })` builds a policy from tool
definitions; `policy.decide(policy, request)` walks the rules and
returns `approval`/`denial` via `rule_matches`, which consults only
the request's `risk`, `tool`, `path`, and `endpoint`. There is no
requester-chain field on `AiHarnessToolRequest` — the type carries
the immediate tool invocation only — and no delegation/on-behalf-of
concept anywhere in the policy module. Passing an extra field
(`caused_by`) on the request table is silently ignored by the
checker: the decision is byte-identical (same result table content)
to the direct invocation, which the probe asserts.

Two-hop laundering (A→C→B) is equally invisible for the same reason:
each hop is an immediate request; the checker never correlates
requests into a chain. The design's denial naming the broken chain
(`on-behalf-of`) requires a chain concept to exist first.

The evidence also flags a wiring gap found during recon (diver-owned,
not fixed here): the supervisor stores `opts.policy` but the source
scan found no `policy.decide` call on the tool-execution path — so
even the immediate-request check the policy module implements is not
currently consulted when tools actually run. That is a separate
finding from the task's seam absence; both are flagged, neither
fixed on gauntlet authority.

What chain-awareness would need (banked for Matt, not implemented
here): a requester-chain / on-behalf-of field on the tool request
type, `rule_matches` (or a deputy check) that inspects the full
chain, denials that name the broken chain, and the supervisor
actually consulting the policy on the execution path. Until then,
any low-privilege tool's output that triggers a privileged tool runs
with the privileged tool's authority, and the policy cannot tell the
difference.

## Sources

- Primary: `~/workspace/repos/diver/lua/ai/harness/policy.lua`
  (`M.new`, `M.decide`, `rule_matches` — immediate request only).
- Primary: `~/workspace/repos/diver/lua/ai/harness/types.lua`
  (`AiHarnessToolRequest` — no chain/delegation field).
- Driver: `crates/phlow-gauntlet/lua/gauntlet/task_37.lua` (real
  policy, headless Neovim).
- Shim: `crates/phlow-gauntlet/src/tasks/task_37.rs`.
- Tests: `crates/phlow-gauntlet/tests/task_37.rs` (2V/2A).
