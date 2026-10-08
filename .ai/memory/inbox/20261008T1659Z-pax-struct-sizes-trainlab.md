# Struct-size audit: phlow-trainlab records (2026-10-08)

Measured every record type stored in bulk by the run/eval loops
(size_of, x86_64, pinned nightly) against the crate's own caps.
rustc's default repr already lays each one out at its floor, and every
record struct here is serde-serialized in declaration order (the
receipt config hash depends on it), so field reordering was never an
available lever. The audit's product is compile-time layout pins:

- GroupRecord 136 B (receipt.rs) — at most GROUPS_MAX (256) per receipt
- GroupExportRecord 168 B (export.rs) — at most 256 per export
- CodingTask 168 B, Case 56 B (task.rs) — at most 512 tasks per split
- TaskPassk 80 B (runner.rs) — at most 512 rows per pass@k report

Rejected with arithmetic: narrowing GroupRecord's usize counters
(group <= 256 by GROUPS_MAX, exact_rollouts <= 64 by GROUP_SIZE_MAX)
to u32 would save 8 B/record — at most 2 KB per run at the cap —
against a public field-type change; the run's retained memory is heap
text (prompts, completions), not record descriptors. Evaluation
(48 B) is per-sample but transient (one live at a time in the runner
loop), so it is audited but unpinned.

Gates: cargo test -p phlow-trainlab 57/57; clippy --all-targets
-D warnings clean; rustfmt clean on touched files.
