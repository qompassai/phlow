# task-188: no foreign-agent parsers

**Kind:** rust · **Status:** pass · **Wave:** 30 · **Commits:** pending (wave 30)

## ELI5

Ghostex (the tool this wave adapts) knows how to read the history
folders of several different AI coding assistants — it has a little
parser for each one. phlow must not do that: it may only index its
own session files, and it must never go poking around other
programs' private folders. This task proves two things with two
very different methods: first, a robot reads the engine's own
source code and confirms it never mentions foreign agent folders or
a per-agent parser module; second, a fake "other agent" folder full
of tempting files is placed next to the session folder, and the
engine is watched to prove it never opens a single file there.

## What this task attempts

- **Goal:** prove the adapted engine indexes only phlow's own
  session format and never touches foreign agent trees.
- **Mechanism:** static source scan of `session_find.rs` + `FsLog`
  read-path auditing during a scan next to a fake foreign tree;
  driver `src/tasks/task_188.rs`.
- **Success criterion:** zero of the 5 forbidden markers
  (`.claude`, `.codex`, `.gemini`, `agent.rs`, `Agent::`) in the
  engine source; zero recorded reads under the fake foreign tree;
  exactly the 5 phlow sessions indexed.
- **Non-goals:** search quality, performance, or what a real
  foreign tree would contain.

## What happened

Pass on the first executable gate run (the wave's shared compile blockers were fixed before any test executed). Integration tests 2/2:
`no_foreign_agent_references` (0 hits across 5 markers) and
`fake_foreign_tree_ignored` (0 reads under the fake tree, 5/5
phlow rows indexed). ~0.00s on primo.

## Where it went wrong

One self-inflicted failure, caught by reading the engine before
gating:

- **Stage:** pre-gate self-review of `session_find.rs`.
- **Symptom:** the module doc comment originally said "Ghostex's
  per-agent parsers (foreign agent homes such as `~/.claude`)" —
  the task's own static scan would have flagged the engine for the
  forbidden `.claude` marker.
- **Root cause:** documentation used the concrete example the test
  forbids. The scan is intentionally dumb (substring match), so
  intent doesn't matter — presence does.

(The pre-gate E0521 lifetime fix is documented in task-186.md.)

## The fix — what changed and why

- **Changed:** `src/session_find.rs` module docs: the sentence now
  reads "Ghostex's per-agent parsers for other agent CLIs' history
  directories" and adds "The scanner walks exactly the configured
  session root and skips dot-directories, so foreign trees are never
  entered." A post-edit `grep` over the file confirms zero hits for
  all 5 markers.
- **Commit:** pending (wave 30).
- **Why:** the test scans the shipped source, so the source must be
  clean — not the test weakened. Rewording loses no information:
  the boundary is "configured session root only", which the new
  text states more precisely than the example did.
- **Source:** `src/tasks/task_188.rs` `FORBIDDEN_MARKERS` (the
  contract under test).
- **Validation agents:** `cargo test -p phlow-gauntlet --test task_188`
  on primo → 2/2 pass; manual `grep -n` over `session_find.rs`
  confirms 0 matches.
- **Adversarial agents:** the fake foreign tree is the adversarial
  half — a sibling directory named after another agent's home,
  containing 3 valid-looking `.session.json` files plus a
  subdirectory, placed to tempt any over-eager directory walk.
- **New convention (if any):** none — but the lesson is recorded:
  when a test greps the source for forbidden strings, the source's
  own prose is in scope. Write docs accordingly.

## Full technical depth

V1 is a static scan: read `src/session_find.rs` via
`CARGO_MANIFEST_DIR` (so it checks the shipped file, not a copy),
count substring matches for each of the 5 markers, fail on any
non-zero count. The markers cover the three foreign home-directory
spellings the wave design names, plus `agent.rs` / `Agent::` for
Ghostex's per-agent parser module shape.

V2 is behavioral. `fresh_temp_dir` creates `sessions/` (5 phlow
`.session.json` files) and a sibling fake foreign tree with 3
decoy session files and a nested subdirectory. `scan_and_index`
runs with an `FsLog`; every recorded read path is checked to not
start with the foreign root. The scanner's directory walk
(`collect_session_files`) only descends into the configured root
and skips dot-directories, so the sibling tree is unreachable by
construction — the test proves the construction, not the hope.
Rows indexed must equal exactly 5: a foreign file leaking into
the index would show up as row 6+.

Boundary honesty: the walk skips dot-directories *inside* the
session root too, which is the conservative choice — a
`.git`-style directory inside the root is never entered.

## Sources

- Primary: `crates/phlow-gauntlet/src/session_find.rs`
  (module docs, `collect_session_files`);
  `crates/phlow-gauntlet/src/tasks/task_188.rs`
  (`FORBIDDEN_MARKERS`, both cases);
  `crates/phlow-gauntlet/tests/task_188.rs`.
- Secondary: `~/workspace/ghostex-recon/adaptation-map.md`
  (which Ghostex pieces were in/out of scope for wave 30).
