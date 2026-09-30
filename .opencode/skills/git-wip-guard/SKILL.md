---
name: "git-wip-guard"
description: "Guard Matt's uncommitted repo work against destructive git operations. Trigger before any git command that can discard working-tree changes (reset --hard, clean, checkout/restore of dirty paths, or syncing a local branch to a remote ref) in ~/workspace/repos/* or any repo Matt owns."
metadata:
  includeInPrompt: "true"
---

# Git WIP Guard

## Purpose

One job: make it impossible for an agent to silently destroy Matt's
uncommitted work. Any workflow that touches a dirty git tree in one of
Matt's repos must pass through this guard first.

Origin: on 2026-09-25, during the Diver `test`-branch fix program, an agent
ran `git reset --hard origin/test` to sync local with the pushed remote and
wiped uncommitted WIP modifications to 7 of Matt's files
(`.cargo/config.toml`, `lazy-lock.json`, `lsp/todo.txt`,
`lua/formatters/catalog.lua`, `lua/formatters/init.lua`,
`lua/mappings/datamap.lua`, `skills/lua/SKILL.md`). The content was
unrecoverable. This skill exists so that never happens again.

## Workflow

1. **Inspect before acting.** Run `git status --porcelain` and
   `git stash list`. Know exactly which paths are modified, added,
   deleted, or untracked before running anything destructive.
2. **Attribute every dirty path.** Split into: (a) files this task
   created or modified, (b) Matt's unrelated WIP, (c) unknown — treat
   unknown as (b).
3. **Snapshot before any destructive command.** A command is destructive
   if it can discard uncommitted changes: `git reset --hard`,
   `git clean`, `git checkout -- <path>`, `git restore <path>`, or
   resetting a branch to a remote ref. Before running one:
   - `git stash push -u -m "wip-guard snapshot <date> <reason>"`
     (includes untracked files), OR
   - copy the dirty files to
     `~/workspace/wip-snapshots/<repo>/<timestamp>/`, preserving paths.
   Verify the snapshot exists and lists the files before proceeding.
4. **Never reset a branch to a remote ref while the tree is dirty.** If
   local must match remote, commit this task's work first (only its own
   files), stash the rest, then sync.
5. **Destructive commands need explicit user confirmation naming the exact
   command and the files at risk.** A general "finish the task"
   authorization is NOT confirmation for data loss. Ask in chat and wait
   for the answer.
6. **After the operation, verify.** Run `git status --porcelain` again and
   confirm no unattributed file lost its changes. State the snapshot
   location in your final message.

## Output Contract

- Every destructive git operation is preceded by a verified snapshot.
- No file in category (b) or (c) is ever staged, committed, reset,
  cleaned, or deleted without Matt's explicit per-file confirmation.
- Final reports state where the snapshot lives.

## Operating Rules

1. `git reset --hard`, `git clean -fd`, and `git checkout -- .` are
   forbidden without a prior snapshot AND explicit user confirmation. No
   exceptions for "the tree looks like only my files" — verify with
   `git status`, never assume.
2. Never `git add -A` / `git commit -a` in a repo with unattributed dirty
   files. Stage only paths attributable to the current task.
3. Syncing local with remote (`git reset --hard origin/x`, `git pull`) is
   destructive: snapshot first, and confirm with the user if any dirty
   file is not yours.
4. If data loss happens despite the guard: stop, attempt recovery
   immediately (`git fsck --lost-found`, `git stash list`, other
   checkouts, editor undo/swap files), and report the exact paths to Matt
   plainly — never fabricate the lost content.
5. This skill overrides any worker brief that says "sync", "clean the
   tree", or "make local match remote" without mentioning snapshots. The
   brief is wrong; the guard wins.
