# task-144: report generation from a validated finding

**Kind:** rust · **Status:** pass · **Wave:** 25 · **Commits:** pending (wave 141-145)

## ELI5

This is the human-readable document — the thing a triager actually
reads. It can only be built from a finding that made it to
`Reportable`; every section (summary, impact, steps to reproduce,
evidence references, scope reference) must be present, or the whole
report is refused — never a half-filled form. The rendered text is
checked by a small lint: no trailing spaces, every code block properly
closed, headings in order. And it's injection-proof: there is no
template engine, just fixed text with the values dropped in, and the
steps section sits inside a code fence that is deliberately longer
than any fence-like text inside it — so a sneaky `{{template}}` or a
triple-backtick breakout in the steps renders as harmless literal text.
Every word is also scrubbed for secrets as it's written.

## What this task attempts

- **Goal:** prove a report generates only from a `Reportable` finding,
  with all sections present, lint-clean markdown, typed refusal on a
  missing field (no partial report), and inert rendering of template
  markers / markdown breakouts; secrets redacted at write time.
- **Mechanism:** `crates/phlow-gauntlet/src/tasks/task_144.rs`
  implements `build_report` (finding + `FindingFields` metadata →
  `Report`, redacting via `bounty::secret::redact_text`),
  `render_report` (fixed section order, lengthened fence via
  `fence_for`), and `lint_markdown` (trailing whitespace, fence
  closure with CommonMark-aware open/close tracking, heading order).
- **Success criterion:** all five sections present; secret in steps →
  `[REDACTED]`; lint violations 0 on the rendered report while the
  same lint catches 3 planted violations; missing `impact` (and
  `summary`, `steps`) → `Err(ReportError::MissingField)`; steps with
  `{{template}}` + a ```` ``` ```` breakout render literally inside a
  4-backtick fence with the lint still clean.
- **Non-goals:** the wire payload (task 145), the approval that
  authorizes the send (task 146). The lint is the driver's, not a
  general markdown linter.

## What happened

All four cases passed on the first attempt against scripted fixtures
(MOCK):

- **reportable_renders_complete (V1):** `Reportable` finding + full
  fields → all five sections present, evidence sha256 embedded, and a
  secret planted in the steps rendered as `[REDACTED]` with the raw
  value absent from the output.
- **rendered_markdown_lint_clean (V2):** 0 lint violations on the
  rendered report; the same lint caught all 3 planted violations
  (heading without space, trailing whitespace, unclosed fence) in a
  bad doc — the lint is proven live, not a rubber stamp.
- **missing_field_refused (A1):** removing `impact` → typed
  `ReportError::MissingField { field: "impact" }`; `summary` and
  `steps` refuse identically. `build_report` returns `Err` before
  constructing anything, so no partial report can be emitted (there is
  no `Report` value to render).
- **injection_rendered_inert (A2):** steps containing `{{template}}`
  and a ```` ``` ```` breakout line rendered with the literal text
  intact, the steps block fenced with 4 backticks (content's longest
  run is 3), the breakout line provably *inside* the fence (line
  indices: open < breakout < close), and the lint still clean.

Verdict: **replicates** — the document is complete, clean, refused
when incomplete, and injection-inert.

## The fix — what changed and why

No fix iterations on the gates. One design correction during writing:
the first lint draft counted fence *lines* and required an even count,
which false-positives on a lengthened fence containing a shorter
backtick run (3 fence-ish lines: open, inner, close). The lint was
rewritten to track open/close state CommonMark-style — a closing fence
must be at least as long as its opener, and a shorter run inside a
block is content. This is the honest fix: the A2 scenario *requires*
the lengthened fence to coexist with inner backtick runs, so the lint
had to understand that.

## Full technical depth

The report pipeline is `build_report` → `render_report`, and the two
stages refuse for different reasons. `build_report` enforces the
*content* contract: the finding must be `Reportable` (any other state
→ `NotReportable { state }`), and the three metadata fields
(`summary`, `impact`, `steps`) must be present in the finding's
`FindingFields` — each absent field is its own `MissingField { field }`
error naming the field. Because the function returns `Result`, a
missing field means no `Report` value ever exists; `render_report`
takes `&Report`, so a partial document is unrepresentable, not just
avoided. Evidence and scope references are derived from the finding
itself (evidence sha256, custody count, program id, scope version,
target id), so they cannot be "missing" — they are structural, not
metadata.

Every string surface passes through `redact_text` at write time
(title, summary, impact, steps) — the scaffold's write-time scrubbing,
so a secret that slips into any field comes out `[REDACTED]` in the
rendered document and never reaches the wire (task 145) raw.

`render_report` enforces the *presentation* contract with no template
engine: values are interpolated into fixed positions in a fixed section
order (`#` title, then `##` Summary / Impact / Steps to reproduce /
Evidence / Scope). `{{template}}` is inert by construction — there is
nothing to interpret it. The one active escaping decision is the steps
fence: `fence_for` finds the longest backtick run in the steps and
emits a fence one backtick longer (minimum 3, bounded at
`FENCE_LEN_MAX = 32`, beyond which it refuses with `FenceOverflow`
rather than emitting an absurd fence). This is the standard defense
(GitHub renders code blocks the same way): the content cannot close
its own block early, so a markdown breakout inside the steps stays
inside the steps.

`lint_markdown` checks three mechanical properties: no trailing
whitespace (diff and copy-paste hygiene), fence closure with
length-aware open/close tracking (a ```` ``` ```` line inside a
```` ```` ```` block is content — this is exactly the A2 shape), and
heading discipline (a space after the hashes; levels never jump more
than one at a time, so the fixed `#`/`##` section order lints clean).
The V2 case doesn't just assert "0 violations" — it plants three known
violations in a bad doc and requires the lint to catch all three,
proving the lint actually runs.

## Sources

- HackerOne / Bugcrowd report templates — the required-sections
  shape (summary, impact, steps to reproduce, supporting evidence).
  (Primary: the section contract; cited by the design doc.)
- OWASP Server-Side Template Injection thinking — the escape rule:
  with no template engine there is no SSTI surface; user content in a
  lengthened fence is inert. (Primary: the A2 design rationale.)
- CommonMark spec § fenced code blocks — a closing fence must be at
  least as long as its opening fence; info strings and shorter runs
  inside are content. (Primary: the lint's fence rule and
  `fence_for`.)
- The scaffold itself: `crates/phlow-gauntlet/src/bounty/secret.rs`
  (`redact_text` — write-time scrubbing), `types.rs` (`FindingFields`,
  the operator-extensible metadata the report fields live in).
- Design doc: `~/workspace/gauntlet-design-tasks-131-150.md`, Wave 25,
  task-144.
