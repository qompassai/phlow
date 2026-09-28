# task-118: policy launch enforcement

**Kind:** nvim-lua · **Status:** fail (open, diver defect) · **Wave:** 116–120 · **Commits:** pending (wave 116-120)

## ELI5

The harness has a security guard (`policy.decide`) that is supposed to
check every job before it starts: "is this allowed?" The guard itself is
fine — if you ask it with no rulebook, it says "no" (fail-closed). But
nobody ever asks it. The launch code walks straight past the guard's desk
to the worker. So a "deny everything" rulebook changes nothing: jobs start
anyway. "Correct" means: every launch builds a risk report (what can this
adapter do? can it reach the network?), asks the guard, and a "no" lands
the job in `failed` with the reason written down.

## What this task attempts

- **Goal:** prove `launch` consults `policy.decide` before starting an
  adapter.
- **Mechanism:** diver's `ai.harness` — `supervisor.launch`
  (`supervisor.lua` line 182), `supervisor.new` (stores `sup.policy`),
  `policy.decide` (`policy.lua` lines 175-184) — via the driver
  `crates/phlow-gauntlet/lua/gauntlet/task_118.lua`.
- **Success criterion:** deny-all blocks the launch (run lands failed with
  the policy reason); allow lets it through; nil policy denies; the risk
  request is built from live probe caps at launch time.
- **Non-goals:** designing the policy language or vetting adapter probes.
  The lying-probe finding (an adapter can attest `remote = false` while
  doing network I/O) is banked as a trust-boundary fact for
  adapter-vetting work, not silently fixed here.

## What happened

Fail, open diver defect (Fix 3 absent) — on the first attempt. The guard
is never asked:

- `supervisor.new` stores `sup.policy`; `launch()` (lines 182-224) never
  reads it — the source scan in the driver confirms zero `policy`
  references in the launch body.
- `policy.decide` fail-closes on a nil policy ("no policy configured")
  and on malformed requests — but has zero call sites in `supervisor.lua`
  and `init.lua`.

Scenarios: `default` fails (deny-all launch proceeds to running —
`where = "fix-3-absent"`); `allow-launches` passes behaviorally but for
the wrong reason (evidence records that `decide` was never consulted);
`nil-policy` fails (`decide(nil)` denies on its own, yet the launch with
`sup.policy == nil` proceeds); `no-classification` fails and banks two
things: the source evidence (no probe call, no policy request built) and
the trust-boundary finding that Phase-2 classifies risk from probe
attestation — a lying probe (`remote = false` while `start` performs
network I/O) bypasses classification, which is adapter-vetting work, not a
launch-path fix. Extensions-smuggling and mutated-policy semantics are
banked as acceptance criteria (unverifiable today: there is no request to
smuggle through).

## The fix — what changed and why

No fix — this is a documented diver finding (Phase-2 Fix 3), and diver
findings are never fixed under gauntlet authority. The "fix" for the
gauntlet side was getting the evidence right:

- **Changed:** `crates/phlow-gauntlet/lua/gauntlet/task_118.lua` (new) —
  four scenarios, two driving real launches under explicit policies, one
  characterizing `decide(nil)`, one scanning the launch source and
  demonstrating the lying probe.
- **Why:** the defect is the missing call site, not a broken `decide`;
  the driver separates the function's own fail-closed behavior (which
  works) from the launch path (which never calls it).
- **Source:** `~/workspace/repos/diver/lua/ai/harness/supervisor.lua`
  line 182 (`launch` — no policy reference),
  `~/workspace/repos/diver/lua/ai/harness/policy.lua` lines 175-184
  (`decide` — fail-closed, zero call sites).
- **Validation agents:** the 2 validation tests
  (`deny_all_policy_does_not_block_launch`,
  `allow_policy_launch_passes_without_consultation`) assert the bypass and
  pin the accidental allow-pass.
- **Adversarial agents:** the 2 adversarial tests
  (`nil_policy_decides_deny_but_launch_proceeds`,
  `no_classification_and_lying_probe_trust_boundary`) characterize the
  fail-closed function vs. the missing call site and bank the
  trust-boundary finding.

## Full technical depth

`M.new({ registry, sink, policy, ... })` stores `policy` on the
supervisor table. `start_run` transitions created→queued, then calls
`launch(sup, run, adapter_name)`, which resolves the adapter (by name or
negotiation) and calls `chosen.start(run, sup.sink)` directly. No risk
request is constructed, no `policy.decide` call exists. The design intent
(Phase-2 spec) is: launch builds the request from probe capabilities via
pcall (`remote = true` → risk `"network"`, else `"process"`), consults
`decide` at launch time against the live policy table, and on deny
transitions the run to failed with the policy reason recorded. Today a
deny-all policy is indistinguishable from no policy at all.

The lying-probe trust boundary: Phase-2's classification input is the
adapter's own `probe()` attestation. An adapter that attests
`remote = false` while performing network I/O in `start` would be
classified `"process"` and could evade a network-denying policy. Fixing
that is adapter vetting (sandboxing, capability auditing), not a
launch-path patch — banked, out of scope.

## Sources

- Primary: `~/workspace/repos/diver/lua/ai/harness/supervisor.lua` line 182
  (`launch` — goes straight to `chosen.start`, no policy reference).
- Primary: `~/workspace/repos/diver/lua/ai/harness/policy.lua` lines 175-184
  (`decide` — fail-closed on nil policy / malformed requests).
- Primary: `~/workspace/repos/diver/lua/ai/harness/init.lua` (`M.run` —
  never references policy).
- Driver: `crates/phlow-gauntlet/lua/gauntlet/task_118.lua` (four
  scenarios; deny/nil/no-classification `where = "fix-3-absent"`).
- Tests: `crates/phlow-gauntlet/tests/task_118.rs` (2V/2A).
- Diver probed: `c84352cc850d507df477706b9166b6541ebe9e1c` (main; no
  Phase-2 branch exists).
