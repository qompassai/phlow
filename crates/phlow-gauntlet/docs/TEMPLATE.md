# Learning doc template — copy to `task-NN.md` and fill in

> Every section is required. ELI5 first, then full depth. Cite primary
> sources for every protocol/API claim. Document failures with evidence,
> not adjectives.

# task-NN: <name>

**Kind:** nvim-lua | rust · **Status:** pass | fail → fixed | fail (open) ·
**Wave:** N · **Commits:** <shas>

## ELI5

Explain the orchestration idea here as if to a smart beginner: what is being
coordinated, who the parties are, and what "correct" looks like. No jargon
without a definition. 3–8 sentences.

## What this task attempts

- **Goal:** one sentence, observable.
- **Mechanism:** which real code paths are driven (crate, module, function —
  with file paths).
- **Success criterion:** the exact observable that counts as pass.
- **Non-goals:** what this task deliberately does not cover.

## What happened

Honest outcome. If it passed, say what the evidence shows (not "it works" —
quote the verdicts, counts, timings). If it failed, say so plainly and move
to the next section. Include the iteration number: first attempts are
*expected* to fail on hard tasks.

## Where it went wrong

(Delete this section if the task passed on the first attempt.)

- **Stage:** which step of the attempt failed.
- **Symptom:** the exact error, verdict, or misbehavior — quote it.
- **Evidence:** excerpts from logs/reports (bounded, relevant lines only).
- **Root cause:** the mechanism that produced the symptom, verified — not
  guessed. If the cause is still unknown, say so.

## The fix — what changed and why

(One entry per fix iteration. Matt's fix-loop rules apply to every entry.)

- **Changed:** file path + what was edited (or created).
- **Commit:** sha.
- **Why:** the rationale — why this change addresses the root cause and why
  the alternatives were rejected. This is the part that teaches.
- **Source:** the primary source that makes this choice correct (upstream
  docs, protocol spec, paper, or guide section). Minimum bar: every fix
  cites one.
- **Validation agents:** who confirmed the fix (what they ran, what passed,
  what regressions they checked for).
- **Adversarial agents:** who red-teamed the fix (what attacks/edge cases
  they tried, what they found — including "found nothing" with the list of
  what was attempted).
- **New convention (if any):** only when tests + gates prove a genuinely
  better pattern. Document the convention, its rationale, and its evidence.
  Never establish a convention by assertion.
- **Citations:** primary sources for any API/protocol/behavior claim the fix
  relies on.

## Full technical depth

The mechanism, end to end: data flow, protocol frames, state machines,
budgets, failure modes, and the exact code paths exercised. Write the
explanation you wish you had before attempting the task. Reference the
ELI5 section's terms and deepen them — don't introduce a second vocabulary.

## Sources

- Primary sources first (protocol specs, upstream docs, source files with
  paths and line numbers).
- Secondary sources clearly marked as such.
