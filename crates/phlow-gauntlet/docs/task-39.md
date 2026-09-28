# task-39: recursive payload bomb

**Kind:** rust · **Status:** fail (seam partially present — depth and size are bounded with typed errors; the expansion-ratio bound the design conjunctively demands is absent; banked as a product decision for Matt) · **Wave:** 36–40 · **Commits:** pending (wave 36-40)

## ELI5

A "billion laughs" attack is a tiny message that explodes when you
read it: it defines "laugh1" as ten "ha"s, "laugh2" as ten laugh1s,
and so on — a few hundred bytes of input becomes gigabytes of memory
when the parser expands it. The defense is two bounds working
together: a **depth bound** (how deeply nested the message can be)
and an **expansion-ratio bound** (the parsed result may never be
more than N times bigger than the input).

phlow's JSON parser (`phlow_json::parse_limited`) has the depth half:
an explicit depth bound of 64 (typed error `DepthExceeded`), enforced
after parsing with an explicit stack — no recursion ever runs over
attacker-controlled depth — plus serde_json's own recursion limit of
128 at parse time, and a 1 MiB total-size cap with its own typed
error (`InputTooLarge`). But the expansion-ratio half does not
exist: no ratio constant, no ratio-specific error variant, no
measurement anywhere. And the attack itself has no seam — phlow has
no recursive-expansion decoder (no entity expansion, no archive
extraction), so a billion-laughs payload has nothing to explode
*through*; a large flat payload parses to at most one node per byte
(no self-amplification — JSON cannot expand beyond O(input)). So
the missing bound is defense-in-depth, not a demonstrated
vulnerability — but the design demands *both* bounds conjunctively,
and the ratio half is absent, so the honest verdict is fail at the
seam.

## What this task attempts

- **Goal:** drive the real `phlow_json::parse_limited` through the
  design's scenarios — default: nested payload parses; adversarial:
  deep nesting rejected with a typed depth error; adversarial: the
  expansion ratio is bounded with its own typed error (billion-laughs
  style); validation: flat 1 MiB cap rejects oversize input with a
  typed error.
- **Mechanism:** a Rust driver against the REAL `phlow_json` crate
  (path dependency) — real `parse_limited` calls with real payloads,
  and an embedded-source scan of phlow-json's own sources proving no
  ratio bound exists (the `JsonError` enum is exhaustively matched, so
  a future ratio variant would force the driver to be revisited).
- **Success criterion:** both bounds exist with typed errors — depth
  AND expansion-ratio.
- **Non-goals:** inventing a ratio bound. A missing defense-in-depth
  bound is a product decision for Matt, not an auto-fix loop.

## What happened

Fail at `"seam"` — on the first and only attempt, honestly. The
depth half is real; the ratio half is absent:

- `nested_payload_parses` (V): a nested payload parses —
  `parse_limited` is a real, working parser, and parsed nodes cannot
  exceed input bytes.
- `deep_nesting_rejected_with_typed_error` (V): a 10_000-deep payload
  is rejected with the typed `DepthExceeded` error — the explicit
  bound (`JSON_DEPTH_MAX = 64`) is enforced after parse with an
  explicit stack, backed by serde_json's own recursion limit (128)
  at parse time.
- `expansion_ratio_bound_absent` (A): no expansion-ratio bound exists
  anywhere — no constant, no `JsonError` variant (verified by
  exhaustive match over every variant and a scan of the embedded
  phlow-json sources). A large flat payload shows parsed nodes <=
  input bytes: no self-amplification, because JSON cannot expand
  beyond O(input).
- `flat_cap_is_the_only_total_bound` (A): the flat cap (1 MiB) is the
  only total-size bound — an oversize payload is rejected with the
  typed `InputTooLarge` error. With no expansion primitive in phlow's
  parse paths it bounds total memory — but the design's required
  expansion-ratio bound with its own typed error is still absent.

The evidence also notes the attack surface is nil: source scans found
no recursive-expansion decoder in phlow's parse paths (no entity
expansion, no archive extraction — no zip/flate2/tar in the graph, no
recursive template expansion of untrusted input) — the billion-laughs
attack has no seam. The missing bound is a defense-in-depth product
decision, not a demonstrated vulnerability — but the conjunctive
criteria are unmet.

## The fix — what changed and why

No product fix was made — a ratio bound was not invented on gauntlet
authority. The gauntlet-side work was an honest probe:

- **Changed:** `crates/phlow-gauntlet/src/tasks/task_39.rs` (new) —
  drives the real `phlow_json::parse_limited` through all four
  scenarios, embeds phlow-json's sources, exhaustively matches
  `JsonError` (a future ratio variant forces driver review), typed
  `DriverError`, bounded payload sizes.
- **Why:** a ratio-bound claim needs a ratio bound. The probe proves
  the depth half is real AND the ratio half is absent — so the honest
  verdict is seam-absent (partially), not a faked pass on the depth
  bound alone.
- **Source:** `crates/phlow-json/src/lib.rs` (`parse_limited`,
  `JSON_DEPTH_MAX`, `JSON_INPUT_BYTES_MAX`, `JsonError`).
- **Validation agents:** the 2 validation tests
  (`nested_payload_parses`,
  `deep_nesting_rejected_with_typed_error`) pin the real parser and
  the real typed depth bound.
- **Adversarial agents:** the 2 adversarial tests
  (`expansion_ratio_bound_absent`,
  `flat_cap_is_the_only_total_bound_and_task_fails_at_seam`) pin the
  absent ratio metric (with the no-self-amplification measurement)
  and fold in the task-level `fail`-at-`seam` verdict.

## Full technical depth

`parse_limited` checks the input length against
`JSON_INPUT_BYTES_MAX` (1 MiB) *before* parsing, returning typed
`JsonError::InputTooLarge`. Parsing itself goes through serde_json,
whose own recursion limit (~128) rejects very deep input at parse
time — before any stack could grow over attacker-controlled depth.
Then the driver-verified depth check walks the parsed value with an
explicit stack and rejects anything deeper than `JSON_DEPTH_MAX`
(64) with typed `JsonError::DepthExceeded`. Two bounds, both typed,
both real.

What does not exist: any measurement of expansion ratio
(parsed-size / input-size), any ratio constant, any ratio-specific
`JsonError` variant. The task_39 driver proves the absence
mechanically: it embeds `crates/phlow-json/src/lib.rs` at compile
time and scans it for ratio-bound tokens (`ratio`, `expansion`,
`bomb`, `laughs`), and it exhaustively matches every `JsonError`
variant — adding a `RatioExceeded` variant later would be a
compile error in the driver until the driver is updated to account
for it. This is the fail-closed design: the probe cannot silently go
stale.

The billion-laughs attack needs a recursive-expansion primitive —
XML entity expansion, archive extraction with nested archives,
recursive template inclusion. The recon scans found none in phlow's
parse paths: no `zip`/`flate2`/`tar` in the dependency graph, no
entity-expansion code, no recursive template expansion of untrusted
input. So the ratio bound is defense-in-depth against a payload
shape that currently has no decoder — worth deciding, not worth
faking.

What the ratio bound would need (banked for Matt, not implemented
here): a defined expansion-ratio limit (e.g. parsed nodes or bytes
may not exceed N× input bytes), measured during or after parsing,
with its own typed `JsonError` variant. Until then, depth and total
size are bounded and the missing conjunct is documented here.

## Sources

- Primary: `crates/phlow-json/src/lib.rs` (`parse_limited`,
  `JSON_DEPTH_MAX = 64`, `JSON_INPUT_BYTES_MAX = 1 MiB`,
  `JsonError::DepthExceeded` / `InputTooLarge`).
- Driver: `crates/phlow-gauntlet/src/tasks/task_39.rs` (real
  `phlow_json`, embedded-source scan, exhaustive `JsonError`
  match).
- Tests: `crates/phlow-gauntlet/tests/task_39.rs` (2V/2A).
