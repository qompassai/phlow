---
name: tiger-style-nix
description: >
  Write or review Nix expressions in Matt's Tiger Style: safety-first Nix standard with explicit contracts, assertions, bounded work, and disciplined ownership. Applies whenever Nix expressions are written, regenerated, refactored, reviewed, or debugged - from flake.nix and packages to NixOS and Home Manager modules and dev shells. Toolchain pairing: nil and nixd for LSP (diagnostics, completions), alejandra and nixfmt for formatting, nix-debug-adapter for DAP debugging. Covers pure evaluation discipline, explicit inputs, pinned sources, and half-adversarial test splits. See references/TIGER_STYLE_NIX.md for the full guide.
license: Apache-2.0
compatibility: >
  Requires Nix with nil or nixd for LSP, alejandra or nixfmt for formatting, and nix-debug-adapter for DAP debugging. Built for Neovim with Matt's diver config (lsp/nil_ls, lsp/nixd_ls, formatters/alejandra, lua/dap/nix); usable by any coding agent that can read the guide and invoke the toolchain.
metadata:
    language: nix
    lsp: nil,nixd
    formatter: alejandra
    dap: nix-debug-adapter
    scip_indexer: not-configured
allowed-tools: Read Edit Bash
---

# Tiger Style Nix

## Purpose

Apply Matt's Tiger Style standard to Nix expressions. Priority order, always:
**Safety > Performance > Developer Experience.** The full guide lives in
[references/TIGER_STYLE_NIX.md](references/TIGER_STYLE_NIX.md); this file is
the operational core. When the two disagree, the full guide wins.
Rules the Lua template covers but Nix lacks (async callbacks, OS handles)
are omitted rather than invented.

## Workflow

1. Determine the Nix language version, evaluator/CLI version, Nixpkgs
   revision, module system, and target system. Declare `nix-command` and
   `flakes` when a project uses them; never assume them silently.
2. Lay the module out top-to-bottom: header/purpose, pinned inputs, small
   `let` bindings with explicit names, pure expression logic first,
   derivations / modules / flake outputs after. Keep reusable pure logic
   separate from derivations and modules.
3. Validate external input with `{ ok = false; error = "descriptive"; }`;
   assert internal invariants with `assert cond; value` (one invariant per
   assert). Remember an assert only fires when that expression evaluates.
4. Bound everything with named constants (`itemCountMax`, `totalMax`):
   list lengths, nesting depth, byte sizes, jobs, attempts.
5. Review against the checklist below before calling the code done.
6. Validate with the pinned toolchain per the guide's recipes:
   `nixfmt --check`, `statix check .`, `deadnix --fail .`; for flake
   projects `nix flake check --no-write-lock-file` once their declared
   experimental features are enabled.

## Operating Rules

- **Contracts first.** Every function answers: permitted inputs, rejected
  inputs, max work, which derivations/builds/activation effects it selects,
  and failure behavior.
- **Assertions are for programmer errors and invariants**, never for
  expected invalid configuration. For that, return a tagged
  `{ ok = false; error = ...; }` or `throw` with context at the boundary.
  Force the values needed to establish a contract — shallow success proves
  nothing.
- **Bound everything.** No unbounded fold, no recursion over
  attacker-controlled depth. Retry budgets are named constants and require
  an idempotent operation.
- **Control flow:** prefer finite transforms over recursive traversal of
  external data. Guard a length bound before a fold, make per-element work
  explicit, avoid repeated list concatenation in folds.
- **Absence/null/false discipline:** absence, `null`, `false`, `[]`, and
  `{}` are different states. Never `attrs.value or fallback` when `false`
  or `null` is valid — check `attrs ? value` explicitly. Do not confuse
  module option merging with ordinary `or`.
- **Laziness discipline:** producing an attrset is not proof its fields
  evaluated. Document laziness/strictness boundaries; force at deliberate
  boundaries with `builtins.deepSeq` over bounded values only. Prefer
  `foldl'`; remember it is strict only to weak head normal form.
- **Names are `camelCase` with units last:** `payloadSizeBytes`,
  `itemCountMax`, `totalMax`. Use conventional Nixpkgs names where
  interoperating (`pname`, `version`, `nativeBuildInputs`, `meta`). No
  near-duplicate aliases of well-known attributes.
- **Formatting:** 2-space indent, max 100 columns, nixfmt from the pinned
  dev environment (record its version/interface). Deterministic formatter
  output wins over manual wrapping; never force Lua's four spaces onto Nix.
- **Functions:** explicit argument sets with documented required/optional
  fields; reject unknown arguments for narrow configuration. Ordinary
  functions stay near or below 70 lines. Keep policy validation separate
  from derivation building.
- **Scope:** keep `let` scopes small; no broad `with` scopes that hide
  identifier origins. `rec` is a semantic tool, not a default. Pass the
  package set and system intentionally — no floating `<nixpkgs>` in
  reproducible code.
- **State:** expressions do not mutate, but evaluation selects builds and
  activation effects. Make configuration changes as new values and review
  the generated plan before applying; avoid recursive attrsets when
  explicit dependencies suffice.
- **Security:** never interpolate secrets into strings that become store
  paths, derivations, generated files, or logs — runtime secrets belong to
  a separately reviewed mechanism. Treat substituters, signing keys, and
  fetchers as trust boundaries; never disable the sandbox to make a build
  pass.
- **Shells and filesystem:** `buildPhase` is a shell boundary — fixed
  commands plus `lib.escapeShellArg`/`lib.escapeShellArgs`; escape Nix
  interpolation separately for literal shell `${...}`. A Nix path value is
  not a runtime-path string; interpolating a path copies content into the
  store, so filter sources deliberately.
- **Determinism:** pin source revisions and hashes; commit a reviewed flake
  lock; use explicit systems. Pure evaluation is not proof of a
  deterministic build.
- **Comments explain why**, not what. Document argument types,
  laziness/strictness boundaries, system assumptions, store exposure, and
  derivation side effects. Module options need description, example, type,
  and merge policy.

## Output Contract

Nix you produce for Matt must: evaluate cleanly under the declared Nix
version, document argument sets on functions, assert its invariants, return
tagged `{ ok = false; error = ...; }` on expected invalid input, bound all
folds/traversals with named constants, stay near ≤70 lines per function and
≤100 columns, format clean with the pinned nixfmt, and pass the review
checklist.

## Review Checklist

- [ ] Language/evaluator/Nixpkgs/module-system/target explicit; experimental features declared.
- [ ] External input validated with tagged results; internal invariants asserted and forced where the contract needs it.
- [ ] All folds/traversals bounded by named constants; no unbounded recursion over external data.
- [ ] Absent vs `null` vs `false` vs `[]` vs `{}` deliberate; no `or` default clobbers a valid value.
- [ ] Laziness/strictness boundaries documented; `deepSeq` used on bounded values only.
- [ ] Names: Nixpkgs conventions where interoperating, `camelCase` with units for locals.
- [ ] No secrets in store paths/derivations/logs; shell args escaped; sources filtered.
- [ ] Dependencies pinned with hashes; flake lock committed; sandbox intact.
- [ ] Functions ≤ ~70 lines, lines ≤ 100 cols, nixfmt-clean, comments explain intent.
