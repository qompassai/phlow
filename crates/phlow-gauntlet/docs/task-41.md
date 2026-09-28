# task-41: authority attenuation

**Kind:** nvim-lua · **Status:** fail (seam absent — no per-run authority, no grant intersection at delegation; diver-owned finding, flagged not fixed) · **Wave:** 41–45 · **Commits:** pending (wave 41-45)

## ELI5

When a manager delegates to an assistant, the assistant should never
have *more* power than the manager gave them — and if the assistant
delegates further, the power can only shrink, never grow. "Authority
attenuation" is that shrinking: a parent run with tools {read}
spawns a child; the child's effective tools must be {read} ∩
whatever the child was granted — so the child asking for {write}
gets denied, and the denial names the missing grant. The same rule
applies transitively: a grandchild of the child is bounded by both
ancestors' grants.

diver has real delegation — `supervisor.spawn_child` sets
`parent_id` and calls `create`, so a parent → child → grandchild
chain exists and is linked. But run tables carry no tool grant, no
authority set, and no privilege list, and `spawn_child` performs no
grant-intersection step. Policy lives at the supervisor level
(`supervisor.new` takes a global policy), not per run. So "child
authority ⊆ parent authority" is unexpressible: there are no grants
to intersect, no denial to produce, and no grant name for the
denial to cite.

## What this task attempts

- **Goal:** drive the real delegation path — parent run with
  {read}-equivalent grant spawns a child; the child attempts
  {write}; the child spawns a grandchild attempting {write} —
  every over-grant request must be denied with the missing grant
  named, at depth 1 and transitively at depth 2.
- **Mechanism:** the `task_41.lua` driver in headless Neovim against
  the REAL diver Lua tree — real `supervisor.new` / `create` /
  `spawn_child` / `parent_id`, real `events` and `policy` modules;
  mock tools at distinct privilege levels for the grant vocabulary.
- **Success criterion:** the effective tool set of any run is the
  intersection of its ancestors' grants; the denial names the
  missing grant.
- **Non-goals:** fixing diver. Diver-owned findings stay flagged,
  never fixed on gauntlet authority.

## What happened

Fail at `"seam"` — on the first and only attempt, honestly. The
delegation path exists; the authority half does not:

- `delegation_chain_built` (V): the parent → child → grandchild
  chain builds through the real `spawn_child`, with `parent_id`
  linked at each depth. The delegation path the design targets
  exists.
- `run_tables_carry_no_authority` (V/A): the parent, child, and
  grandchild run tables carry no authority field — no tools grant,
  no grant set, no privilege list. There is nothing to intersect.
- `policy_is_supervisor_global` (A): a policy rule granting
  {fs.read} lives at the supervisor level (`supervisor.new`
  `opts.policy`) — policy is a supervisor-global collaborator, never
  a per-run field. `spawn_child` sets `spec.parent_id` and calls
  `M.create`, which validates the spec and builds the run table —
  no grant-intersection step anywhere in that path.
- `attenuation_unexpressible` (A): with no per-run grants, "child
  authority ⊆ parent authority" and transitivity to depth 2 are
  unexpressible — the denial that should name the missing grant
  cannot be produced, because there is no grant vocabulary to deny
  against.

## The fix — what changed and why

No product fix was made — diver-owned, flagged not fixed. The
gauntlet-side work was an honest probe:

- **Changed:** `crates/phlow-gauntlet/lua/gauntlet/task_41.lua` (new) —
  drives the real delegation path through three depths, inspects
  run-table keys for authority fields, and traces `spawn_child` →
  `create` for an intersection step; fail-closed (`where = "recon"`
  if run tables ever gain a tool grant or `spawn_child` intersects).
- **Changed:** `crates/phlow-gauntlet/src/tasks/task_41.rs` (new) —
  thin `nvim-lua` shim, mirroring `task_40.rs`.
- **Why:** an attenuation claim needs an authority seam. The probe
  proves delegation is real (parent_id-linked chain) but authority
  is absent (no grants, no intersection, supervisor-global policy)
  — so the honest verdict is seam-absent, not a faked pass on the
  delegation chain alone.
- **Source:** `~/workspace/repos/diver/lua/ai/harness/supervisor.lua`
  (`create`, `spawn_child`, `parent_id`; no authority fields),
  `~/workspace/repos/diver/lua/ai/harness/policy.lua`
  (supervisor-global policy).
- **Validation agents:** the 2 validation tests pin the
  `fail`-at-`seam` verdict and prove the real delegation path
  (three linked depths) was exercised before concluding.
- **Adversarial agents:** the 2 adversarial tests rule out a
  crashing probe masquerading as the finding and pin the
  no-authority-fields / no-intersection-step evidence.

## Full technical depth

`supervisor.spawn_child(spec)` is the real delegation primitive:
it stamps `spec.parent_id` and delegates to `M.create`, which
validates the spec, resolves the parent for `root_id`/children
bookkeeping, and builds the run table. The probe walks this path
three deep — parent, child, grandchild — and confirms the
`parent_id` linkage at each step. Delegation, as a mechanism, is
present and correct.

What is absent is everything the design's pass criteria need.
Dumping the run-table keys at all three depths shows no authority
carrying field: no `tools` grant, no `authority` set, no
`privileges` list. The `policy` module — the only grant vocabulary
in the harness — is a supervisor-global collaborator passed to
`supervisor.new` as `opts.policy`; it is never attached to a run.
Reading `spawn_child` → `create` shows no intersection step: no
code takes a parent grant and a child request and computes their
intersection, so "the effective tool set of any run is the
intersection of its ancestors' grants" cannot hold — there are no
sets to intersect.

Consequences: a child requesting {write} against a {read} parent
has no denial point — the request is not denied with the missing
grant named; it is simply never evaluated against any grant.
Transitivity to depth 2 is likewise unexpressible. The design's
scenarios (default {read} use allowed; adversarial {write} denied
with the grant named; grandchild {write} denied transitively) all
need the per-run authority seam, and it does not exist.

What attenuation would need (banked for Matt, not implemented
here): a per-run tool grant on the run table, an intersection step
in `spawn_child` (child grant ∩ parent effective grant, computed
from the `parent_id` chain), and a denial path that names the
missing grant. Until then, delegation in diver carries identity
(parentage) but no authority.

## Sources

- Primary: `~/workspace/repos/diver/lua/ai/harness/supervisor.lua`
  (`create`, `spawn_child`, `parent_id`; run tables carry no
  authority fields).
- Primary: `~/workspace/repos/diver/lua/ai/harness/policy.lua`
  (supervisor-global policy, never per-run).
- Driver: `crates/phlow-gauntlet/lua/gauntlet/task_41.lua` (real
  delegation path, headless Neovim).
- Shim: `crates/phlow-gauntlet/src/tasks/task_41.rs`.
- Tests: `crates/phlow-gauntlet/tests/task_41.rs` (2V/2A).
