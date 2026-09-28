# task-148: triager feedback ingestion

**Kind:** rust (validation/adversarial) · **Status:** pass · **Wave:** 146–150 · **Commit:** pending (wave 26)

## ELI5

The triager writes back: "I need more information." Task 148 checks that Phlow handles that note correctly — it sends the report back to the *checking desk* (validation) with the new evidence, never back to the *search team* (recon, which would redo work and risk duplicate reports). And it checks the two ways this can go wrong: a note arriving for a report that is already closed (too late — closed means closed), and a note arriving for a report number that doesn't exist (filed as "unknown", never invented).

## What this task attempts

- **Goal:** verify the `NeedsMoreInfo` return-to-validation path is exact: `Triage → NeedsMoreInfo`, queued for validation (not recon); new evidence re-enters the full `ValidationPipeline`; terminal states reject feedback; unknown ids are a typed error with nothing created implicitly.
- **Mechanism:** `src/tasks/task_148.rs` adds a driver-local `FeedbackIngester` (owns a validation queue and a recon queue) over a finding registry, driven by scripted `FakePlatform` events: `needs_more_info_routes_to_validation` (V1: state → `NeedsMoreInfo`, validation queue gets the id, recon queue stays empty); `new_evidence_revalidates_to_reportable` (V2: operator attaches new evidence, `NeedsMoreInfo → Validated`, all three default checks pass, `Validated → Reportable`); `terminal_finding_rejects_feedback` (A1: `Accepted` + `NeedsMoreInfo` → `Illegal{from: Accepted, to: NeedsMoreInfo}`, state and queues untouched); `unknown_finding_id_typed_error` (A2: event for `f999999` → `FeedbackError::UnknownFinding`, registry size unchanged, nothing queued).
- **Success criterion:** return-to-validation path exact; terminal violations rejected; unknown ids never implicitly created.
- **Non-goals:** the tracking itself (task 147); what the new evidence contains (fixture bytes).

## What happened

PASS on the exact tree, all four scenarios:

- **V1:** the `NeedsMoreInfo` verdict moved the finding `Triage → NeedsMoreInfo`; the id landed in the validation queue; the recon queue stayed empty — the cycle did not restart.
- **V2:** with the triager's requested detail attached as new evidence, the finding went `NeedsMoreInfo → Validated`, the full default pipeline ran (`in-scope`, `evidence-present`, `non-duplicate` — all pass), and the finding returned to `Reportable`.
- **A1:** feedback for the `Accepted` finding was refused with the typed illegal transition; the terminal state did not move and nothing was queued.
- **A2:** feedback for `f999999` returned `UnknownFinding{id: f999999}`; the registry kept exactly its one finding and the queues stayed empty.

## Full technical depth

The no-recon rule is the load-bearing invariant. The state machine already encodes it — `(NeedsMoreInfo, Validated)` is a legal transition and there is *no* transition from `NeedsMoreInfo` back to `Candidate` — but the ingester proves the workflow honors it operationally: the finding is queued for the validation pipeline, and the recon queue (the drain that would re-probe targets) is asserted empty. Sending a `NeedsMoreInfo` finding back to recon would re-probe the target, produce a second observation of the same bug, and risk a duplicate finding against the dedup key — the exact busywork the cycle is designed to avoid.

The V2 path is deliberately the *full* pipeline, not a delta check: new evidence could invalidate as well as confirm (a PoC that no longer reproduces, a target that left scope), so the finding re-earns `Reportable` through `in-scope` + `evidence-present` + `non-duplicate` rather than getting it back by courtesy. The `non-duplicate` check passes here because the store holds the same record (same id) — it is the *same* finding returning, not a second report of it.

`UnknownFinding` is a driver-local typed error (the scaffold has no such variant, and the scaffold is read-only for this wave): the point is the *shape* of the refusal — typed, named, and non-creating. An ingestion layer that auto-created a finding for an unknown id would let a malformed platform event conjure work out of nothing.

Primary-source note: HackerOne's `needs-more-info` is the back-and-forth state — "when further progress on the issue is blocked on response from the reporter, this is the state" (Node.js security-team workflow, citing HackerOne docs). The reporter's response (new evidence) is what unblocks it, which is exactly the V2 path.

## Sources

- `crates/phlow-gauntlet/src/bounty/types.rs` — the `(NeedsMoreInfo, Validated)` transition; terminal states have no exits
- `crates/phlow-gauntlet/src/bounty/validate.rs` — `ValidationPipeline::with_defaults` (the real pipeline re-entered in V2)
- `crates/phlow-gauntlet/src/bounty/platform.rs` — `FakePlatform` scripted verdicts
- https://github.com/jasnell/tsc/blob/HEAD/Security-Team.md — `NEEDS-MORE-INFO`: "the state of back-and-forth with reporter… blocked on response from the reporter"
- `~/workspace/gauntlet-design-tasks-131-150.md` — task-148 design (Wave 26)
