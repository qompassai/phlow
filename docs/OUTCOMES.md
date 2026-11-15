# Outcome records — one row per run (PH-1, PH-K3)

Status: landed 2026-11-10. Store: `phlow-self-improve::outcome`;
runtime wiring: `phlow-runtime::Runtime::set_outcome_db`, set by the
`phlow-cli` dispatch for every production surface (CLI, TUI, MCP
server).

## The rule

Every finished run appends **exactly one** outcome row, written by
the runtime — never by the model (no model-visible tool can reach
the store). A run that errors still leaves its row; a run reaped by
the run contract is closed with status `abandoned` and also has a
row. A second row for the same `run_id` is a typed refusal
(`OutcomeDuplicate`), and the table is append-only at the SQLite
level: `UPDATE`/`DELETE` triggers abort on any connection.

The suite test is `crates/phlow-runtime/tests/outcome_record.rs`
(completed run, backend-failure run, two runs → two rows) plus the
store-level tests in `phlow-self-improve::outcome` (including the
killed/`abandoned` shape). A run with no row fails those tests by
construction.

## The row

| Field | Source |
|---|---|
| `run_id` | Minted by the runtime: `run-<pid>-<unix-nanos>-<counter>` |
| `task_class` | Caller classification; the runtime passes `flow_run` until intake classes land (PH-6) |
| `plan` | Planner role's summary from the run report |
| `checks_json` | `[{name, status}]` from the report's host-verification checks |
| `reviewer_verdict` | Reviewer role's `approved` flag (`approved`/`not_approved`), else its status |
| `cycles`, `tool_calls`, `model_calls` | The report's own counters |
| `context_chars` | `NULL` when unmeasured — never invented |
| `wall_ms` | Runtime-measured wall time |
| `model_ids_json` | Distinct role models, first-use order |
| `status` | Final report status |
| `verification_strength` | See below |
| `origin_lane`, `content_label` | Bloch labels, from birth |
| `evidence` | What backs the row (the host verification gate) |
| `created_at` | Store timestamp |

## Verification strength (PH-K3, INDEX §2.3)

`self_reported` < `tool_checked` < `independently_cross_checked` <
`owner_judged`. An automated run carries at most `tool_checked` —
set exactly when the report's host verification says `verified` —
else `self_reported`. An outcome with no (or an unknown) strength
label is refused at the type level: the field is an enum, and
`VerificationStrength::parse` rejects anything outside the ladder.
Stronger labels are attached by later, separate record-keeping —
never by rewriting this row (it is append-only).

## Lane labels

Rows are born labelled. `origin_lane` defaults to `phlow` for
runtime-written rows (empty → `unknown` at the store). `content_label`
defaults to **`nsfw`, fail-closed**: a phlow run over an NSFW-lane
workspace must never surface as SFW by omission, and only an explicit
classification by the caller may set `sfw`. NSFW-lane outcomes never
reach a qompassai surface (bloch intake, BL-1/PH-7, enforces the
audience side).

## Where the store lives

The `run_outcomes` table lives in the **same SQLite file** as the
feedback store — one store file, two tables; the feedback schema is
untouched. Production path: `feedback.db` beside the operator config
(`$XDG_CONFIG_HOME/phlow/feedback.db` under the default layout).
With no config file there is no phlow home to record into and
recording stays off — a deliberate absence: every *configured*
production run records. Library embedders opt in with
`set_outcome_db`; reports from an unconfigured runtime carry no
outcome keys, byte-identical to pre-PH-1 behaviour.

Recording failure never rewrites a run's verdict: the report gains
`outcome_error` with the store's message, and a successful record
adds `outcome_run_id`.

## Export to bloch (PH-13)

`phlow-export` maps outcome rows into bloch's BL-1 intake objects
(bloch 1.0.0, `bloch::outcome`): `server` is `phlow`, `outcome_id`
the run id, `verification` the row's own strength label, `cost`
in **seconds** (bloch's unit — the row's `wall_ms` divided by
1,000; tokens are not recorded per run and stay 0 rather than
being converted from characters), `skills` from PH-8, and
`produced` empty (phlow records no artifact hashes — bloch's
field stays honestly empty).

Two joins make the record lawful. The **lane** comes from the
run's PH-7 capture — derived there from the flowguard trajectory,
never asserted by the export's caller and never read from the
row's placeholder labels; a run with no capture, or a capture
whose lane is outside bloch's vocabulary, is refused, never
defaulted. **`asked`** is the run's task text, joined from its
run report by `outcome_run_id` (the row stores the plan, not the
task); a run no report joins is refused, never fabricated.
`result` is the capture's BL-3 word (`ok`/`failed`/`abandoned`),
refused when outside the vocabulary. An `nsfw`-labelled record
is never exported into an `sfw`-scoped intake — the scope is a
required operator argument, with no default.

Bloch's store appends without dedupe, so dedupe is phlow's: an
append-only `outcome_exports` ledger in the same store file,
plus a scan of the target file itself before appending, so a
lost ledger cannot re-export what the target already holds.
Re-running `phlow-export` reports already-exported runs and
appends nothing twice. Acceptance is proven against bloch's own
gate in `phlow-self-improve`'s `bloch_link` suite, which links
the published `blochs` 1.0.0 crate as a dev-dependency.
