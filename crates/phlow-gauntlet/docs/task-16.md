# task-16: manifest validation

**Kind:** rust · **Status:** pass · **Wave:** 4a · **Commits:** pending (wave 4a)

## ELI5

Phlow's experiment runs on written contracts called *manifests* — TOML
files that say which test suites exist, what the resource budgets are,
what the promotion gates are, and what each task looks like. Before the
experiment trusts any of them, they are *parsed and validated*: the
parser reads the TOML the way a strict teacher reads an exam — every
answer must be present, be the right type, stay within size limits, and
no extra answers are allowed.

This task proves that validation works. It feeds the real parsers four
kinds of manifests (suite, budget, promotion, task): correct ones must
parse with all their fields intact, and broken ones — garbage TOML,
wrong types, missing keys, sneaky extra keys, oversized lists — must be
rejected with a *typed error* that names the file and the exact key that
failed. A rejection that just says "bad" without saying *where* is
almost as useless as accepting the bad file, because the operator would
have no idea what to fix.

## What this task attempts

- **Goal:** prove `phlow-experiment`'s manifest parsers accept every
  valid manifest kind and reject every malformed/adversarial manifest
  with `ExperimentError::ManifestInvalid` naming the file label and key.
- **Mechanism:** `crates/phlow-gauntlet/src/tasks/task_16.rs`
  (`run` → `drive` → `drive_valid` / `drive_malformed` /
  `drive_adversarial`) drives `phlow_experiment::{parse_suite_manifest,
  parse_budget_manifest, parse_promotion_manifest, parse_task_manifest}`
  (`crates/phlow-experiment/src/manifest.rs`) through 4 valid fixtures
  and 17 malformed/adversarial inputs. `tests/task_16.rs` probes the
  same parsers independently, including the manifests that actually ship
  in `crates/phlow-experiment/manifests/*.toml`.
- **Success criterion:** driver reports `Pass` with 4 `ok:` and 17
  `reject:` evidence lines; every rejection is `ManifestInvalid` with
  the caller's file label and a non-empty key and reason.
- **Non-goals:** the language-tier manifest parser is not driven (same
  validation primitives, covered by the crate's own tests); file I/O
  failures (`ManifestUnreadable`) are not exercised — only validation.

## What happened

Iteration 1. The driver battery was written and run: the 4 valid
manifests parsed with fields intact, and 16 of 17 malformed inputs were
rejected at the expected key. The 17th — "unknown top-level key" —
failed, but the failure was in my fixture, not the parser (next
section). Iteration 2, after the one-line fixture fix, went fully
green: `cargo test -p phlow-gauntlet --test task_16` → 5 passed
(2 validation + 2 adversarial + metadata).

Evidence excerpts from the passing driver run:

- `ok: suite manifest parses (2 suites: unit, adversarial)`
- `ok: budget manifest parses (workers_max=4, depth_max=3)`
- `ok: promotion manifest parses (critical_safety_pass_pct=100)`
- `ok: task manifest parses (id=gauntlet-probe-001, 1 check)`
- `reject: workers_max as float -> key defaults.workers_max: must be an integer`
- `reject: 17 suites (max 16) -> key suite: at most 16 items allowed`
- `reject: duplicate schema_version -> key document: TOML parse error ... duplicate key`

The independent tests additionally proved the shipped manifests
(`suites.toml` with 6 suites, `budgets.toml`, `promotion.toml`, and the
real eval manifest `evals/public/rust-cli-parse-001.toml`) still parse
against the current code — a contract-drift tripwire.

## Where it went wrong

- **Stage:** driver battery, malformed phase, case "unknown top-level
  key".
- **Symptom:** the driver failed with `wrong-rejection: rejection key
  "defaults.evil" / reason "unknown key" did not match want key
  "manifest.evil"`.
- **Evidence:** the case appended `\nevil = 1\n` to the end of the
  budget fixture — but the fixture ends with the `[defaults]` table, so
  the key landed *inside* `[defaults]` and the parser correctly reported
  `defaults.evil`. The parser was right; the fixture was wrong.
- **Root cause:** fixture construction assumed "end of file" means
  "top level" in TOML. It does not — TOML tables extend to the next
  table header or end of file, so a trailing key belongs to the last
  table. This is exactly the kind of assumption the task exists to
  punish: a test that doesn't understand the format it attacks.

## The fix — what changed and why

- **Changed:** `crates/phlow-gauntlet/src/tasks/task_16.rs` — the
  "unknown top-level key" fixture now inserts `evil = 1` immediately
  after the `schema_version = 1` line (before the `[defaults]` header)
  instead of appending it at the end of the file.
- **Commit:** <pending — same commit as the task>
- **Why:** the case's intent is "a stray key at the document root must
  be rejected as `manifest.evil`". The fix makes the fixture actually
  express that intent; the alternative (changing the expectation to
  `defaults.evil`) would have tested nested-key rejection twice and
  left top-level injection untested.
- **Source:** TOML v1.0.0 spec, "Table" section — keys belong to the
  most recent table header (toml-lang/toml).
- **Validation agents:** `cargo test -p phlow-gauntlet --test task_16`
  → 5/5 green after the fix.
- **Adversarial agents:** the battery itself is the red team — 9
  adversarial cases (type confusion, size bombs, injected keys) plus
  the independent test's 8-case type-confusion battery; all rejected
  with typed errors, none coerced, none panicking.
- **New convention:** none. The existing convention (explicit
  validation, typed errors naming file+key) held.

## Full technical depth

The parsers under test (`crates/phlow-experiment/src/manifest.rs`)
deliberately avoid derive macros: every manifest kind has a hand-written
`parse_*_manifest(toml_text, file)` that goes through `toml::Value`
with explicit control flow, mirroring `phlow-config`. The validation
primitives, bottom-up:

- `parse_root` — the document must parse as TOML and be a table;
  anything else (garbage syntax, duplicate keys) becomes
  `ManifestInvalid { file, key: "document", reason }`. Duplicate keys
  are a *parse-time* rejection by the `toml` crate, so they surface
  here rather than in a later check.
- `check_schema_version` — `schema_version` must be the integer `1`
  (`MANIFEST_SCHEMA_VERSION`); a string `"1"` is rejected with "must
  be an integer", never coerced.
- `reject_unknown` — every table, at every nesting level, rejects keys
  not on its allow-list, naming the full path (`manifest.evil`,
  `defaults.evil`, `suite[0].evil`). This is the anti-smuggling
  property: a manifest cannot carry configuration the reader doesn't
  know about.
- Typed getters — `get_u64` / `get_positive_u64` / `get_bool` /
  `get_bounded_string` / `get_string_array` enforce type, sign,
  positivity, and length bounds. TOML floats never truncate to ints;
  integers never stand in for booleans; negative budgets are rejected
  rather than wrapped; strings are bounded (`MANIFEST_NAME_CHARS_MAX`
  = 128, `MANIFEST_DESC_CHARS_MAX` = 1024).
- `parse_array` — checks `values.len() > max` *before* per-item work,
  so a 17-entry (or 17,000-entry) array is rejected without unbounded
  allocation. Bounds: `SUITES_MAX` = 16, `TASK_CHECKS_MAX` = 32,
  `REQUIRED_FILES_MAX` = `FORBIDDEN_PATHS_MAX` = 64.

Every error is `ExperimentError::ManifestInvalid { file, key, reason }`
with `file` echoing the caller-supplied label — the driver asserts the
label propagates by passing `"task-16"` and requiring it back, and the
independent tests pass a distinct label (`"task-16-adversarial"`) to
prove the propagation isn't hardcoded. The `Display` impl renders
`manifest {file}: key {key}: {reason}`, so the operator always learns
*which file* and *which key* failed.

The driver's three phases mirror the gauntlet's 50/50 discipline:
`drive_valid` (validation — the contract holds for good inputs),
`drive_malformed` (validation — the contract holds for ordinary
breakage), `drive_adversarial` (red team — type confusion, size bombs,
injected keys, unknown enum spellings). The integration tests re-probe
independently rather than re-running the driver, so a bug in the
driver's own battery cannot mask a parser regression — and they pin the
shipped manifests, so a contract drift in the real files breaks the
test instead of passing silently.

## Sources

- Primary: `crates/phlow-experiment/src/manifest.rs` (parsers, bounds,
  validation primitives), `crates/phlow-experiment/src/error.rs`
  (`ManifestInvalid` shape and `Display`), `crates/phlow-experiment/src/lib.rs`
  (crate-root re-exports), `crates/phlow-experiment/manifests/*.toml`
  and `evals/public/rust-cli-parse-001.toml` (shipped contracts).
- TOML v1.0.0 specification (toml-lang/toml): table scoping (why a
  trailing key belongs to the last table) and duplicate-key rejection.
- Secondary: `docs/00-design.md` task-16 brief (spec: "malformed TOML
  suite/budget/promotion manifests → typed rejections; valid manifests
  parse").
