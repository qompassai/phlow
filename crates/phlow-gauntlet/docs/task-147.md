# task-147: post-submission state tracking

**Kind:** rust (validation/adversarial) · **Status:** pass · **Wave:** 146–150 · **Commit:** pending (wave 26)

## ELI5

After you mail a letter, you want to know exactly where it is: received, being read, answered, or sent back. Task 147 checks that Phlow tracks a submitted bug report the same way — every step the platform reports (`triaged`, `needs more info`, `duplicate`, …) lands in the right box, a step the platform invents out of thin air is written down but never guessed at, and a step that skips the line (jumping straight to "accepted" without triage) is rejected with the evidence preserved.

## What this task attempts

- **Goal:** verify `Submitted → Triage → Accepted | Duplicate | NeedsMoreInfo | Closed` is tracked exactly; illegal transitions and unknown platform states are handled, not mapped silently.
- **Mechanism:** `src/tasks/task_147.rs` adds a driver-local `TriageTracker` that applies scripted `FakePlatform` verdict events through `FindingState::transition`: `submitted_triage_accepted` (V1: intake ack `Submitted → Triage`, then `Accepted` → ledger holds both transitions, final `Accepted`); `duplicate_links_original` (V2: `DuplicateOf("orig-123")` → `Duplicate` + link recorded); `unknown_state_never_mapped` (A1: `Unknown("triaged_by_contractor")` → logged verbatim, state stays `Triage`); `skipped_state_rejected` (A2: `Accepted` while still `Submitted` → `IllegalTransition{from: Submitted, to: Accepted}`, event quarantined, state untouched).
- **Success criterion:** legal sequences tracked exactly; unknown states never mapped; illegal transitions rejected with the event preserved.
- **Non-goals:** acting on the tracked states (that's task 148); the platform's real API (FakePlatform models mechanics only).

## What happened

PASS on the exact tree, all four scenarios:

- **V1:** intake ack moved `Submitted → Triage`; the scripted `Accepted` verdict moved `Triage → Accepted`. Event log held exactly the two transitions in order; quarantine empty.
- **V2:** `DuplicateOf("orig-123")` → state `Duplicate`, and the tracker recorded the link `(f000002, orig-123)` — the original-report-id the real state_change API carries.
- **A1:** `Unknown("triaged_by_contractor")` → event log gained a verbatim `Unknown(…)` entry; state stayed `Triage`; the tracker returned `Ok` (an unknown state is not an error, it is information) and quarantined nothing.
- **A2:** `Accepted` landing on a `Submitted` finding → `IllegalTransition`; the event was preserved verbatim in quarantine; state stayed `Submitted`.

## Full technical depth

The tracker's discipline is: verdicts go through the same `can_transition_to` table as everything else, so the platform cannot drive the finding anywhere the state machine forbids — the machine is the single authority on legal movement, and the platform is just another input. Unknown states take a separate path that cannot move the finding at all: they are recorded with the raw string on the event log, which means a platform that renames or extends its states degrades to "tracked but uninterpreted" instead of "silently misclassified". That is the never-guess rule, and it matters because misclassifying a platform state (e.g. reading a new "needs-triage" as "triaged") would corrupt every downstream decision.

The intake ack (`Submitted → Triage`) is the tracker's own bookkeeping, not a platform verdict: `TriageKind` carries verdicts (`Accepted`, `DuplicateOf`, `Closed`, `NeedsMoreInfo`, `Unknown`), and the platform acknowledging receipt is what moves a submitted finding into triage. Modeling it as an explicit step keeps the ledger honest about who said what.

Primary-source note: HackerOne's real report states are `new`, `triaged`, `needs-more-info`, `resolved`, `not-applicable`, `informative`, `duplicate`, and `spam` (per the hackerone-client `state_change` documentation and the HackerOne report-states docs). The scaffold's machine is a deliberate subset shaped to the workflow (`Accepted` ≈ resolved, `Closed` covers the closed states); `FakePlatform` models mechanics, not the real API — the design doc's labeling requirement is honored in the driver docstrings.

Distinct from task 148, which *acts* on `NeedsMoreInfo`: this task only *tracks* the platform's verdicts.

## Sources

- `crates/phlow-gauntlet/src/bounty/types.rs` — `FindingState::can_transition_to` / `transition` (the machine under test)
- `crates/phlow-gauntlet/src/bounty/platform.rs` — `FakePlatform`, `TriageEvent`, `TriageKind` (scripted double)
- https://github.com/github/hackerone-client — `state_change` states: `new, triaged, needs-more-info, resolved, not-applicable, informative, duplicate, spam`; duplicate carries the original report id
- https://github.com/acidburn0zzz/docs.hackerone.com/blob/HEAD/docs/programs/report-states.md — closed states (Resolved, Informative, Duplicate, Not Applicable, Spam) and their meanings
- `~/workspace/gauntlet-design-tasks-131-150.md` — task-147 design (Wave 26)
