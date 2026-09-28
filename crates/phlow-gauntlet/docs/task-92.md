# task-92: proposal scope binding and drift detection

**Kind:** nvim-lua (validation) · **Status:** fail (open) · **Wave:** 91–95 · **Commits:** pending (wave 91-95)

## ELI5

Imagine you approve a blueprint for a house — but your approval is stapled to *exactly* the blueprint you reviewed, down to the paper fibers. If anyone swaps a page, rebases the plan onto a new lot, or regenerates the drawing before construction starts, the staple breaks and the approval is void — you'd have to review the new pages and re-approve. Diver's approval queue has no staples: it records *what kind of work* was approved (write a file, run a command) and *who* approved it, but never fingerprints the exact proposal bytes. A proposal could drift between approval and apply — rebased, edited, regenerated — and nothing in the lua layer would notice.

## What this task attempts

- **Goal:** verify an approval binds to hash(base revision + diff bytes), drift between approval and apply is detected and rejected with `proposal_drift`, and a re-review recovery path exists — or document the absence with module evidence.
- **Mechanism:** `lua/gauntlet/task_92.lua` drives the REAL `ai.harness` modules headless in four scenarios: `binding` (the approval queue exposes no `bind_approval`/`proposal_hash`/`content_hash`/`candidate_digest` API; the record carries id/tool/risk/summary/argv/paths, no hash field; `store.content_hash` is artifact dedup, unreachable from approvals); `scope` (`ai.harness.policy` binds approvals to action scope via rule decisions — the task-59 mechanism — never to reviewed bytes); `drift` (no `ai.harness.drift`/`ai.self_improve`/`ai.approval` modules; zero `drift` token hits across lua/ai); `recovery` (zero `re_approve`/`reapprove`/`proposal_drift` token hits). `src/tasks/task_92.rs` runs the driver scenarios and probes the machine-readable `binding-trace.json`.
- **Success criterion:** content-hash binding verified with drift detection, or the absence documented with module evidence.
- **Non-goals:** adding binding/drift detection to diver on gauntlet authority (Diver-owned — flagged, never fixed here).

## What happened

Honest FAIL at `where = "seam"`, first attempt — the seam is ABSENT as designed:

- **V1:** no content-hash binding — the approval queue is data-only; no binding API, no hash field on the record. The store's `content_hash` hashes artifact bytes for dedup; the approval queue never references it (the red herring, confirmed unreachable).
- **V2:** scope, not hash — the policy binds approvals to action scope (allow/deny/approval rule decisions — task-59's tool-use mechanism). It binds *what the tool may do*, never *which bytes were reviewed*. Distinct from what task-92 demands.
- **A1:** no drift detection — no drift module, zero `drift` token hits. A rebase or concurrent edit between approval and apply would not be detected lua-side.
- **A2:** no recovery path — zero hits for re-approve vocabulary or `proposal_drift`. Fail-closed would be fail-stuck: a drifted proposal has no lua-side path back to approval.

## Full technical depth

The driver requires the real `ai.harness.approval`, `ai.harness.policy`, and `ai.harness.store` through the rtp shim (read-only; Matt's diver files are never touched) and writes machine-readable traces the Rust harness probes independently. The verdict logic lives in `src/tasks/task_92.rs`: the driver's per-scenario pass/fail is about the *mechanism* (do the real modules load? does the queue really lack the API?), while the harness probes assert the *absence* (binding_api=false, record_has_hash=false, scope_binding=true with hash_binding=false, drift_hits=0, recovery_hits=0) and the task-level verdict reports the honest seam absence.

The token scans are bounded (500 files, 256 KiB each, word-boundary matching) over `lua/ai/**/*.lua` — the same harness-probe discipline as prior waves. The `drift` scan returning zero hits is the substantive result: not even the vocabulary exists, let alone the mechanism.

Distinct from task-59 (tool-use approval *scope* — which exists and is verified) and task-91 (the rust half — the approval *has* a digest but the proposal has nothing to match): this is the lua half — the approval has no digest binding at all, and no drift detection across time.

Diver-owned finding: flagged, never fixed on gauntlet authority — whether diver should bind approvals to content hashes with drift detection and a re-review path is Matt's call.

## Sources

- `~/workspace/repos/diver/lua/ai/harness/approval.lua` — the real approval queue (data-only; no binding API, no hash field)
- `~/workspace/repos/diver/lua/ai/harness/policy.lua` — action-scope binding via `decide` (task-59's mechanism, distinct from content-hash binding)
- `~/workspace/repos/diver/lua/ai/harness/store.lua` — `content_hash` for artifact dedup (the red herring; unreferenced by the approval queue)
- `~/workspace/gauntlet-design-tasks-71-100.md` — task-92 design (Wave 91–95)
