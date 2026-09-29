# Blocked gauntlet tasks

Rust-kind, blocked on an operator trust decision (zero fix attempts):

- (task-242, check-child AF_INET/AF_INET6 socket() is not denied: ipv4_raw_socket_denied, ipv6_raw_socket_denied. The EPERM requirement needs a seccomp filter on the spawning thread, which children inherit. phlow-checks is `forbid(unsafe_code)`; rustix's seccomp surface could not be verified from this sandbox, and no seccomp crate (seccompiler/libseccomp) is in Cargo.lock. Unblock by approving either a reviewed seccomp dependency or a scoped unsafe boundary. Landlock and network namespaces do not make socket() itself fail.)

## Rescope round (2026-09-29): 31 tasks unblocked

Per Matt's decision the Diver-probing NvimLua tasks were rescoped to phlow: each file is now
`TaskKind::Rust` and drives the new `crates/phlow-approval` crate directly (no Neovim, Diver
untouched). IDs, names and case names are unchanged. Every task passed on the first attempt,
measured with `cargo test -p phlow-gauntlet --lib` (124/124 cases for these 31 tasks).

- (task-209, denials retain tool and exact paths: `decide` returns `Decision { verdict, reason, scope }`)
- (task-210, extra `permissions`/`tools` rejected: closed request schema in `Request::from_json`)
- (task-211, forged and consumed IDs refused: queue-bound IDs, `WrongState` on replay, queue not `Clone`)
- (task-212, anonymous and agent approval refused: explicit operator allowlist on `ApprovalQueue::new`)
- (task-213, pending/approved scope cannot expand: queue owns admitted scope; widening needs a new request)
- (task-214, unknown top-level and rule fields rejected by name: `Error::UnknownField`)
- (task-215, `legacy`/`permissive` flags and malformed scopes rejected: no parser modes exist)
- (task-216, `version` required, 1 retained, others rejected: `POLICY_VERSION`, `Error::UnsupportedVersion`)
- (task-217, allow/deny conflict denies in either order: most restrictive matching rule wins)
- (task-218, mixed paths/endpoints denied: allow rules must cover every requested resource)
- (task-219, readers cannot edit history or attribution: `get` returns owned snapshots; no re-decide)
- (task-220, expired requests refused even unswept: `decide` checks the deadline and marks `Expired`)
- (task-221, denial builds an exact narrowed proposal: `Decision::proposal`)
- (task-222, approvals revocable, revocation visible: `ApprovalQueue::revoke`, attribution kept)
- (task-223, `false`/map rules and invalid defaults fail closed: parse error, `decide(None, ..)` denies)
- (task-224, union of separately approved scopes denied: no rule combining)
- (task-225, permission_delta exposed on records: `PermissionDelta::between` at admission)
- (task-226, true set-difference delta: `BTreeSet` difference, not cardinality)
- (task-227, byte-deterministic delta: sorted lists, canonical `PermissionDelta::to_json`)
- (task-228, delta independent of model claims: summary inert; claimed `permission_delta` rejected at admission)
- (task-229, before/after snapshots retained and isolated: owned `PermissionSet`s on the record)
- (task-230, permission sets bounded and validated: `PERMISSIONS_MAX` = 4096; non-string/sparse rejected)
- (task-231, permission identity byte-exact: case and scope changes are revoke + grant)
- (task-232, pending/get surfaces agree on the computed delta: `ApprovalQueue::pending`)
- (task-239, policy state isolated from input and agents: parsed copy, no mutating API, `compile_fail` doctest)
- (task-240, read handles cannot approve: `&mut` transitions, snapshot edits inert, forged fields rejected)
- (task-241, missing and model actors refused: allowlist plus `RESERVED_ACTORS` at queue construction)
- (task-246, decisions carry actor and monotonic timestamp; scope not rewritable: `decided_by`/`decided_at`)
- (task-247, denials carry tool, risk and scope: same `Decision` record as task-209)
- (task-248, events append-only and owned: `EventSink` stores and returns copies)
- (task-249, envelopes reject empty spawn identity and pre/at-epoch time: `make_envelope`)

## History: Diver-probe root cause (2026-09-28, resolved by the rescope above)

Common root cause for task-209 .. task-225: every probe spawns `nvim` with
`NVIM_APPNAME=diver-fixed` and calls Diver's `ai.harness.policy`,
`ai.harness.approval` (and `ai.harness.events` for task-225). Those modules
live only in `~/.config/diver-fixed/lua/ai/harness/` (upstream:
`qompassai/diver/lua/ai/harness/`). No phlow source reaches these probes, and
the brief forbids editing `~/.config/diver-fixed`. Zero fix attempts were
made: every possible fix is in a forbidden directory.

The task-226 .. task-249 NvimLua batch (appended 2026-09-28) had the same root
cause via the shared `task_225::probe` prelude: 41 failing cases, every one
`Ok(false)`, and 15 positive controls passed.

## task-242 round 2 (2026-09-29): resolved, not blocked

Matt approved the seccompiler dependency and a scoped unsafe boundary. New crate
`crates/phlow-seccomp` (seccompiler =0.5.0, Apache-2.0 OR BSD-3-Clause, rust-vmm)
installs a pre_exec filter in every check child spawned by `phlow-checks`:
socket(AF_INET/AF_INET6) and io_uring_setup (plus x86_64 x32 aliases) -> EPERM.
`phlow-checks` stays `forbid(unsafe_code)`. First attempt: 4/4 cases pass
(`cargo test -p phlow-gauntlet --lib task_242`). The task-242 entry at the top of
this file is stale and can be removed by its owner.
