# Learning doc template — copy to `task-NN.md` and fill in

> Every section is required. ELI5 first, then full depth. Cite primary
> sources for every protocol/API claim. Document failures with evidence,
> not adjectives.

# task-19: council arbitration

**Kind:** rust · **Status:** pass · **Wave:** 4b · **Commits:** pending (wave 4b)

## ELI5

A council of reviewers looks at a candidate and each votes one of three
ways: Keep (ship it), Revise (send it back for more work), or Reject (drop
it). The rule is simple: whichever side gets the most votes wins — but
"most" means *strictly* more than each of the others, and ties are broken
toward caution. If Keep doesn't beat *both* rivals outright, it loses;
a tie for first place means Revise — more work, not a verdict — with one
narrow exception (below). "Correct" means the tally code implements exactly
that rule for every possible combination of votes, and no ballot trick —
duplicate voters, fake voters, empty rooms — can sneak a Keep through a tie.

## What this task attempts

- **Goal:** drive the real `phlow-council` vote tally and prove the decision
  rule is exactly right on every possible vote distribution.
- **Mechanism:** the real `phlow_council::CouncilReview::decision`
  (`crates/phlow-council/src/review.rs:80-95`); the gauntlet driver
  (`crates/phlow-gauntlet/src/tasks/task_19.rs`) builds councils and asserts
  decisions.
- **Success criterion:** 4/4 scenarios pass, including an exhaustive check
  that all 120 possible vote distributions over 1–4 reviewers resolve per
  the rule, and the CLI shows the decision with its vote tally.
- **Non-goals:** reviewer identity/authentication (the constructor validates
  shape — count, name length, duplicates — not who the reviewers are);
  multi-round deliberation.

## What happened

Passed on the first test run: 4 passed, 0 failed. (One authoring note: the
first write of `task_19.rs` was truncated by the editing tool mid-file; it
was repaired before the first compile, so no test attempt ever ran against
the truncated version.) Evidence: `strict_plurality_wins` — Keep 2/1/0
decides Keep, Reject 2/1/0 decides Reject; `every_tie_resolves_to_revise`
(with the documented reject/revise exception) — all 120 distributions over
1–4 reviewers checked; `no_tie_can_ever_yield_keep` — zero of the 120 yield
Keep without a strict Keep plurality; `tie_forcing_coalitions_and_ballot_stuffing_repelled`
— a 4-4 Keep/Revise split decides Revise, ballot-stuffing and empty/oversize
councils are rejected at construction.

## The fix — what changed and why

No fix iteration was needed — but the task *documented* a behavior that is
easy to state wrong, so this entry records the wording correction made
during review instead of code:

- **Changed:** `crates/phlow-gauntlet/src/tasks/task_19.rs` comments and
  the `every_tie_resolves_to_revise` scenario — the loose claim "every tie
  resolves to Revise" was corrected to the exact rule.
- **Why:** the real code (`review.rs:89-94`) is:
  `if keep > reject && keep > revise { Keep }`
  `else if reject > keep && reject >= revise { Reject }`
  `else { Revise }`.
  The second arm lets Reject win while *tying* Revise, as long as Reject
  strictly beats Keep. So a 1-1-2 (keep/revise/reject) split decides
  **Reject**, not Revise. Stating "every tie → Revise" would teach the wrong
  rule; the doc and scenario now say: every tie *involving Keep*, and every
  other fallthrough case, resolves to Revise — with the reject/revise-tie
  exception called out and asserted in the test evidence.
- **Source:** `crates/phlow-council/src/review.rs:80-95` (the whole
  `decision` function; the doc comment at `:76-79` states the same rule).
- **Validation agents:** `cargo test -p phlow-gauntlet --test task_19` —
  4/4 green; the exhaustive scenario prints the reject/revise-tie example
  as evidence rather than hiding it.
- **Adversarial agents:** the `tie_forcing` scenario tries 4-4, 3-3,
  2-2-2, and 2-2 splits plus ballot-stuffing (9th voter), duplicate names,
  and empty councils — Keep never emerges from a tie, and malformed
  councils never reach a tally. Found nothing that breaks the rule.
- **Citations:** decision arms — `review.rs:89-94`; constructor bounds —
  `:8` (`REVIEWERS_MAX = 8`), `:10` (`REVIEWER_CHARS_MAX = 64`),
  `:52-55` (1..=8 count check), `:60-65` (duplicate-name rejection).

## Full technical depth

`CouncilReview::new(candidate, votes)` validates shape before any tally:
1–8 reviewers (`review.rs:52`), each name ≤ 64 chars (`:60`), no duplicate
names (`:61-65`, typed `CouncilError::DuplicateReviewer`). An empty or
oversize council, or a stuffed ballot with a repeated name, never produces
a review object — the tally is unreachable, which is why the adversarial
scenario asserts on the constructor error rather than on a decision.

`decision()` (`:80-95`) tallies into `(keep, revise, reject)` and applies
three arms in order:

1. `keep > reject && keep > revise` → **Keep**. Strict plurality over
   *both* rivals. This is the only path to Keep — the exhaustive check
   proves no tie and no plurality-over-one-rival-only yields it.
2. `reject > keep && reject >= revise` → **Reject**. Strict over Keep,
   *at least tied* with Revise. This is the exception: a reject/revise tie
   where reject beats keep (e.g. keep 1, revise 2, reject 2) decides Reject.
   Rationale visible in the code's own doc comment (`:76-79`): the safe
   default is more work, not a verdict — but an explicit reject plurality
   over keep is a verdict the council did reach, and the code honors it
   rather than downgrading it to Revise.
3. `else` → **Revise**. Every tie involving Keep, every revise plurality,
   every three-way tie.

The security property the adversarial tests pin down: Keep is the
dangerous outcome (it ships the candidate), and the rule makes Keep the
*hardest* outcome to reach — it needs an outright strict plurality. A
coalition trying to force Keep through a tie (4-4, 3-3, 2-2-2) always gets
Revise. There is no path from any tie to Keep in any of the 120
distributions — that is the `no_tie_can_ever_yield_keep` proof, and it is
exhaustive, not sampled.

The CLI golden test prints the operator-facing shape: the decision plus
the `(keep, revise, reject)` tally, so a human can re-derive the verdict
from the counts without trusting the code.

## Sources

- Primary: `~/workspace/repos/phlow/crates/phlow-council/src/review.rs`
  - `:8` — `REVIEWERS_MAX = 8`
  - `:10` — `REVIEWER_CHARS_MAX = 64`
  - `:44-71` — `CouncilReview::new`: count, length, duplicate validation
  - `:76-79` — doc comment stating the decision rule
  - `:80-95` — `decision()`: the three arms, including the
    `reject >= revise` exception at `:91`
- Task code: `crates/phlow-gauntlet/src/tasks/task_19.rs`,
  `crates/phlow-gauntlet/tests/task_19.rs`
- Dependency wiring: `crates/phlow-gauntlet/Cargo.toml`
  (`phlow-council` path dependency), `Cargo.lock` (one added edge)
