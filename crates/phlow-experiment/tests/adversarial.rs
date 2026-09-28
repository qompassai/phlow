//! Adversarial tests: every attack the scaffolding must reject.
//!
//! Each test attempts one misuse — a forged approval, an escalation, a
//! skipped gate, a weakened check — and asserts the typed rejection. No
//! test weakens a check to pass.

#[path = "common/mod.rs"]
mod common;

use common::{
    RegistryEntry, TEST_EXPIRES_MS, TEST_OPERATOR, complete_evidence, decode_hex, hex,
    manifest_text, ok, operator_record, proposal_params, root_capabilities, signed_operator_record,
    test_agent, test_budget, test_clock, test_consumed, test_keypair, test_keypair_b,
    test_registry, write_registry_file,
};
use phlow_experiment::{
    APPROVAL_TTL_MAX_MS, ArtifactDigest, CheckRun, ConsumedApprovals, EvidenceBundle,
    ExperimentError, HumanApproval, ImprovementProposal, Lifecycle, LifecycleEvent, ManualClock,
    OperatorRegistry, PromotionGate, VerificationOutcome, WorkerRole, parse_suite_manifest,
};

#[test]
fn lifecycle_promoted_cannot_return() {
    // A promoted candidate can never go back to an earlier state.
    let result = Lifecycle::Promoted.transition(LifecycleEvent::WorkspaceCreated);
    assert!(matches!(result, Err(ExperimentError::BadTransition { .. })));
    let result = Lifecycle::Promoted.transition(LifecycleEvent::HumanApproved);
    assert!(matches!(result, Err(ExperimentError::BadTransition { .. })));
}

#[test]
fn lifecycle_terminal_states_reject_everything() {
    for terminal in [Lifecycle::Rejected, Lifecycle::RolledBack] {
        for event in [
            LifecycleEvent::WorkspaceCreated,
            LifecycleEvent::HumanApproved,
            LifecycleEvent::RegressionDetected,
        ] {
            let result = terminal.transition(event);
            assert!(
                matches!(result, Err(ExperimentError::LifecycleTerminal { .. })),
                "{terminal:?} accepted {event:?}"
            );
        }
    }
}

#[test]
fn lifecycle_skip_isolation_rejected() {
    // Proposed -> Tested skips the isolated workspace: denied.
    let result = Lifecycle::Proposed.transition(LifecycleEvent::ChecksComplete);
    assert!(matches!(result, Err(ExperimentError::BadTransition { .. })));
    // Reviewed -> Promoted skips the human: denied.
    let result = Lifecycle::Reviewed.transition(LifecycleEvent::HumanApproved);
    assert!(matches!(result, Err(ExperimentError::BadTransition { .. })));
}

#[test]
fn capability_child_broader_tools_denied() {
    let parent = root_capabilities();
    let result = parent.derive_child(
        vec!["read".to_string(), "shell".to_string()],
        vec!["workspace/".to_string()],
        32,
        524_288,
    );
    assert!(matches!(
        result,
        Err(ExperimentError::CapabilityEscalation { .. })
    ));
}

#[test]
fn capability_child_broader_paths_denied() {
    let parent = root_capabilities();
    let result = parent.derive_child(
        vec!["read".to_string()],
        vec!["workspace/".to_string(), "/etc".to_string()],
        32,
        524_288,
    );
    assert!(matches!(
        result,
        Err(ExperimentError::CapabilityEscalation { .. })
    ));
}

#[test]
fn approval_empty_record_rejected() {
    let result = HumanApproval::from_operator_record("");
    assert!(matches!(
        result,
        Err(ExperimentError::ApprovalRejected { .. })
    ));
}

/// Replaces the value of the `signature_*` line starting with `key` in a
/// v2 record; used to build forgery fixtures.
fn with_sig_value(record: &str, key: &str, value: &str) -> String {
    record
        .lines()
        .map(|line| {
            if line.starts_with(key) {
                format!("{key} {value}")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

/// Reads back the value of the `signature_*` line starting with `key`.
fn sig_value<'a>(record: &'a str, key: &str) -> &'a str {
    record
        .lines()
        .find(|line| line.starts_with(key))
        .unwrap_or_else(|| panic!("test record missing {key}"))
        .split_once(':')
        .map(|(_, value)| value.trim())
        .unwrap_or_else(|| panic!("test record has malformed {key}"))
}

/// The standard genuine promotion inputs: a valid proposal, a genuine
/// dual-signed approval, complete evidence, a distinct acting-agent
/// identity, a live clock, the test registry, and a fresh replay store.
#[allow(clippy::type_complexity)]
fn genuine_inputs() -> (
    ImprovementProposal,
    HumanApproval,
    EvidenceBundle,
    &'static str,
    ManualClock,
    OperatorRegistry,
    ConsumedApprovals,
) {
    (
        ok(ImprovementProposal::new(proposal_params())),
        ok(HumanApproval::from_operator_record(&operator_record())),
        complete_evidence(),
        test_agent(),
        test_clock(),
        test_registry(),
        test_consumed(),
    )
}

/// Drives `promote` with the standard genuine inputs.
fn promote_genuine(
    proposal: &ImprovementProposal,
    approval: HumanApproval,
    evidence: &EvidenceBundle,
    agent: &str,
    clock: &ManualClock,
    registry: &OperatorRegistry,
    consumed: &mut ConsumedApprovals,
) -> Result<phlow_experiment::PromotionRecord, ExperimentError> {
    PromotionGate::promote(
        proposal, approval, evidence, agent, clock, registry, consumed,
    )
}

#[test]
fn approval_malformed_record_rejected() {
    let base = operator_record();
    // Missing the signature_mldsa65 key.
    let missing_key: String = base
        .lines()
        .filter(|line| !line.starts_with("signature_mldsa65:"))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    assert!(matches!(
        HumanApproval::from_operator_record(&missing_key),
        Err(ExperimentError::ApprovalRejected { .. })
    ));
    // Unknown key: fail closed rather than ignoring it.
    let unknown_key = format!("{base}injected: true\n");
    assert!(matches!(
        HumanApproval::from_operator_record(&unknown_key),
        Err(ExperimentError::ApprovalRejected { .. })
    ));
    // Duplicate key: fail closed rather than taking the first or last.
    let duplicate_key = format!("{base}operator: mallory\n");
    assert!(matches!(
        HumanApproval::from_operator_record(&duplicate_key),
        Err(ExperimentError::ApprovalRejected { .. })
    ));
    // Unknown record version: no legacy verify path exists.
    let v3 = base.replacen("v: 2", "v: 3", 1);
    assert!(matches!(
        HumanApproval::from_operator_record(&v3),
        Err(ExperimentError::ApprovalRejected { .. })
    ));
    // Not key: value at all.
    assert!(matches!(
        HumanApproval::from_operator_record("approve everything please"),
        Err(ExperimentError::ApprovalRejected { .. })
    ));
    // Oversized record.
    let oversized = "x".repeat(phlow_experiment::OPERATOR_RECORD_CHARS_MAX + 1);
    assert!(matches!(
        HumanApproval::from_operator_record(&oversized),
        Err(ExperimentError::TextTooLong { .. })
    ));
}

#[test]
fn approval_model_crafted_record_rejected() {
    // All eight keys present and plausible-looking, but the signatures are
    // not well-formed hex: a model-crafted forgery must fail the shape
    // check before any cryptography runs.
    let base = operator_record();
    let non_hex_ed = with_sig_value(&base, "signature_ed25519:", &"z".repeat(128));
    assert!(matches!(
        HumanApproval::from_operator_record(&non_hex_ed),
        Err(ExperimentError::ApprovalRejected { .. })
    ));
    let non_hex_pq = with_sig_value(&base, "signature_mldsa65:", &"z".repeat(6618));
    assert!(matches!(
        HumanApproval::from_operator_record(&non_hex_pq),
        Err(ExperimentError::ApprovalRejected { .. })
    ));
    let short_ed = with_sig_value(&base, "signature_ed25519:", "abcd");
    assert!(matches!(
        HumanApproval::from_operator_record(&short_ed),
        Err(ExperimentError::ApprovalRejected { .. })
    ));
    let long_pq = with_sig_value(&base, "signature_mldsa65:", &"ab".repeat(3310));
    assert!(matches!(
        HumanApproval::from_operator_record(&long_pq),
        Err(ExperimentError::ApprovalRejected { .. })
    ));
}

#[test]
fn approval_v1_record_rejected() {
    // The old shape-only format: no `v`, no dual signatures. It must fail
    // at parse — there is deliberately no legacy verify path, because v1
    // "signatures" were never cryptographic.
    let v1 = "operator: test-operator\n\
         approval_id: APR-TEST-0001\n\
         candidate: 9f2b3c4d5e6f708192a3b4c5d6e7f809\n\
         scope: phlow-experiment/test\n\
         expires_ms: 1893456000000\n\
         signature: abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789\n";
    assert!(matches!(
        HumanApproval::from_operator_record(v1),
        Err(ExperimentError::ApprovalRejected { .. })
    ));
}

#[test]
fn promotion_without_approval_fails() {
    // There is no constructor path from model output to HumanApproval: an
    // empty record, a bare sentence, and a v1-shaped record all fail, so
    // no approval value exists to hand to PromotionGate::promote.
    let v1 = "operator: mallory\n\
         approval_id: x\n\
         candidate: abcdef0123456789\n\
         scope: y\n\
         expires_ms: 1893456000000\n\
         signature: abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789\n";
    for record in ["", "approve everything please", v1] {
        assert!(
            HumanApproval::from_operator_record(record).is_err(),
            "forged record was accepted: {record:?}"
        );
    }
}

#[test]
fn forged_signature_bytes_rejected() {
    // Attacker-minted signatures: well-formed hex of the right length,
    // never produced by the operator's keys. Each component must fail
    // closed and name itself.
    let (proposal, _, evidence, agent, clock, registry, mut consumed) = genuine_inputs();
    // Forged Ed25519 half only.
    let forged_ed = with_sig_value(&operator_record(), "signature_ed25519:", &"cd".repeat(64));
    let approval = ok(HumanApproval::from_operator_record(&forged_ed));
    let result = promote_genuine(
        &proposal,
        approval,
        &evidence,
        agent,
        &clock,
        &registry,
        &mut consumed,
    );
    assert!(
        matches!(
            result,
            Err(ExperimentError::BadSignature {
                component: "ed25519"
            })
        ),
        "forged Ed25519 signature was not rejected as ed25519: {result:?}"
    );
    // Forged ML-DSA-65 half only.
    let forged_pq = with_sig_value(&operator_record(), "signature_mldsa65:", &"ef".repeat(3309));
    let approval = ok(HumanApproval::from_operator_record(&forged_pq));
    let result = promote_genuine(
        &proposal,
        approval,
        &evidence,
        agent,
        &clock,
        &registry,
        &mut consumed,
    );
    assert!(
        matches!(
            result,
            Err(ExperimentError::BadSignature {
                component: "mldsa65"
            })
        ),
        "forged ML-DSA-65 signature was not rejected as mldsa65: {result:?}"
    );
}

#[test]
fn wrong_operator_key_rejected() {
    // A genuine dual signature — but from a DIFFERENT operator's keypair.
    // The registry pins keys per operator, so this is forgery, not a
    // parse error: the signature check must fail, not the lookup.
    let record = signed_operator_record(
        TEST_OPERATOR,
        "APR-TEST-0002",
        "9f2b3c4d5e6f708192a3b4c5d6e7f809",
        "phlow-experiment/test",
        TEST_EXPIRES_MS,
        test_keypair_b(),
    );
    let (proposal, _, evidence, agent, clock, registry, mut consumed) = genuine_inputs();
    let approval = ok(HumanApproval::from_operator_record(&record));
    let result = promote_genuine(
        &proposal,
        approval,
        &evidence,
        agent,
        &clock,
        &registry,
        &mut consumed,
    );
    assert!(
        matches!(result, Err(ExperimentError::BadSignature { .. })),
        "wrong-key signature was not rejected: {result:?}"
    );
}

#[test]
fn tampered_scope_rejected() {
    // Flipping `scope` after signing breaks both signatures: the
    // canonical bytes no longer match what was signed.
    let record = operator_record().replacen(
        "scope: phlow-experiment/test",
        "scope: phlow-experiment/pwned",
        1,
    );
    let (proposal, _, evidence, agent, clock, registry, mut consumed) = genuine_inputs();
    let approval = ok(HumanApproval::from_operator_record(&record));
    let result = promote_genuine(
        &proposal,
        approval,
        &evidence,
        agent,
        &clock,
        &registry,
        &mut consumed,
    );
    assert!(
        matches!(result, Err(ExperimentError::BadSignature { .. })),
        "tampered scope was not rejected as forgery: {result:?}"
    );
}

#[test]
fn tampered_expires_rejected() {
    // Flipping `expires_ms` after signing must read as FORGERY, not as
    // expiry: the signature covers the expiry, and it no longer matches.
    // The tampered value stays inside the TTL window so the TTL check
    // cannot mask the signature failure.
    let tampered_ms = TEST_EXPIRES_MS + 1_000_000;
    let record = operator_record().replacen(
        &format!("expires_ms: {TEST_EXPIRES_MS}"),
        &format!("expires_ms: {tampered_ms}"),
        1,
    );
    let (proposal, _, evidence, agent, clock, registry, mut consumed) = genuine_inputs();
    let approval = ok(HumanApproval::from_operator_record(&record));
    let result = promote_genuine(
        &proposal,
        approval,
        &evidence,
        agent,
        &clock,
        &registry,
        &mut consumed,
    );
    assert!(
        matches!(result, Err(ExperimentError::BadSignature { .. })),
        "tampered expires_ms was not rejected as forgery: {result:?}"
    );
    assert!(
        !matches!(result, Err(ExperimentError::ApprovalExpired)),
        "tampered expires_ms must not read as expiry"
    );
}

#[test]
fn replayed_approval_rejected() {
    // A genuine record promotes once; the identical record parsed again
    // (a fresh token with the same approval_id) must be rejected: single
    // use closes the re-parse hole that by-value consumption alone leaves.
    let (proposal, approval, evidence, agent, clock, registry, mut consumed) = genuine_inputs();
    let first = promote_genuine(
        &proposal,
        approval,
        &evidence,
        agent,
        &clock,
        &registry,
        &mut consumed,
    );
    assert!(first.is_ok(), "genuine first promotion failed: {first:?}");
    let replay = ok(HumanApproval::from_operator_record(&operator_record()));
    let second = promote_genuine(
        &proposal,
        replay,
        &evidence,
        agent,
        &clock,
        &registry,
        &mut consumed,
    );
    match second {
        Err(ExperimentError::ApprovalReplayed { approval_id }) => {
            assert_eq!(approval_id, "APR-TEST-0001");
        }
        other => panic!("replayed approval was not rejected as replay: {other:?}"),
    }
}

#[test]
fn expired_approval_rejected() {
    // At and after expires_ms the record is dead, even with a genuine
    // dual signature: expiry is enforced against the trusted clock.
    let (proposal, _, evidence, agent, _, registry, mut consumed) = genuine_inputs();
    for now_ms in [TEST_EXPIRES_MS, TEST_EXPIRES_MS + 3_600_000] {
        let clock = ManualClock::new(now_ms);
        let approval = ok(HumanApproval::from_operator_record(&operator_record()));
        let result = promote_genuine(
            &proposal,
            approval,
            &evidence,
            agent,
            &clock,
            &registry,
            &mut consumed,
        );
        assert!(
            matches!(result, Err(ExperimentError::ApprovalExpired)),
            "expired approval accepted at now_ms={now_ms}: {result:?}"
        );
    }
}

#[test]
fn expiry_beyond_max_ttl_rejected() {
    // An expiry past the policy cap is rejected even though the
    // signatures are genuine: it bounds the blast radius of a stolen
    // approval. The cap is a named constant, easy to change when Matt
    // sets operational policy.
    let now_ms = 1_750_000_000_000u64;
    let expires_ms = now_ms + APPROVAL_TTL_MAX_MS + 1;
    let record = signed_operator_record(
        TEST_OPERATOR,
        "APR-TEST-0003",
        "9f2b3c4d5e6f708192a3b4c5d6e7f809",
        "phlow-experiment/test",
        expires_ms,
        test_keypair(),
    );
    let (proposal, _, evidence, agent, _, registry, mut consumed) = genuine_inputs();
    let approval = ok(HumanApproval::from_operator_record(&record));
    let clock = ManualClock::new(now_ms);
    let result = promote_genuine(
        &proposal,
        approval,
        &evidence,
        agent,
        &clock,
        &registry,
        &mut consumed,
    );
    assert!(
        matches!(result, Err(ExperimentError::ExpiryBeyondMaxTtl)),
        "absurd expiry was not rejected: {result:?}"
    );
}

#[test]
fn self_approval_rejected() {
    // The exact banked hole: the agent names ITSELF as the operator and
    // hands its own approval to the gate. The acting-agent identity is an
    // explicit parameter, so the comparison cannot be dodged.
    let (proposal, approval, evidence, _, clock, registry, mut consumed) = genuine_inputs();
    let result = promote_genuine(
        &proposal,
        approval,
        &evidence,
        TEST_OPERATOR,
        &clock,
        &registry,
        &mut consumed,
    );
    match result {
        Err(ExperimentError::SelfApproval { operator }) => {
            assert_eq!(operator, TEST_OPERATOR);
        }
        other => panic!("self-approval was not rejected: {other:?}"),
    }
    // An empty acting-agent identity is rejected before any comparison:
    // the check must not be bypassable with an empty string.
    let approval = ok(HumanApproval::from_operator_record(&operator_record()));
    let result = promote_genuine(
        &proposal,
        approval,
        &evidence,
        "",
        &clock,
        &registry,
        &mut consumed,
    );
    assert!(
        matches!(result, Err(ExperimentError::ApprovalRejected { .. })),
        "empty acting agent was not rejected: {result:?}"
    );
}

#[test]
fn unknown_operator_rejected() {
    // "mallory" is not enrolled. The registry lookup fails before any
    // cryptography runs — and the error names the operator, not a
    // signature failure, so a typo is distinguishable from a forgery.
    let record = signed_operator_record(
        "mallory",
        "APR-TEST-0004",
        "9f2b3c4d5e6f708192a3b4c5d6e7f809",
        "phlow-experiment/test",
        TEST_EXPIRES_MS,
        test_keypair(),
    );
    let (proposal, _, evidence, agent, clock, registry, mut consumed) = genuine_inputs();
    let approval = ok(HumanApproval::from_operator_record(&record));
    let result = promote_genuine(
        &proposal,
        approval,
        &evidence,
        agent,
        &clock,
        &registry,
        &mut consumed,
    );
    match result {
        Err(ExperimentError::UnknownOperator { operator }) => {
            assert_eq!(operator, "mallory");
        }
        other => panic!("unknown operator was not rejected: {other:?}"),
    }
}

#[test]
fn revoked_operator_rejected() {
    // Revocation is a one-line registry edit and takes effect on the next
    // promote: a revoked operator's genuine signatures no longer verify
    // into a promotion.
    let path = write_registry_file(&[RegistryEntry {
        name: TEST_OPERATOR,
        keypair: test_keypair(),
        revoked: true,
    }]);
    let registry = ok(OperatorRegistry::load(&path));
    let (proposal, approval, evidence, agent, clock, _, mut consumed) = genuine_inputs();
    let result = promote_genuine(
        &proposal,
        approval,
        &evidence,
        agent,
        &clock,
        &registry,
        &mut consumed,
    );
    assert!(
        matches!(result, Err(ExperimentError::RevokedOperator { .. })),
        "revoked operator was not rejected: {result:?}"
    );
}

#[test]
fn cross_record_signature_mix_rejected() {
    // Nested binding: the PQ signature covers `canonical || ed_sig`, so an
    // Ed25519 signature spliced from another genuine record breaks the PQ
    // half — components cannot be mixed across records.
    let record_a = signed_operator_record(
        TEST_OPERATOR,
        "APR-TEST-0005",
        "9f2b3c4d5e6f708192a3b4c5d6e7f809",
        "phlow-experiment/test",
        TEST_EXPIRES_MS,
        test_keypair(),
    );
    let record_b = signed_operator_record(
        TEST_OPERATOR,
        "APR-TEST-0006",
        "9f2b3c4d5e6f708192a3b4c5d6e7f809",
        "phlow-experiment/test",
        TEST_EXPIRES_MS,
        test_keypair(),
    );
    let ed_sig_a = sig_value(&record_a, "signature_ed25519:").to_string();
    let mixed = with_sig_value(&record_b, "signature_ed25519:", &ed_sig_a);
    let (proposal, _, evidence, agent, clock, registry, mut consumed) = genuine_inputs();
    let approval = ok(HumanApproval::from_operator_record(&mixed));
    let result = promote_genuine(
        &proposal,
        approval,
        &evidence,
        agent,
        &clock,
        &registry,
        &mut consumed,
    );
    assert!(
        matches!(result, Err(ExperimentError::BadSignature { .. })),
        "cross-record signature mix was not rejected: {result:?}"
    );
}

#[test]
fn malleated_ed25519_signature_rejected() {
    // Set the high bit of S: the signature becomes non-canonical, and
    // verify_strict must reject what a lax verifier would accept.
    let record = operator_record();
    let mut ed_bytes = decode_hex(sig_value(&record, "signature_ed25519:"));
    let last = ed_bytes.len() - 1;
    ed_bytes[last] |= 0x80;
    let malleated = with_sig_value(&record, "signature_ed25519:", &hex(&ed_bytes));
    let (proposal, _, evidence, agent, clock, registry, mut consumed) = genuine_inputs();
    let approval = ok(HumanApproval::from_operator_record(&malleated));
    let result = promote_genuine(
        &proposal,
        approval,
        &evidence,
        agent,
        &clock,
        &registry,
        &mut consumed,
    );
    assert!(
        matches!(
            result,
            Err(ExperimentError::BadSignature {
                component: "ed25519"
            })
        ),
        "malleated Ed25519 signature was not rejected as ed25519: {result:?}"
    );
}

#[test]
fn promotion_incomplete_evidence_fails_closed() {
    // One failing check poisons the bundle: promotion fails closed. The
    // genuine dual signature passes the crypto checks first, so the
    // evidence check is what surfaces.
    let checks = vec![ok(CheckRun::new(
        "tests",
        vec!["cargo".to_string(), "test".to_string()],
        false,
    ))];
    let artifacts = vec![ok(ArtifactDigest::new("candidate.diff", "9f2b3c4d"))];
    let verification = ok(VerificationOutcome::new(
        true,
        vec!["src/lib.rs".to_string()],
    ));
    let evidence = ok(EvidenceBundle::new(checks, artifacts, verification));
    assert!(!evidence.is_complete());
    let (proposal, approval, _, agent, clock, registry, mut consumed) = genuine_inputs();
    let result = promote_genuine(
        &proposal,
        approval,
        &evidence,
        agent,
        &clock,
        &registry,
        &mut consumed,
    );
    assert!(matches!(
        result,
        Err(ExperimentError::IncompleteEvidence { .. })
    ));
}

#[test]
fn promotion_oversized_diff_summary_rejected() {
    let mut params = proposal_params();
    params.candidate_diff_summary = "x".repeat(4_097);
    let result = ImprovementProposal::new(params);
    assert!(matches!(result, Err(ExperimentError::TextTooLong { .. })));
}

#[test]
fn budget_limits_fail_closed() {
    let mut tracker = test_budget();
    // Spending more tool calls than remain fails closed.
    let result = tracker.consume(65, 0);
    assert!(matches!(
        result,
        Err(ExperimentError::BudgetExhausted { .. })
    ));
    // A passed deadline fails closed even for a zero-cost consume.
    tracker.set_now_ms(300_001);
    let result = tracker.consume(0, 0);
    assert!(matches!(result, Err(ExperimentError::DeadlineExceeded)));
}

#[test]
fn malformed_manifests_rejected() {
    // Not TOML at all.
    let result = parse_suite_manifest("not toml [[[", "manifests/suites.toml");
    assert!(matches!(
        result,
        Err(ExperimentError::ManifestInvalid { .. })
    ));
    // Unknown key: fail closed, naming the file and key.
    let unknown_key = manifest_text("suites.toml") + "\nself_approve = true\n";
    let result = parse_suite_manifest(&unknown_key, "manifests/suites.toml");
    match result {
        Err(ExperimentError::ManifestInvalid { file, key, .. }) => {
            assert_eq!(file, "manifests/suites.toml");
            assert!(key.contains("self_approve"), "unexpected key: {key}");
        }
        other => panic!("expected ManifestInvalid, got {other:?}"),
    }
    // Wrong schema version.
    let wrong_version =
        manifest_text("suites.toml").replacen("schema_version = 1", "schema_version = 2", 1);
    let result = parse_suite_manifest(&wrong_version, "manifests/suites.toml");
    assert!(matches!(
        result,
        Err(ExperimentError::ManifestInvalid { .. })
    ));
}

#[test]
fn no_role_has_production_write() {
    // Asserted for every role: no worker may write production state, and
    // the only scoped write is the candidate workspace.
    for role in WorkerRole::all() {
        assert!(
            !role.can_write_production(),
            "{:?} claims production write access",
            role
        );
    }
    // The adversary explicitly has no production write: it attacks with
    // controlled fixtures, not production mutation.
    assert_eq!(
        WorkerRole::Adversary.write_access(),
        phlow_experiment::WriteAccess::NoProductionWrite
    );
}
