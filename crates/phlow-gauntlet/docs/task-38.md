# task-38: ReDoS guard

**Kind:** rust · **Status:** fail (seam absent — no untrusted regex/pattern evaluation exists in the workspace; banked as a product decision for Matt) · **Wave:** 36–40 · **Commits:** pending (wave 36-40)

## ELI5

Some search patterns are time bombs. The pattern `(a+)+$` — "some
a's, then some a's, then the end" — looks innocent, but against the
input `aaaa…a!` (thirty a's and a bang) a naive matcher tries every
possible way to split the a's before giving up: the work doubles
with each extra a. Thirty characters can mean billions of steps. That
is ReDoS — regular-expression denial of service — and the defense is
a guard: patterns must be checked when they arrive (an allowlist, or
a complexity check), and matching must be bounded (a linear-time
engine, or a timeout that fires a clear, typed error).

The probe looked for the place this guard would attach — some code
in phlow that matches a regex against untrusted input (tool-argument
validation, log scanning, the task-36 redaction patterns). It found
nothing: no phlow crate depends on a regex crate, and a scan of
every Rust source file in the workspace finds zero regex API calls.
The time bomb has no fuse because there is no detonator: the design's
adversarial weapon — `(a+)+$` against `aaaa…a!` — has no evaluator to
feed it to. The honest verdict is seam-absent. Banked for Matt: if
phlow ever adds pattern matching on untrusted input, the design's
guard requirement applies then — linear-time engine or typed
timeout, plus an allowlist/complexity check for patterns arriving
from untrusted config.

## What this task attempts

- **Goal:** find phlow's pattern-evaluation seam and assert the ReDoS
  guard — worst-case evaluation provably bounded (timeout with a
  typed error, or a linear-time engine), and untrusted patterns never
  reaching the evaluator unchecked (allowlist or complexity check at
  load). Adversarial: the catastrophic pattern `(a+)+$` vs
  `aaaa…a!` must terminate in bounded time.
- **Mechanism:** an audit-only Rust driver against the real working
  tree — no mocks: (1) a parse of the workspace `Cargo.lock` checking
  whether any member (transitively) pulls a pattern-matching crate;
  (2) a recursive walk over every `crates/*/src/**/*.rs` scanning for
  compiled regex API/import tokens (assembled at runtime so the probe
  cannot match its own source).
- **Success criterion:** bounded worst-case evaluation with a typed
  error; untrusted patterns checked at load.
- **Non-goals:** inventing a seam. The missing pattern evaluation is a
  product decision for Matt, not an auto-fix loop.

## What happened

Fail at `"seam"` — on the first and only attempt, honestly. Both
probe prongs agree there is no seam:

- `workspace_members_pull_no_regex` (V): `Cargo.lock` shows no member
  depends on `regex`/`fancy-regex`. The only regex crates in the
  graph are `regex`/`fancy-regex` pulled by `termwiz` (via
  `ratatui-termwiz` ← `ratatui`, the TUI backend's terminal-escape
  parsing) — registry packages, not phlow code, unreachable from
  phlow's input paths.
- `no_regex_api_usage_in_sources` (V): the recursive source scan
  finds zero pattern-evaluation API usages across every
  `crates/*/src/**/*.rs` — no constructor, no match predicate, no
  captures accessor, no crate import. No application call site can
  feed untrusted input to a backtracking matcher.
- `catastrophic_pattern_has_no_evaluator` (A): the design's
  adversarial weapon has no target — `evaluators_found = 0`. No
  worst-case timing was measured because there is no engine to time.
- `untrusted_config_patterns_have_no_sink` (A): phlow-config's
  check-name validation is explicitly hand-rolled — "no regex crate
  needed" (`crates/phlow-config/src/load.rs`) — so the deliberate
  posture is even documented in-tree. Untrusted config patterns have
  no evaluation sink.

## The fix — what changed and why

No product fix was made — a regex engine was not introduced just to
guard it. The gauntlet-side work was an honest audit:

- **Changed:** `crates/phlow-gauntlet/src/tasks/task_38.rs` (new) —
  the audit driver: `Cargo.lock` graph parse + recursive source scan,
  four cases (2V/2A), typed `DriverError`, bounded scan limits
  (`LOCK_BYTES_MAX`, `SOURCE_BYTES_MAX`, `SOURCE_FILES_MAX`).
- **Why:** a ReDoS guard claim needs an evaluator to guard. The audit
  proves none exists — so the honest verdict is seam-absent, not a
  faked pass on a simulated timeout.
- **Source:** the workspace `Cargo.lock` (dependency graph),
  `crates/*/src/**/*.rs` (source scan),
  `crates/phlow-config/src/load.rs` (documented hand-rolled
  posture).
- **Validation agents:** the 2 validation tests
  (`workspace_members_pull_no_regex`,
  `no_regex_api_usage_in_sources`) pin the audit prongs and their
  zero-hit metrics.
- **Adversarial agents:** the 2 adversarial tests
  (`catastrophic_pattern_has_no_evaluator`,
  `untrusted_config_patterns_have_no_sink_and_task_fails_at_seam`)
  pin the no-evaluator metric and fold in the task-level
  `fail`-at-`seam` verdict.

## Full technical depth

The driver parses the workspace `Cargo.lock` by hand (bounded read,
8 MiB cap): for each `[[package]]` it records the name and walks the
`dependencies` lists to compute the transitive closure reachable
from each workspace member, checking membership in
`["regex", "fancy-regex"]`. Result: zero members pull a
pattern-matching crate. The regex crates that do appear in the lock
(`regex`, `fancy-regex`) are consumed only by `termwiz` — the
terminal-escape parser inside the `ratatui` TUI backend — a
registry-side dependency doing ANSI parsing, unreachable from any
phlow input path.

The source scan walks every `.rs` file under a `src` component of
each `crates/*` directory (bounded: 1 MiB per file, 50_000 files
max), matching against tokens assembled at runtime from char codes
(`Regex::new`, `.is_match(`, `fancy_regex`, `use regex`, …) so the
scan cannot match its own documentation strings. Zero hits — and the
scan covers this crate too.

Both prongs are bounded and fail-closed: an unreadable lock file, an
unscannable tree, or a scan-limit breach is a driver error, not a
pass. The four cases all pass their own assertions (the audited
facts are real); the task-level verdict is `fail` at `"seam"`
because the design's pass criteria — a bounded evaluator and
load-time pattern checks — need an evaluator to attach to, and there
is none.

What the guard would need (banked for Matt, not implemented here):
if phlow ever matches patterns against untrusted input, evaluate
them with a linear-time engine or behind a typed timeout, and
gate patterns arriving from untrusted config through an
allowlist/complexity check at load. The design's weapon
(`(a+)+$` vs `aaaa…a!`) is documented in the driver so the future
guard can be tested against it.

## Sources

- Primary: the workspace `Cargo.lock` (dependency graph, parsed at
  probe time).
- Primary: `crates/*/src/**/*.rs` (source scan, tokens assembled at
  runtime).
- Primary: `crates/phlow-config/src/load.rs` ("Hand-rolled: no regex
  crate needed").
- Driver: `crates/phlow-gauntlet/src/tasks/task_38.rs`.
- Tests: `crates/phlow-gauntlet/tests/task_38.rs` (2V/2A).
