# task-152: unknown fields ignored, never denied

**Kind:** rust (validation) · **Status:** pass → fixed · **Wave:** 25 · **Commit:** pending (wave 25 commit)

## ELI5

Programs evolve: your peer adds a `"future_feature"` field to its messages before you do. If your decoder rejects any field it doesn't recognize, the whole conversation breaks the moment either side upgrades — that's what serde's `deny_unknown_fields` does, and the Ghostex rule bans it on wire types. This task proves the decoder is permissive: an envelope with 3 known fields and 5 unknown ones parses fine, the known fields are exact, the unknowns are dropped (and counted in a debug metric so you can see them), and a static scan of the real `phlow-mcp` and `phlow-runtime` sources finds zero `deny_unknown_fields` attributes.

## What this task attempts

- **Goal:** unknown fields on wire structs are ignored (never denied); the static scan finds zero `deny_unknown_fields` on any type crossing the socket in `phlow-mcp`/`phlow-runtime`.
- **Mechanism:** `crates/phlow-gauntlet/src/wire.rs` — `parse_envelope`, `Envelope.unknown_fields`; driver `crates/phlow-gauntlet/src/tasks/task_152.rs` (`rs_files_under` iterative walker + text scan).
- **Success criterion:** the 3-known + 5-unknown envelope parses with known fields exact and `unknown_fields == 5`; the scan reports 0 hits over both crates' sources; license gate passes.
- **Non-goals:** unknown enum *variants* (task 151); whether unknown fields are *preserved* (they are dropped by design — the metric counts them).

## What happened

Passed after one fix. `cargo test -p phlow-gauntlet --test task_152` → 3 passed, 0 failed. The augmented envelope parsed with `unknown_fields == 5`, the known fields exact, and none of the five `future_*` fields leaked into the canonical form. The static scan covered the real `phlow-mcp` and `phlow-runtime` sources and reported zero hits.

## Where it went wrong

- **Stage:** V2 static scan, first gate run.
- **Symptom:** the scan case failed: `N deny_unknown_fields hits, want 0`, where the hits were the gauntlet's own comments and string literals.
- **Evidence:** the driver scanned three roots — `../phlow-mcp`, `../phlow-runtime`, and the gauntlet's own `src/` — and the plain-text match `deny_unknown_fields` fired on the driver's own doc comments (`"the count must be zero"` context) and on `text.matches("deny_unknown_fields")` itself.
- **Root cause:** scope error. The spec's V2 is "all wire structs in phlow-mcp/phlow-runtime scanned"; the gauntlet's `src/` was never in scope, and a substring text scan cannot distinguish a serde attribute from prose mentioning it. Verified by reading the spec (`~/workspace/gauntlet-design-tasks-151-200.md`, task-152 V2) and by locating the hits in `src/wire.rs` / `src/tasks/task_152.rs` comments.

## The fix — what changed and why

- **Changed:** `crates/phlow-gauntlet/src/tasks/task_152.rs` — removed `manifest.join("src")` from the scan roots; doc comment updated to say the scan covers exactly `phlow-mcp` and `phlow-runtime`. `tests/task_152.rs` — corrected to the driver's real case name (`static_scan_no_deny`) and real metric keys (`unknown_fields`, `deny_unknown_fields_hits`, `files_scanned`); the test had assumed a `resend_reparsed` metric the driver never emitted.
- **Commit:** pending (wave 25 commit).
- **Why:** the spec scopes the scan to the two crates whose types cross the socket; scanning the gauntlet's own sources tests nothing about forward compatibility and self-reports. Matching the test to the driver's actual contract (rather than inventing metrics) keeps the integration test honest.
- **Source:** wave-25 design doc task-152 ("V2: all wire structs in phlow-mcp/phlow-runtime scanned → zero `deny_unknown_fields` attributes"); Ghostex rule "never `deny_unknown_fields`" (adaptation map); serde default struct behavior (unknown fields ignored unless denied).
- **Validation agents:** `cargo test -p phlow-gauntlet --test task_152` → 3/3 pass; the scan reports `files_scanned > 0` and `deny_unknown_fields_hits == 0` (fails closed on an empty crate).
- **Adversarial agents:** considered a serde attribute written with whitespace (`deny_unknown_fields` with odd spacing) — the attribute name is a single ident, so substring matching is exact for real attributes; prose mentions only exist in the gauntlet's own tree, which is out of scope.

## Full technical depth

`parse_envelope` decodes into `serde_json::Value` first and then projects the known fields (`version`, `kind`, `id`, `body`) into `Envelope`, so unknown fields are dropped structurally — there is no struct with `deny_unknown_fields` anywhere on the path, and none *can* be, because the decoder never materializes a closed struct from wire bytes. The dropped count is computed as (object key count − known keys present) and stored in `Envelope.unknown_fields` as a debug metric. The canonical form (`to_canonical_json`) is built from the typed `Envelope`, so dropped fields provably leave no trace — the test asserts none of the five `future_*` names appear in it. The static scan is a deliberate plain-text sweep rather than an AST query: the banned item is a single attribute ident, and the scan fails closed (zero files found = error, not pass). Iterative directory walk, no recursion, symlinked dirs not descended.

## Sources

- Ghostex rule "never `deny_unknown_fields`": `packages/gx-protocol/src/lib.rs` @ c91146607205ac49303d1bcfe2fd6f9a86741500 (adaptation map).
- Wave-25 design doc, task-152.
- `crates/phlow-gauntlet/src/wire.rs` — `parse_envelope`, `Envelope`.
- `crates/phlow-gauntlet/src/tasks/task_152.rs`, `crates/phlow-gauntlet/tests/task_152.rs`.
