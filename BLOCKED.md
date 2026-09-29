# Blocked gauntlet tasks

Common root cause for task-209 .. task-225: every probe spawns `nvim` with
`NVIM_APPNAME=diver-fixed` and calls Diver's `ai.harness.policy`,
`ai.harness.approval` (and `ai.harness.events` for task-225). Those modules
live only in `~/.config/diver-fixed/lua/ai/harness/` (upstream:
`qompassai/diver/lua/ai/harness/`). No phlow source reaches these probes, and
the brief forbids editing `~/.config/diver-fixed`. Zero fix attempts were
made: every possible fix is in a forbidden directory. Unblock by implementing
the listed invariants in the Diver repo, then refreshing diver-fixed.

Failing cases measured with `cargo test -p phlow-gauntlet --lib` (the task
files' own `#[cfg(test)]` probes, real headless Neovim), 2026-09-28.
Case names are measured; the one-line reasons are paraphrased from the case
names and each file's "Honest scope" doc, not traced through the Lua source.

- (task-209, policy.decide omits `tool` and `paths` from deny decisions: denial_retains_tool, denial_retains_exact_paths)
- (task-210, policy accepts extra tools/permissions beyond the single approved scope: extra_tools_rejected, extra_permissions_rejected)
- (task-211, approval accepts forged and copied consumed tokens: forged_id_rejected, copied_consumed_token_rejected)
- (task-212, approval accepts anonymous and agent self-approval: anonymous_approval_rejected, agent_self_approval_rejected)
- (task-213, approved and pending scope can be expanded after request: approved_scope_cannot_expand, pending_scope_cannot_expand)
- (task-214, policy.new accepts unknown top-level and rule fields: unknown_top_level_rejected, unknown_rule_field_rejected)
- (task-215, policy.new accepts legacy bypass flag and malformed permissive scope: legacy_bypass_flag_rejected, permissive_malformed_scope_rejected)
- (task-216, no policy version field contract: missing_version_rejected, version_one_retained, future_version_rejected)
- (task-217, allow rule before deny rule allows instead of denying: allow_then_deny_denies)
- (task-218, request mixing matched and unmatched paths/endpoints is allowed: mixed_paths_denied, mixed_endpoints_denied)
- (task-219, readers can mutate decision history/attribution: reader_cannot_edit_history, reader_cannot_delete_attribution)
- (task-220, expired request can be approved before a sweep: unswept_expired_cannot_approve)
- (task-221, denial record lacks proposal/scope round-trip: denial_is_sufficient_for_proposal, denial_round_trip_preserves_only_scope)
- (task-222, no revocation of approved requests: approved_can_be_revoked, revocation_visible_on_next_read)
- (task-223, map-shaped or `false` rules do not fail closed: map_rules_fail_closed, false_rules_fail_closed)
- (task-224, union of separately approved scopes is allowed: union_of_scopes_denied, union_with_unapproved_scope_denied)
- (task-225, approval queue drops permissions_before/after; no permission_delta: all four cases)

## task-226 .. task-250 batch (appended 2026-09-28)

Same root cause for every NvimLua-kind task below: the probes (shared
`task_225::probe` prelude) require Diver's `ai.harness.approval`,
`ai.harness.policy` and `ai.harness.events` from `~/.config/diver-fixed`, a
forbidden directory. Zero fix attempts were made. Measured with
`cargo test -p phlow-gauntlet --lib`, real headless Neovim: 41 failing cases,
every one `Ok(false)` (the probe ran and the invariant did not hold; no
fixture/driver errors), and 15 positive controls passed. Reasons are
paraphrased from the case names and "Honest scope" docs, not traced through Lua.

- (task-226, approval records no set-difference permission_delta: add_only, remove_only, rename_remove_add, equal_size_disjoint)
- (task-227, permission_delta not byte-deterministic/sorted set: identical_inputs_identical_bytes, sorted_additions, permutation_invariant, duplicates_are_set_members)
- (task-228, delta not computed independently of model-supplied summary/delta: summary_absent, summary_descriptive, lying_summary_ignored, forged_delta_ignored)
- (task-229, before/after permission snapshots not retained or isolated from aliasing: input_retained, two_independent_requests, caller_mutation_isolated, reader_mutation_isolated)
- (task-230, permission sets not validated or bounded: single_permission, large_valid_set, non_string_member_rejected, sparse_set_rejected)
- (task-231, permission identity not preserved exactly: exact_scope_unchanged, independent_scopes, case_change_is_change, scope_widening_visible)
- (task-232, pending/get surfaces lack a faithful delta: pending_exposes_delta, get_and_pending_agree, summary_cannot_hide_revoke, empty_claim_cannot_hide_rename)
- (task-239, policy state can be widened by aliases/direct writes: input_alias_cannot_grant, agent_write_refused)
- (task-240, a read handle can approve: read_handle_cannot_approve)
- (task-241, approval accepts a missing or model actor: missing_actor_refused, model_actor_refused)
- (task-246, approval record lacks decision timestamp; scope rewritable: decision_has_timestamp, scope_cannot_be_rewritten)
- (task-247, denial record lacks tool and scope: denial_has_tool, denial_has_scope)
- (task-248, events append/read aliases can rewrite or erase history: append_input_cannot_rewrite, reader_cannot_erase_scope)
- (task-249, make_envelope accepts empty spawn identity and invalid time: empty_spawn_rejected, invalid_time_rejected)

Rust-kind, blocked on an operator trust decision (zero fix attempts):

- (task-242, check-child AF_INET/AF_INET6 socket() is not denied: ipv4_raw_socket_denied, ipv6_raw_socket_denied. The EPERM requirement needs a seccomp filter on the spawning thread, which children inherit. phlow-checks is `forbid(unsafe_code)`; rustix's seccomp surface could not be verified from this sandbox, and no seccomp crate (seccompiler/libseccomp) is in Cargo.lock. Unblock by approving either a reviewed seccomp dependency or a scoped unsafe boundary. Landlock and network namespaces do not make socket() itself fail.)
