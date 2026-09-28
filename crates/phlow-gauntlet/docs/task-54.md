# task-54: gossip convergence

**Kind:** rust · **Status:** fail (open) · **Wave:** 51–55 · **Commits:** pending (wave 51-55)

## ELI5

When a group of machines must agree on a setting without a central
boss, they "gossip": each machine periodically picks a random peer
and swaps what it knows, like trading baseball cards. After enough
rounds, everybody holds the same cards — the group has "converged".
The guarantees that matter: convergence happens within a bounded
number of rounds, a machine cut off by a network split and then
rejoined catches up, and two machines that set the same thing
differently get resolved by a fixed tiebreak rule (not by who gossiped
loudest).

## What this task attempts

- **Goal:** find a cluster membership / config dissemination path in
  phlow and show its gossip converges on the design's numbers.
- **Mechanism:** `src/tasks/task_54.rs`, a Rust driver with four
  scenarios: (V1) a module/type-level seam search over the real phlow
  crate sources — file names and `struct`/`enum`/`trait` declarations
  for gossip/dissemination/swim/anti-entropy — zero hits; raw
  "membership"/"peer" tokens are documented as verified false
  friends (JSON-schema enum membership, scoring-pool members,
  msgpack-RPC framing peers), never as seam evidence; (V2) load the
  real `phlow_config::load_config` twice from two distinct operator
  config files and show each load keeps its own values with no
  convergence protocol; (A1) a module/type-level search for a
  membership primitive (member/cluster modules or declared types,
  join/leave) — zero hits; (A2) write conflicting values to two
  configs and show the two loads disagree forever — no tiebreak, no
  liveness argument.
- **Success criterion:** the design's gossip pass criteria (bounded
  convergence rounds; rejoin catches up; conflict tiebreak).
- **Non-goals:** inventing a gossip protocol. The design says an
  evidenced `where = "seam"` failure is the correct result when the
  design says an absent seam is valid.

## What happened

Fail, open seam — no cluster membership / config dissemination path
exists in any phlow crate. Operator config is per-process file
loading; two processes never share it. The driver:

- `no_gossip_vocabulary` (V): the gossip vocabulary scan returns
  zero hits across all phlow crates — the design's "locate; document
  if absent" step.
- `config_does_not_disseminate` (V): two real config loads keep
  their own values (`rounds = 0`, `converged = false`) — no rounds,
  no peers, no convergence protocol.
- `no_membership_primitive` (A): the membership vocabulary scan
  returns zero hits — partition/rejoin scenarios are vacuous.
- `conflicting_updates_never_converge` (A): two conflicting loads
  disagree forever with nothing to converge them — no last-writer-
  wins tiebreak, no liveness argument.

Banked product decision for Matt (NOT auto-implemented): whether
future multi-node phlow needs cluster membership and config
dissemination — and if so, what the dissemination seam looks like.
The driver flags the question and stops.

## The fix — what changed and why

No fix — this is a documented design gap, never fixed under gauntlet
authority. The gauntlet-side work was making the absence check honest:

- **Changed:** `crates/phlow-gauntlet/src/tasks/task_54.rs` (new) —
  vocabulary scans over real crate sources plus two real
  `load_config` drives with distinct files; distinguishes "seam
  absent" from "probe crashed" and "premise changed".
- **Why:** a claim of "no gossip" must be evidenced against the real
  code, and the membership-token scan is scoped to the relevant
  config/dissemination seam — the word "cluster" appears in
  unrelated inference/schema files, and those are not gossip.
- **Source:** `phlow-config` crate (`load_config`); crate manifests
  of all phlow crates for the token scans.
- **Validation agents:** the 2 validation tests
  (`no_gossip_vocabulary`, `config_does_not_disseminate`) assert the
  zero-hit scan and the independent config loads.
- **Adversarial agents:** the 2 adversarial tests
  (`no_membership_primitive`,
  `conflicting_updates_never_converge`) assert the membership scan
  is zero and conflicting loads never converge.

## Full technical depth

The driver searches every phlow crate's `src` tree (excluding the
gauntlet crate itself, whose driver docs legitimately carry the
vocabulary) at module/type level: file names containing
gossip/dissemination/swim/anti-entropy, and
`struct`/`enum`/`trait` declarations with those fragments. Zero
hits. It does the same for membership (member/cluster modules or
declared types, join/leave): zero hits. Raw token greps are
deliberately not the evidence — the words "membership" and "peer"
appear in product code with unrelated meanings, verified by reading
them: JSON-schema enum membership in `phlow-mcp/src/schema.rs`,
two-stage scoring-pool members in `phlow-inference/src/two_stage.rs`,
and msgpack-RPC framing peers in `phlow-runtime`/`phlow-tuios` ("a
hostile peer cannot make the daemon echo unbounded text" — transport
threat-modeling, not cluster membership). Those are documented as
false friends so the zero-seam finding is evidenced, not assumed.
The real config seam — `phlow_config::load_config` — reads an
operator config file into a per-process struct; driving it twice with
two distinct files yields two independent values, zero rounds, and no
peer interaction. A2 writes conflicting values to the two files and
confirms the two loads keep disagreeing indefinitely: no
last-writer-wins rule, no timestamp tiebreak, no liveness argument to
appeal to.

What is missing for the design's gossip: a membership primitive
(join/leave/failure detection), a gossip/dissemination protocol with
bounded convergence rounds, partition/rejoin handling, and a fixed
conflict tiebreak (e.g. last-writer-wins with logical timestamps).
The design gap: either multi-node phlow grows membership +
dissemination or per-process config is documented as the intended
boundary.

## Sources

- Primary: `phlow-config` crate (`load_config`); token scans over all
  phlow crates (gossip vocabulary) and over config-relevant seam
  files (membership vocabulary).
- Driver: `crates/phlow-gauntlet/src/tasks/task_54.rs` (rust driver).
- Tests: `crates/phlow-gauntlet/tests/task_54.rs` (2V/2A).
