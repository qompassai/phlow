# task-85: provider usage accounting integrity

**Kind:** nvim-lua (validation) · **Status:** fail (open) · **Wave:** 81–85 · **Commits:** pending (wave 81-85)

## ELI5

When a provider says "that answer cost 37 tokens", the cost ledger writes down 37. But provider reports are *untrusted input*: what if the provider says "that 'hi' cost a trillion tokens"? The ledger must cap the absurd number at a sanity bound and flag it, not bill it silently. What if the provider sends no usage at all (common with streaming)? The ledger should record an estimate, computed by a documented method, and mark it `estimated: true` so nobody mistakes it for a measurement. What if the provider later corrects itself ("actually 300, not 500")? The ledger must keep the biggest number seen and add an audit note — never silently rewrite history downward. Diver's budget ledger does none of this: it writes down whatever number you hand it. Well-formed numbers are exact, but absurd ones are absorbed silently, estimates are indistinguishable from measurements, and corrections have no audit path.

## What this task attempts

- **Goal:** verify the harness per-run cost ledger treats provider usage as untrusted input — estimated vs measured always distinguishable, absurd values capped + flagged, corrections as append-only annotations, budget enforcement consuming the conservative (max) number — or document the absence with file evidence.
- **Mechanism:** `lua/gauntlet/task_85.lua` drives the REAL `ai.harness.budget` module headless in four scenarios: `wellformed` (consume(100,'token') → snapshot.used.token == 100 exactly); `estimated` (the driver's source probe resolves `budget.consume`'s declaration from the real budget.lua via debug.getinfo('S') — exactly `(budget, kind, amount)`, no flag parameter; the snapshot carries bare numbers, no estimated flag); `absurd` (consume(b,'token',1e12) returns ok — check() only rejects negatives; no cap, no flag); `shrinking` (the module exposes exactly new/check/consume/remaining/exhausted/snapshot — no correction API, no audit; the only downward path is raw table mutation). `src/tasks/task_85.rs` runs the driver scenarios and probes the machine-readable `usage-trace.json`.
- **Success criterion:** the ingestion discipline verified, or the absence documented with file evidence.
- **Non-goals:** adding ingestion discipline on gauntlet authority (it is Diver-owned — flagged, never fixed here).

## What happened

Honest FAIL at `where = "seam"`, first attempt — the seam is REAL but does not meet the criteria:

- **V1:** well-formed usage exact — the real ledger records 100 → 100. The mechanism works; the ingestion discipline around it does not exist.
- **V2:** estimated/measured indistinguishable — `consume`'s arity is 3 (no flag slot); the snapshot shows bare numbers.
- **A1:** absurd usage absorbed silently — 1e12 tokens enters the ledger unflagged, uncapped; budget enforcement downstream eats it as-is.
- **A2:** no correction path — no correction/audit function in the module; a provider correction is possible only by raw table mutation, which keeps no max-observed and writes no audit note.

## Full technical depth

The driver resolves `ai.harness.budget` through the rtp shim (never a hardcoded diver path; Matt's diver files are never touched — the shim is read-only) and writes machine-readable traces the Rust harness probes independently. The verdict logic lives in `src/tasks/task_85.rs`: the driver's per-scenario pass/fail is about the *mechanism* (did consume behave as documented?), while the harness probes assert the *absence* (flagged=false, capped=false, correction_api=false, audit_trail=false) and the task-level verdict reports the honest seam failure.

Distinct from task-02/task-14 (budget *exhaustion policy*) and task-65 (plan *cost estimation*): this is *metrology integrity* of the cost inputs themselves. Task-02's budget enforcement consumes this ledger — it must be trustworthy input, and today it is not defended.

Diver-owned finding: flagged, never fixed on gauntlet authority — whether diver's budget ledger should gain usage-ingestion discipline (estimated flags, sanity caps, append-only corrections) is Matt's call.

## Sources

- `~/workspace/repos/diver/lua/ai/harness/budget.lua` — the real ledger under test (new/check/consume/remaining/exhausted/snapshot; check rejects only negatives and unknown kinds)
- `~/workspace/repos/diver/lua/ai/harness/types.lua` — BUDGET_KINDS (turn, tool_call, token, time_ms, byte, cost)
- `~/workspace/gauntlet-design-tasks-71-100.md` — task-85 design (Wave 81–85)
