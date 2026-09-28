# task-95: approval render integrity (WYSIWYG)

**Kind:** nvim-lua (validation) · **Status:** fail (open) · **Wave:** 91–95 · **Commits:** pending (wave 91-95)

## ELI5

Imagine you're asked to sign a contract, but the signing table has no contract on it — just a sticky note saying "trust us, it's fine." The design demands the opposite: the exact pages you sign must be the exact pages shown to you — what you see is what you sign. Hidden ink (terminal escape codes that can hide lines or rewrite displayed text), folded-over pages with no "continued" marker (elided hunks), and a missing plain-language summary of what actually changed must all be handled before you sign. Diver's approval queue is the sticky note: it records *that* you approved a write, with a one-line summary in the approver's own words — but it never holds the diff, never renders it, and has no renderer to harden. With no table, there's nothing to put the contract on.

## What this task attempts

- **Goal:** verify the approval UI renders the diff from the exact bytes the approval binds to — escapes neutralized with a warning, every elision visibly marked, a semantic-change summary beside the author's rationale — or document the absence with module evidence.
- **Mechanism:** `lua/gauntlet/task_95.lua` drives the REAL `ai.harness.approval` headless in four scenarios: `ui` (no `render`/`render_diff`/`show`/`display`/`format_diff` API; the record carries summary text only, no diff bytes); `diff` (zero `diff_render`/`render_diff`/`approval_ui` token hits across lua/ai); `escapes` (zero `strip_escapes`/`neutralize`/`sanitize_render` hits); `elision` (zero `elision`/`collapsed_hunk`/`hunk_marker` hits). `src/tasks/task_95.rs` runs the driver scenarios and probes the machine-readable `render-trace.json`.
- **Success criterion:** WYSIWYG render verified, or the absence documented with module evidence.
- **Non-goals:** building a render UI on gauntlet authority (Diver-owned — flagged, never fixed here).

## What happened

Honest FAIL at `where = "seam"`, first attempt — the seam is ABSENT as designed:

- **V1:** no render API — the queue is data-only. Its header says "The single approval surface renders from this queue," but no render function exists in lua/ai: the surface lives outside the lua tree.
- **V2:** no diff-render pipeline — zero hits. No module renders a proposal diff for approval.
- **A1:** no escape neutralization — zero hits. With no renderer, an ANSI-bomb in a diff is out of scope: there is no render surface to harden.
- **A2:** no elision marking — zero hits. Nothing collapses hunks, so no marker discipline can be verified — and the record carries no diff bytes for a WYSIWYG check to bind.

## Full technical depth

The driver requires the real `ai.harness.approval` through the rtp shim (read-only; Matt's diver files are never touched) and writes machine-readable traces the Rust harness probes independently. The verdict logic lives in `src/tasks/task_95.rs`: the driver's per-scenario pass/fail is about the *mechanism* (does the real module load? does the record really lack diff bytes?), while the harness probes assert the *absence* (render_api=false, record_has_diff=false, render_hits=0, escape_hits=0, elision_hits=0) and the task-level verdict reports the honest seam absence.

The token scans are bounded (500 files, 256 KiB each, word-boundary matching) over `lua/ai/**/*.lua`. The `escapes` result deserves emphasis: escape neutralization is only meaningful *given a renderer*. The honest finding is not "the renderer is vulnerable to ANSI bombs" but "there is no renderer" — the attack surface the design hardens does not exist yet.

Distinct from task-92 (approval→content-hash *binding*): this is the *render* half — even if a binding existed, there is no UI rendering the bound bytes for the approver to see.

Diver-owned finding: flagged, never fixed on gauntlet authority — whether diver should render approval diffs from the exact bytes the approval binds to, with escape neutralization, elision markers, and a semantic-change summary, is Matt's call.

## Sources

- `~/workspace/repos/diver/lua/ai/harness/approval.lua` — the real approval queue (data-only; "The single approval surface renders from this queue" — but no render function in the lua tree)
- `~/workspace/gauntlet-design-tasks-71-100.md` — task-95 design (Wave 91–95)
