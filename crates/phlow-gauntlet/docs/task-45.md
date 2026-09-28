# task-45: plugin dependency confusion

**Kind:** nvim-lua · **Status:** fail (defense absent at an existing seam — live shadow demonstration; diver-owned security finding, flagged not fixed) · **Wave:** 41–45 · **Commits:** pending (wave 41-45)

## ELI5

Imagine a school where every classroom door has a nameplate, and a
substitute teacher is told "go to room ACP". An attacker sticks a
fake "ACP" nameplate on a broom closet *earlier* in the hallway.
The substitute follows the hallway order, opens the first door
labeled ACP, and walks into the broom closet. "Dependency
confusion" is that hallway attack applied to plugins: a malicious
`acp` adapter placed earlier on the module search path shadows the
legitimate one, and the loader — which just asks for "the module
named acp" — loads the attacker's code.

The safe design pins every built-in to its trusted source: the
loader resolves `acp` to the pinned path, a shadower earlier on
the path never executes, and if the pinned source is missing the
load fails closed with "unresolved" instead of falling through to
the shadower.

diver's `ai.harness.registry.register_builtins` loads the six
built-in adapters with `pcall(require, 'ai.harness.adapters.' ..
name)` — a path-ordered `require` with no trusted-source pinning.
The probe demonstrates the confusion LIVE: in a child Neovim whose
search path is the clean default runtimepath with the probe-owned shadow prepended and the diver root appended,
the shadow `acp` — which writes an execution marker
and nothing else — wins the `require`, its code EXECUTES for the
trusted name `acp`, and `get_adapter('acp')` returns the shadow.
The registry's duplicate-registration rejection does not help: the
confusion happens at load time, *before* registration.

## What this task attempts

- **Goal:** verify trusted-first adapter resolution — legit `acp`
  resolves to the pinned path; a second `acp` earlier on the path
  is ignored (the shadower never executes); a missing pinned
  source fails closed with "unresolved".
- **Mechanism:** the `task_45.lua` driver in headless Neovim
  against the REAL diver Lua tree. The default scenario uses the
  real registry (legit `acp` registers; duplicate registration is
  rejected). The adversarial scenario plants a probe-owned shadow
  `ai/harness/adapters/acp.lua` (execution marker only) and runs
  the REAL `register_builtins` in a child nvim process (normal
  `--headless` mode, where runtimepath mutation is honored) with
  rtp [shadow, diver root, $VIMRUNTIME], then reports what won.
  Nothing is written outside the task scratch directory; the
  diver repo is never modified.
- **Success criterion:** resolution order is explicit and
  trusted-first; a shadow plugin's code never executes (asserted
  via the marker).
- **Non-goals:** fixing diver. Diver-owned security finding —
  flagged, never fixed on gauntlet authority.

## What happened

Fail at `"resolution"` — on the first attempt, honestly, with a
live demonstration of the confusion:

- `explicit_registry_half_exists` (V): the legit `acp` registers
  through the real `register_adapter`, `get_adapter('acp')`
  returns it, and a duplicate registration is rejected with
  "adapter already registered: acp". The explicit-registry half
  of the seam exists — first-registration-wins.
- `no_trusted_source_pinning` (V): `registry.lua` contains zero
  pin/trust verification tokens for built-in loading —
  `register_builtins` is `pcall(require,
  'ai.harness.adapters.' .. name)`, a path-ordered require.
- `shadow_wins_live` (A): the child probe's
  `register_builtins` completed (6 adapters), the shadow's
  execution marker WAS written (its code executed at require
  time), and `get_adapter('acp')` returned the SHADOW
  (`is_shadow == true`). The shadower did not merely resolve —
  it ran.
- `no_pinned_source_to_be_absent` (A): with no pinned source,
  the "absent pinned source → fail closed with unresolved"
  scenario cannot hold — ANY runtimepath entry providing the
  module name satisfies the `pcall(require)`.

## The fix — what changed and why

No product fix was made — diver-owned security finding, flagged
not fixed. (This is the wave's only task that fails at an
*existing* seam rather than an absent one, and the only one with
a live exploit demonstration; it is reported as a security
finding, not a robustness gap.) The gauntlet-side work was an
honest probe:

- **Changed:** `crates/phlow-gauntlet/lua/gauntlet/task_45.lua` (new) —
  default scenario against the real registry plus the child-nvim
  shadow demonstration; fail-closed (`where = "recon"` if
  `registry.lua` ever gains pin/trust verification or the shadow
  stops winning).
- **Changed:** `crates/phlow-gauntlet/src/tasks/task_45.rs` (new) —
  thin `nvim-lua` shim, mirroring `task_40.rs`.
- **Why:** a resolution-order claim needs the real resolver. The
  probe drives the real `register_builtins` and shows the shadow
  winning — so the honest verdict is fail-at-`resolution`, not a
  faked pass on "duplicate registration is rejected".
- **Source:** `~/workspace/repos/diver/lua/ai/harness/registry.lua`
  (`register_builtins`: path-ordered `pcall(require, ...)`; no
  pinning).
- **Validation agents:** the 2 validation tests pin the
  `fail`-at-`resolution` verdict and prove the real registry
  (legit registration, duplicate rejection, zero pin tokens) was
  exercised before concluding.
- **Adversarial agents:** the 2 adversarial tests pin the live
  shadow evidence (marker written, `get_adapter` returns the
  shadow) and rule out a probe crash masquerading as the finding.

## Full technical depth

`register_builtins(reg)` iterates the six built-in names and, for
each, calls `pcall(require, 'ai.harness.adapters.' .. name)`,
registering whatever module the require returns. `require` in
Neovim resolves through `runtimepath` order: the first rtp entry
containing `lua/ai/harness/adapters/<name>.lua` wins. There is no
step that says "the built-in `acp` must come from the diver tree"
— no path pinning, no hash check, no trusted-source list. The
probe verifies this by token-scanning `registry.lua` for
pin/trust vocabulary: zero hits.

The child demonstration makes the consequence concrete. The
driver writes a shadow `lua/ai/harness/adapters/acp.lua` whose
entire body writes an execution marker into the task work dir and
returns a minimal adapter table (`is_shadow = true`). It then
spawns a child nvim in normal `--headless --clean` mode — clean
so the ONLY harness sources are the ones the probe puts on the
path — with `rtp = [shadow_root, diver_root, $VIMRUNTIME]` set via
`:set rtp^=` / `:set rtp+=` (in-place mutations; wholesale rtp
replacement is used nowhere). The child requires the real
`ai.harness.registry`, runs the real `register_builtins`, and
reports: registry loaded, 6 adapters registered, marker file
present, `get_adapter('acp').is_shadow == true`. Every link in
that chain is the production code path — the only synthetic
element is the shadow's earlier path position, which is exactly
the attacker's capability in the threat model (any rtp entry the
operator's config pulls in: a plugin directory, a pack path, a
misordered entry).

Two subtleties the probe handles. First, the explicit registry's
duplicate rejection ("adapter already registered: acp") is real
but irrelevant: it guards double-*registration*, while the
confusion happens at *load* — by the time anything registers,
the shadow's code has already executed at require time. Second,
the "absent pinned source fails closed" scenario is vacuous here:
with no pinned source, there is no "absent" state distinct from
"any rtp entry satisfies the require" — the failure mode the
design wants ("unresolved") cannot be produced by a loader that
never pins.

What trusted-first resolution would need (diver-owned, flagged
not fixed): resolve each built-in against an explicit
trusted-source record (pinned path or content hash verified
before `require`), so a shadower earlier on the path is never
loaded and a missing pinned source fails closed with
"unresolved". Until then, the built-in names are hallway
nameplates, and hallway order decides what code runs.

## Sources

- Primary: `~/workspace/repos/diver/lua/ai/harness/registry.lua`
  (`register_builtins`: `pcall(require, 'ai.harness.adapters.'
  .. name)`; duplicate rejection in `register_adapter`; no
  trusted-source pinning).
- Driver: `crates/phlow-gauntlet/lua/gauntlet/task_45.lua` (real
  registry + child-nvim live shadow demonstration, headless
  Neovim).
- Shim: `crates/phlow-gauntlet/src/tasks/task_45.rs`.
- Tests: `crates/phlow-gauntlet/tests/task_45.rs` (2V/2A).
