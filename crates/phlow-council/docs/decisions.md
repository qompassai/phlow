# phlow-council: placement and adaptation decisions

## Why this crate exists

Long autonomous programs need a shape for "try, check, decide" that is
auditable after the fact. `phlow-council` is that shape: a task contract
written up front, one candidate implemented at a time, evidence retained
per candidate, and a keep/revise/reject council decision per iteration.
It is deliberately domain-free — it knows nothing about GPUs, tiles, or
kernels — so any lane can reuse the workflow.

## What was adapted (from NVlabs kda, NOASSERTION — methodology only)

- Task contract before work begins (goal, constraints, acceptance).
- Workspace inspection → draft plan → implement one candidate.
- Validate correctness; measure/profile when applicable.
- Retain evidence per candidate; keep candidate lineage (parent ids).
- Council review per iteration with keep / revise / reject outcomes.

Only the workflow methodology was adapted. **No kda material was
copied**: no documentation prose, no prompts, no skill text, no agent
definitions, no file layouts. Everything here — code, doc comments,
tests, and this file — is original.

## Deliberately excluded

- **Agent orchestration.** kda is a multi-agent system; this crate is a
  state machine. It does not spawn agents, run tools, or schedule work —
  it records what happened and what was decided.
- **Prompt engineering.** There are no prompts here. Reviewer "votes" are
  data supplied by the caller, not model output.
- **Automatic promotion.** A keep vote promotes only a *verified*
  candidate; the workflow never promotes on votes alone and never skips
  the evidence step.
- **Persistence.** The workflow lives in memory. Snapshotting to disk is
  a caller concern.

## License basis

kda's repository carries NOASSERTION. Accordingly, nothing was copied
from it — only the high-level research → implement → verify/profile →
iterate methodology was adapted, expressed here in original code and
prose.
