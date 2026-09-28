# task-46: context pressure compaction

**Kind:** rust · **Status:** pass · **Wave:** 46–50 · **Commits:** pending (wave 46-50)

## ELI5

An agent's conversation with its tools is a long scroll of steps —
"read this file", "ran that check", "saw this output". The scroll
cannot grow forever, so the agent needs a pressure gauge: when the
scroll gets too full, pick the oldest *finished* steps and squeeze
them down into short summaries, freeing room. The rules are simple
and strict: never squeeze a step that is still running (that would
be tearing up someone's live work), never squeeze a pinned step
(the system prompt, safety context — the stuff that must survive),
and oldest finished steps go first. Under light load the engine
stays quiet and changes nothing.

phlow has exactly this gauge: `evaluate_compaction` in
`phlow-agent/src/solpi/context_compact.rs` (the SoL-Pi "Online
Context Compact" port). The driver proves the rules hold with the
real engine: under budget nothing compacts; over budget the oldest
completed unpinned steps are named first; active reasoning is never
touched; the plan's output is bounded (at most 64 candidates, a
reason of at most 512 characters).

## What this task attempts

- **Goal:** run the design's context-pressure scenarios against
  the real compaction engine: sustained fill past the pressure
  threshold, pressure during active reasoning, an adversarial size
  mix trying to pull in-flight or pinned steps into the candidate
  set, and a check that the plan's own output stays bounded.
- **Mechanism:** the `task_46.rs` driver calls the real
  `evaluate_compaction` with hand-built `CompactStep` lists and
  verifies the returned `CompactionPlan` — `compact`,
  `candidate_ids`, `reason` — with reference checks that resolve
  every candidate id back to the input list (completed AND
  unpinned, oldest first). No mocks: the engine under test is the
  production function.
- **Success criterion:** the engine's documented contract holds —
  economic + window-pressure gates decide, completed unpinned
  steps go oldest-first, in-flight and pinned steps are never
  candidates, output is bounded.
- **Non-goals:** the engine produces a *plan*; the caller performs
  the compaction. This task proves the plan is honest, not that
  any caller's real context stays capped.

## What happened

Pass — on the first and only attempt. All four cases hold against
the live engine:

- `under_budget_nothing_compacted` (V): three steps, 360/1000
  bytes of pressure — the engine stays quiet (`compact=false`).
  It still lists the two eligible candidates as advisory, but the
  pressure gate refuses to act: the plan is computed regardless,
  the decision is separate. That separation is honest, not a bug —
  the caller compacts only on `compact=true`.
- `pressure_compacts_oldest_first_sustained` (V): ten rounds of
  sustained fill; every plan stays within the engine's named
  bounds (candidates ≤ 64, reason ≤ 512 chars) no matter how much
  input arrives. Final round: 2760/1000 = 2.76 pressure, ten
  completed unpinned steps eligible — the plan compacts and names
  the oldest first.
- `critical_section_pressure_compacts_nothing` (A): 1500/1000
  pressure but every step in-flight — the plan refuses with an
  empty candidate list. The engine has no separate
  critical-section flag; the protection is the completed-only
  eligibility rule. Active reasoning is never named a candidate,
  so with zero completed steps the plan waits. This is the
  design's "compact only safe candidates or wait", implemented as
  eligibility rather than a veto — documented as such.
- `adversarial_size_mix_never_candidates_active` (A): a 600-byte
  in-flight step and a 300-byte pinned step dominate the byte
  count, trying to drag live or safety context into the set. The
  reference check resolves every candidate id back to the input:
  exactly steps 3 and 4 (the small completed ones). The big steps
  are absent.

## The fix — what changed and why

Nothing in the product changed: the seam was present and correct.
The driver is new in this wave (`src/tasks/task_46.rs`), plus
four integration tests (`tests/task_46.rs`). No production code
was touched.

## Full technical depth

The engine's decision is a pure function of policy + steps:
`window_pressure = filled_bytes / window_bytes`; compaction needs
all three of (a) pressure ≥ threshold, (b) reclaimable savings ≥
minimum bytes, (c) candidate count ≥ minimum steps. Candidates
are completed AND unpinned steps, oldest first, capped at
`candidates_max`. The reason string names which check decided and
is hard-bounded at 512 chars — the plan's own memory discipline.

Two scope caveats, stated plainly because they bound what this
pass claims:

1. The engine emits a plan; caller-side compaction is a
   different component. This task proves the plan is honest
   (oldest-first, bounded, never touches active/pinned), not
   that any caller's context is actually capped.
2. The candidate list is advisory even when `compact=false` —
   the engine always names eligible steps, and only the
   `compact` flag authorizes action. A caller that compacts on a
   false flag would be misusing the API, not exposing an engine
   bug.

Banked for Matt (product decision, NOT auto-implemented): none
from this task — the mechanism exists and behaves. If a future
task compacts caller-side state, that seam gets its own gauntlet
task.

## Sources

- Primary: `crates/phlow-agent/src/solpi/context_compact.rs`
  (`evaluate_compaction`, `CompactionPolicy`, `CompactStep`,
  `CompactionPlan`).
- Driver: `crates/phlow-gauntlet/src/tasks/task_46.rs` (drives
  the real engine; reference-checks every candidate id).
- Tests: `crates/phlow-gauntlet/tests/task_46.rs` (2V/2A).
