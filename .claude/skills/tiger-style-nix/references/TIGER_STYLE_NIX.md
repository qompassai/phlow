<!-- TIGER_STYLE_NIX.md — Independent language adaptation; no active scripts. -->
# Tiger Style for Nix

> **Safety first. Performance second. Developer experience third.**

**Guide version:** 1.0  
**Primary target:** Nix 2.x expressions; selected CLI and experimental features declared separately  
**Line-width review target:** 100 columns, subject to the language formatter  
**Ordinary function review threshold:** 70 physical source lines  
**Platform emphasis:** Arch Linux; portable core with explicit OS adapters

This guide follows the structure of your Lua guide while adapting contracts,
errors, numbers, ownership, concurrency, tooling, and examples to Nix.
It remains usable as plain Markdown; the rich-documentation sections are
optional and contain no required remote assets or executable scripts.

## Table of Contents

1. [The Tiger Style Contract](#1-the-tiger-style-contract)
2. [Language and Runtime Assumptions](#2-language-and-runtime-assumptions)
3. [File and Module Layout](#3-file-and-module-layout)
4. [Assertions, Preconditions, and Postconditions](#4-assertions-preconditions-and-postconditions)
5. [Bound Everything](#5-bound-everything)
6. [Control Flow](#6-control-flow)
7. [Error Handling](#7-error-handling)
8. [Absence, False, and Defaults](#8-absence-false-and-defaults)
9. [Numbers, Counts, and Units](#9-numbers-counts-and-units)
10. [Collections and Data Shapes](#10-collections-and-data-shapes)
11. [State and Mutation](#11-state-and-mutation)
12. [Function Design](#12-function-design)
13. [Scope](#13-scope)
14. [Resource Lifetime](#14-resource-lifetime)
15. [Memory and Allocation](#15-memory-and-allocation)
16. [Performance](#16-performance)
17. [Determinism](#17-determinism)
18. [Security](#18-security)
19. [Filesystem Safety](#19-filesystem-safety)
20. [Processes and Shells](#20-processes-and-shells)
21. [Concurrency and Async Work](#21-concurrency-and-async-work)
22. [Naming](#22-naming)
23. [Formatting](#23-formatting)
24. [Comments and Documentation](#24-comments-and-documentation)
25. [Dependencies](#25-dependencies)
26. [Language-Specific Engineering](#26-language-specific-engineering)
27. [Arch Linux Development](#27-arch-linux-development)
28. [Testing](#28-testing)
29. [Review Checklist](#29-review-checklist)
30. [Anti-Patterns](#30-anti-patterns)
31. [Reference Module](#31-reference-module)
32. [Images](#32-images)
33. [HTML](#33-html)
34. [CSS](#34-css)
35. [SVG Diagrams](#35-svg-diagrams)
36. [Video](#36-video)
37. [JavaScript in Documentation](#37-javascript-in-documentation)
38. [Mathematics and Resource Models](#38-mathematics-and-resource-models)
39. [Renderer Compatibility](#39-renderer-compatibility)
40. [Suggested Repository Layout](#40-suggested-repository-layout)
41. [Sources](#41-sources)

---

## 1. The Tiger Style Contract

This is an independent language adaptation of the engineering philosophy in
TigerBeetle's Tiger Style and the supplied **Tiger Style for Lua, version 1.0**.
It is not an official TigerBeetle standard. The hierarchy is a project policy:

1. Preserve correctness, invariants, resource bounds, and authority boundaries.
2. Make resource use predictable and optimize measured costs.
3. Make the implementation easy to read, review, test, and maintain.

The implementation should reveal permitted inputs, failure outcomes, maximum
work, mutable state, resource owners, and success postconditions. A compiler
check is useful evidence, not a replacement for a runtime trust boundary.

**MUST** marks a safety requirement of this guide. **SHOULD** marks a default
that may be changed with a documented reason. Language facts and project policy
are deliberately separate. The 70-line function threshold and 100-column target
are review aids; never damage idiomatic formatter output to satisfy a metric.

At each public operation, record: **input → validation → bounded computation →
validated change → commit → cleanup**. State precisely whether failure leaves
state unchanged, partially updated, or requiring recovery.

---

## 2. Language and Runtime Assumptions

Distinguish the Nix language, evaluator/CLI version, Nixpkgs revision, module
system, and target system. This guide uses ordinary Nix 2.x expressions; its
reference test does not require flakes. Declare `nix-command` and `flakes` when
a project chooses them instead of assuming every installation enables them.
Flake interfaces have an upstream experimental status that should be checked
against the selected implementation. [N1]

Nix is lazy and dynamically typed. Producing an outer attribute set is not proof
that all its fields have evaluated successfully. [N2]

---

## 3. File and Module Layout

Use small functions with explicit argument sets. Keep reusable pure expression
logic separate from derivations, NixOS/Home Manager modules, and flake outputs.
Avoid hiding system selection or package-set overrides in distant imports.
Prefer `let` bindings with explicit names over broad `with` scopes that make
identifier origins difficult to audit.

---

## 4. Assertions, Preconditions, and Postconditions

Use `assert condition; value` to express internal assumptions, but remember that
an assertion is encountered only when that expression is evaluated. It is not
Python-style debug-only syntax. For expected invalid configuration, return a
clear tagged result or issue an intentional, contextual evaluation error at the
configuration boundary. Module-system `assertions` are a different mechanism.
Force the values needed to establish a contract; do not assume shallow success.

---

## 5. Bound Everything

A count bound alone is not a complete resource budget. Bound item count,
individual item size, nesting depth, cumulative bytes, concurrent work, waiting
work, elapsed time, and retry amplification separately.

| Resource | Example policy | Exhaustion behavior |
| --- | --- | --- |
| Input bytes | 1 MiB before decoding | Reject before allocation grows further. |
| Collection items | 1,024 | Return a capacity error. |
| Nesting depth | 32 | Reject the structure. |
| Active jobs | 8 | Admit to a bounded queue or reject. |
| Queue entries | 64 | Apply backpressure; do not silently drop work. |
| Attempts | 3 total, within one deadline | Return the final classified failure. |
| Captured output | 256 KiB per stream | Stop capture and terminate or drain by policy. |

These values are examples, not universal defaults. Derive actual limits from
workload and deployment budgets. Streaming, lazy evaluation, and asynchronous
APIs do not automatically bound total consumption. Reject compressed data based
on both compressed and expanded sizes. A long-lived service can have an open-ended
lifetime while each iteration and its owned work remain bounded.

Retries MUST require an idempotent operation or an idempotency mechanism.
Use a shared deadline, bounded backoff, and an injected random source for jitter;
never multiply independent retry policies invisibly across layers.

---

## 6. Control Flow

Prefer finite transforms over recursive traversal of arbitrary external data.
Document recursion and fixed-point behavior in modules and overlays. Guard a
length bound before a fold, then make per-element work explicit. Lazy filters and
attribute construction can defer failures; forcing at a deliberate boundary makes
the point of failure predictable. Avoid repeated list concatenation in folds.

---

## 7. Error Handling

Use a consistent result attribute set such as `{ ok = false; error = "..."; }`
for recoverable validation. Use `throw` or contextual assertions when invalid
configuration must abort evaluation. `builtins.tryEval` is shallow and catches
only certain evaluation failures; it is not an arbitrary exception handler or a
sandbox. `builtins.deepSeq` can force nested results, but forcing must itself
have a bounded input domain. [N3]

---

## 8. Absence, False, and Defaults

Attribute absence, `null`, `false`, an empty list, and an empty attribute set
are different states. `attrs.value or fallback` supplies a default for an absent
attribute path, not for an existing `null` or `false`. Function argument defaults
also apply to missing arguments. Prefer `attrs ? value` when presence is part
of the contract. Module option merging has its own defaults and priorities;
do not confuse those with ordinary language operators. [N4]

---

## 9. Numbers, Counts, and Units

Keep counts and integer budgets within explicit limits, rather than relying on
implementation-specific overflow behavior. Reject floats when an integer is
required; `builtins.isInt` is useful at this boundary. Separate numeric values
from string units used by services. Test arithmetic near configured limits and
avoid coercing a number to a string early enough to conceal a type error.

---

## 10. Collections and Data Shapes

Lists and attribute sets are immutable values, but their members may be lazy.
Define accepted keys and validate value types deliberately. A list length bound
does not bound the bytes retained by its strings or nested values. For a strict
validation result, force all required elements at the boundary. Keep Nixpkgs
module option types separate from hand-written record schemas.

---

## 11. State and Mutation

Nix expressions do not mutate variables, but evaluation can select builds and
activation scripts that cause effects elsewhere. Make configuration changes as
new values and review the generated derivations/activation plan before applying
them. Avoid recursive attribute sets when ordinary explicit dependencies suffice;
self-reference can introduce evaluation cycles or surprising scope.

---

## 12. Function Design

Prefer explicit argument sets with documented required and optional fields.
Reject unknown arguments for narrow application configuration unless forward
compatibility is intentional. Keep a function that validates policy separate from
one that builds a derivation. Pass the package set and system intentionally;
do not import a floating ambient `<nixpkgs>` when reproducibility is required.

---

## 13. Scope

Keep `let` scopes small enough that binding origins remain visible. In modules,
understand the fixed point before reading values derived from `config` inside
conditions that define that same config. `rec` is a semantic tool, not a default
formatting preference. Avoid implicit package names supplied by a large `with`.

---

## 14. Resource Lifetime

The evaluator owns values; derivation builders own files, processes, and temporary
resources. Separate those lifetimes explicitly. A development shell does not
clean up arbitrary services it starts. Generated scripts need normal checked
cleanup and signal handling. Profile generations and store garbage collection
are deployment/storage policies, not substitutes for a builder's resource limits.

---

## 15. Memory and Allocation

Laziness can retain thunks and large dependency graphs. Use strict folds where
appropriate, but understand that strictness to weak head normal form is not
recursive forcing. `builtins.foldl'` does not make all nested accumulator fields
strict. Bound traversals and use `deepSeq` only for a justified bounded value.
Do not repeatedly rebuild large attribute sets inside a tight fold. [N3]

---

## 16. Performance

Measure evaluation separately from build time, downloads, substitution, and
activation. Avoid import-from-derivation when it obscures evaluation cost and
forces a build just to understand configuration. Share a reviewed package set
rather than repeatedly instantiating it unnecessarily. A binary cache improves
latency, but trust policy and cache provenance still need review.

---

## 17. Determinism

Pin source revisions and hashes; commit a reviewed lock file for flake projects.
Pure evaluation and content-addressed inputs improve reproducibility but do not
prove a build is deterministic. Builders can still depend on clocks, randomness,
CPU behavior, or undeclared external inputs where permitted. Test reproducibility
separately from “the input revision is pinned.” Use explicit systems and options.

---

## 18. Security

Never embed secrets in Nix strings that become store paths, derivations, generated
files, or logs; store objects are commonly readable by other local users. Runtime
secret delivery belongs to a separately reviewed mechanism. Evaluate and build
untrusted expressions under an appropriate OS policy; build sandboxing is not a
complete sandbox for arbitrary evaluation. Treat trusted users, substituters,
public keys, overlays, and fetchers as privilege/supply-chain boundaries. [N5]

---

## 19. Filesystem Safety

Distinguish a Nix path value from a string containing a runtime path. Interpolating
a path can copy source content into the store; filter sources deliberately to
exclude credentials, private data, generated caches, and unrelated files.
A source filter is not a generic sandbox. Use runtime descriptor-relative file
policy inside generated programs, and do not assume the immutable store prevents
an activation script from overwriting a mutable system path.

---

## 20. Processes and Shells

A derivation's `buildPhase` and similar attributes are commonly shell scripts;
string interpolation there is a shell boundary. Use fixed commands and the
pinned Nixpkgs `lib.escapeShellArg`/`lib.escapeShellArgs` helpers when constructing
POSIX-shell arguments. Those helpers are not SQL, JSON, PowerShell, or arbitrary
shell encoders. Escape Nix's own interpolation separately when writing literal
shell `${...}` syntax. Prefer structured service `ExecStart`/argv facilities
where the target module supports them.

---

## 21. Concurrency and Async Work

The expression language is not an imperative async task scheduler. Apply Tiger
Style concurrency bounds to evaluation workers, build jobs, substituters, and
generated services. Configure job parallelism and per-build resource limits
according to the selected evaluator/daemon. Cancellation must cover external
builders and owned child work. Generated service startup and shutdown deserve
the same timeout and ownership review as hand-written service code.

---

## 22. Naming

Use conventional Nix/Nixpkgs names where interoperating: `pname`, `version`,
`nativeBuildInputs`, `buildInputs`, and `meta`. Prefer `camelCase` for local
functions and policy fields, while preserving upstream option and package names.
Use `payloadSizeBytes` or an option type with documented units. Avoid inventing
nearly identical local aliases for well-known package attributes.

---

## 23. Formatting

Choose one formatter from the pinned development environment, such as Nixpkgs'
`nixfmt`, and record its version/interface. Two-space indentation is the common
profile here. Do not force Lua's four spaces onto formatter output. A 100-column
review target is useful, but deterministic formatting wins over manual wrapping.
Configure lint exclusions narrowly with an explanation.

---

## 24. Comments and Documentation

Document argument types, laziness/strictness boundaries, system assumptions,
store exposure, and derivation side effects. Module options need descriptions,
examples, sensible types, and an explicit merge policy. A comment claiming
“pure” must identify whether it means expression evaluation, a derivation's
inputs, or actual reproducible output.

---

## 25. Dependencies

Pin Nixpkgs and external sources. A flake lock identifies inputs but does not
make an overlay, build script, or source fetcher harmless. Review dependency
updates as code changes, including recursive inputs and new substituters.
Prefer explicit source hashes and revisions. Do not add trusted signing keys
or disable sandbox controls merely to make a build pass.

---

## 26. Language-Specific Engineering

Review the evaluation/build/runtime split on every module. Keep secrets out of
the store, avoid accidental evaluation cycles, and distinguish `mkDefault` /
`mkForce` option priorities from ordinary attribute replacement. On Arch, Nix is
an additional package/build environment; it does not turn the host into NixOS.
Do not copy NixOS-only activation instructions into an Arch shell workflow.

---

## 27. Arch Linux Development

Use a supported Arch installation and a coherent full-system update; avoid
partial upgrades. Prefer distribution packages where they meet the declared
toolchain contract. Pin project versions separately when Arch's rolling version
moves beyond that contract. Review AUR `PKGBUILD`, sources, checksums, and hooks.
Build as an ordinary user; never use `sudo` for project package managers.

Keep configuration, caches, state, and temporary runtime files in their proper
XDG locations when writing user applications. Honor explicit overrides; avoid
assuming the current directory is trusted. Do not put credentials in a repo,
build cache, environment dump, or command line. The commands below are validation
recipes, not automatic installation scripts. Run project build code only after
reviewing the repository and its dependencies.
```sh
nix --version
nix-instantiate --eval --strict examples/test_bounded_sum.nix
# In an already reviewed environment containing these tools:
nixfmt --check examples/*.nix
statix check .
deadnix --fail .
# Flake projects, only when their declared experimental features are enabled:
nix flake check --no-write-lock-file
```

Install/configure the Nix daemon deliberately; do not run a remote installer pipe
as a routine project build step. Keep daemon trust settings minimal.

---

## 28. Testing

Test evaluation results, expected invalid configurations, derivation builds,
and activation behavior as separate levels. Force complete results in validation
tests; a shallow success can hide a failing field. Check absent/null/false,
wrong types, empty lists, count limits, arithmetic bounds, and recursive failures.
Tests that need builds or network access must be labeled accordingly. Never
include production secrets in evaluation fixtures.

---

## 29. Review Checklist

- [ ] The compiler/runtime, platform, and supported build modes are explicit.
- [ ] External bytes are bounded before decoding and shape validation.
- [ ] Assertions diagnose bugs; external rejection survives release settings.
- [ ] Numeric domain, units, conversion, and overflow policy are documented.
- [ ] Collections have both count and byte budgets.
- [ ] Ownership and borrowing are clear on success and every failure path.
- [ ] State is committed only after validation, or partial mutation is documented.
- [ ] Cancellation stops or joins owned work; a timeout is not mistaken for cleanup.
- [ ] Stale asynchronous results cannot mutate a newer generation of state.
- [ ] Paths use a stated symlink and race policy; string prefixes are not containment.
- [ ] Processes have explicit argv, cwd, environment, deadlines, and output bounds.
- [ ] Secrets are absent from logs, command arguments, build outputs, and fixtures.
- [ ] Dependencies and build hooks have been reviewed and versions recorded.
- [ ] Ordering, clocks, locale, and randomness are explicit where reproducibility matters.
- [ ] Boundary, exhaustion, cleanup, and production-mode behavior are tested.
- [ ] Ordinary functions are reviewed near 70 lines; formatting uses one tool.
- [ ] Exceptions to this guide name an owner, reason, evidence, and review date.

---

## 30. Anti-Patterns

| Avoid | Prefer |
| --- | --- |
| Secret interpolated into a derivation | A reviewed runtime secret mechanism. |
| Floating `<nixpkgs>` in reproducible code | An explicit pinned input. |
| `tryEval` assumed to validate a whole result | Bounded deliberate forcing. |
| Broad `with pkgs;` obscuring name origins | Explicit bindings at important boundaries. |
| Raw user strings in `buildPhase` | Target-appropriate quoting or structured arguments. |
| Disabling the sandbox to fix unexplained failures | Diagnose undeclared inputs and required capabilities. |

---

## 31. Reference Module

The reference computes a total from at most **1,024 nonnegative integers**,
with a maximum total of **1,000,000**. It validates count first, then visits
elements in order, returning the first element/budget error encountered. It
uses `value > maximum - total` before addition. The input remains unchanged;
no partial successful result is published. A caller must not mutate borrowed
input concurrently. The decoded container already exists: upstream byte parsing
needs its own byte, allocation, and depth limits.

Time is O(n) with n <= 1,024 for the admitted arithmetic loop. Auxiliary
accumulator state is constant-sized; runtime representation, input storage,
validation overhead, and the bundled test fixtures are separate costs.

Complete source: [examples/bounded_sum.nix](examples/bounded_sum.nix). Run instructions and actual validation
results are in the collection README and VALIDATION.md. This is a deliberately
small reference for arithmetic/contracts, not a claim to implement the file,
process, or async policies described elsewhere.

```nix
# A pure validator. Source bytes must be bounded before producing this value.
let
  itemCountMax = 1024;
  totalMax = 1000000;
  failure = code: { ok = false; error = code; };
  step = state: value:
    if !state.ok then state
    else if !builtins.isInt value then failure "invalid-item"
    else if value < 0 then failure "invalid-item"
    else if value > totalMax - state.value then failure "total-exceeded"
    else { ok = true; value = state.value + value; };
in
input:
if !builtins.isList input then failure "invalid-input"
else if builtins.length input > itemCountMax then failure "too-many-items"
else
  let
    # Force this small accumulator: foldl' alone is not recursive strictness.
    result = builtins.foldl'
      (state: value:
        let next = step state value;
        in builtins.deepSeq next next)
      { ok = true; value = 0; }
      input;
  in builtins.deepSeq result result
```

The Nix entry point traverses an already constructed list to determine its length;
this does not bound evaluator work or bytes before that value exists.

The separate [Nix test expression](examples/test_bounded_sum.nix) forces and compares the results.

---

## 32. Images

Use images only when they explain a relationship better than text. Give each
image descriptive alternative text and ship the asset in the same repository.
Prefer a static SVG for precise diagrams and WebP/PNG for screenshots. Strip
private metadata and credentials from screenshots before publication.

A linked image must exist. This guide does not contain placeholder image links;
the optional image syntax below is a template for a future real asset:

```markdown
![Owned resources and their cleanup paths](assets/resource-lifetime.svg)
```

Keep the contract available as prose so an image failure cannot hide a rule.

---

## 33. HTML

Use semantic Markdown first. Native disclosure widgets provide a useful
progressive enhancement without scripts:

<details>
<summary><strong>Failure contract</strong></summary>

A failure result identifies whether the operation committed state. Resource
cleanup has one owner, and cleanup errors follow a documented reporting policy.

</details>

Do not embed unsanitized user HTML. A trusted documentation renderer is a separate
execution environment from the application being documented.

---

## 34. CSS

Styling must not carry meaning that disappears when CSS is removed. Use a local
stylesheet in a controlled documentation site; GitHub-style renderers may strip
it. Maintain readable contrast, keyboard focus visibility, and a printable view.

```css
.tiger-note {
    border-inline-start: 0.25rem solid #087ea4;
    padding-inline-start: 1rem;
}
@media print {
    .tiger-interactive { display: none; }
}
```

This is an illustrative stylesheet, not active content required by this guide.
Avoid remote fonts, trackers, and styles merely to present a coding standard.

---

## 35. SVG Diagrams

For architecture drawings, use static SVG with a title, description, explicit
view box, and no scripts or external resource loads. Treat an untrusted SVG as
active document content until sanitized. Store actual assets alongside the guide.

The ownership diagram should show the resource's creator, current owner,
transfer point, and success/error/cancellation cleanup paths. A straight sequence
of labels is better expressed in prose than a large decorative diagram.

---

## 36. Video

Videos can demonstrate debugging or a formatter workflow; they must not be the
only source of a rule. Provide a transcript or accompanying steps, playback
controls, captions where practical, and a link fallback. Do not autoplay.

The following is a template, not a claim that media files are bundled:

```html
<video controls preload="none">
  <source src="assets/debugging.webm" type="video/webm">
  <a href="assets/debugging.webm">Open the debugging demonstration</a>
</video>
```

Verify the files exist before enabling this markup in a published page.

---

## 37. JavaScript in Documentation

Documentation does not require JavaScript. Prefer `<details>` for disclosure
and plain links for navigation. If a trusted site adds interactivity, keep scripts
local, apply an appropriate Content Security Policy, and validate DOM lookups.
Insert untrusted text using `textContent`, not `innerHTML`; avoid inline event
handlers and dynamic evaluation. The full standard must remain readable with
scripts disabled. No executable scripts are embedded in this guide.

---

## 38. Mathematics and Resource Models

Use mathematics to state a checkable contract, with a prose fallback.
For a zero-based slice of `length` elements, a range starting at `offset` and
containing `count` elements is valid when:

$$
0 \leq offset \leq length,\qquad 0 \leq count \leq length-offset.
$$

Check `offset <= length` before subtracting. This avoids an overflowing
`offset + count` test and allows an empty range at the end.

For admission control with at most $J$ active jobs and $Q$ queued jobs:

$$
M_{retained} \leq M_{base} + J M_{job,max} + Q M_{queued,max} + M_{cache,max}.
$$

Count parser expansion, allocator overhead, stacks, and runtime overhead in
real budgets; this model only covers the categories explicitly included.
For a measured latency budget, distinguish queue wait, I/O, compute, and commit.
No equation here is a hard-real-time guarantee.

---

## 39. Renderer Compatibility

| Feature | Plain Markdown reader | GitHub-style host | Controlled docs site |
| --- | --- | --- | --- |
| Text, tables, fenced code | Text remains useful | Supported with GFM tables | Supported |
| Relative links/images | Host-dependent | Supported when assets exist | Supported |
| `<details>` | May display markup | Usually supported | Supported |
| SVG, video | Link/text fallback | Sanitized or restricted | Policy-dependent |
| CSS and JavaScript | Not needed | Often removed | Optional, trusted only |
| Mathematics | Source remains readable | Host extension | Math renderer needed |

The shared contract and language examples remain understandable without rich
rendering. No renderer is instructed to enable untrusted scripts. Internal table
of contents links follow ordinary GitHub heading anchors.

---

## 40. Suggested Repository Layout

A minimal repository can place this guide at `docs/TIGER_STYLE_NIX.md`,
production code in its language-appropriate source directory, and boundary tests
alongside that code. Keep development-tool configuration, dependency pins, and
CI checks under version control. Optional real media belongs in `docs/assets/`.

In this downloadable collection, the guide is at the archive root and its
reference source is `examples/bounded_sum.nix`. Relative links work when the archive is extracted
as a whole. No image/video files are required. Treat the example as a teaching
module and adapt its file/package layout before adding it to a production repo.

Keep generated binaries, caches, virtual environments, credentials, and test
scratch directories out of version control. Record tool versions in CI logs,
with environment values redacted.

---

## 41. Sources

The supplied **TIGER_STYLE_LUA(8).md**, version 1.0, supplies the document
structure and engineering priorities. Its language-specific rules are adapted,
not mechanically renamed. Primary conceptual source:
[TigerBeetle Tiger Style](https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/TIGER_STYLE.md).

Language/tool references (reviewed 2026-09-23; use documentation matching your
pinned version):

- **[N1]** [Nix flakes and status](https://nix.dev/concepts/flakes.html)
- **[N2]** [Nix evaluation model](https://nix.dev/manual/nix/2.34/language/evaluation)
- **[N3]** [Nix built-ins: foldl, tryEval, deepSeq](https://nix.dev/manual/nix/2.34/language/builtins.html)
- **[N4]** [Nix language operators](https://nix.dev/manual/nix/2.34/language/operators.html)
- **[N5]** [Nix security](https://nix.dev/manual/nix/2.34/installation/multi-user.html)
- **[N6]** [Nixpkgs manual](https://nixos.org/manual/nixpkgs/stable/)

The numeric limits, naming choices, function-size review threshold, and example
API are this guide's policy choices, not claims that the language specification
mandates them. Toolchain validation scope is recorded in [VALIDATION.md](VALIDATION.md).

---

# Compact Tiger Style Nix Card

<details>
<summary><strong>Open the condensed rules</strong></summary>

**Safety:** validate at trust boundaries; distinguish absence and failure; bound
work and storage; expose units; make ownership explicit; preserve invariants
through errors and cancellation; keep privileged operations narrow.

**Performance:** model capacities first; batch expensive work; profile realistic
loads; account for allocation and retention; keep hot paths simple and defined.

**Developer experience:** follow Nix's conventions and one pinned formatter;
keep contracts visible; use precise types without confusing them with runtime
validation; test boundaries and cleanup; document justified exceptions.

</details>

**Final rule:** a reviewer should be able to identify the program's limits,
invariants, state transitions, resource owners, failure behavior, and external
capabilities without reverse-engineering hidden conventions.
