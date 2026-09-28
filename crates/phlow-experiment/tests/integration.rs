//! Integration tests: the skeleton flow end to end, the manifest → types →
//! record round trip, a fully-gated promotion, scheduler admission, and
//! evaluator stage order.

#[path = "common/mod.rs"]
mod common;

use common::{
    RegistryEntry, TEST_EXPIRES_MS, TEST_OPERATOR, complete_evidence, eval_text, hex, ok,
    operator_record, proposal_params, signed_operator_record, test_agent, test_budget, test_clock,
    test_consumed, test_keypair, test_keypair_b, test_node, test_registry, test_scheduler,
    write_registry_file,
};
use phlow_experiment::{
    CheckRecord, Clock as _, ConsumedApprovals, EvaluationRecord, Evaluator, ExperimentError,
    HumanApproval, ImprovementProposal, Lifecycle, LifecycleEvent, ManualClock, NodeId, NodeState,
    OperatorRegistry, PromotionGate, RecordParams, SystemClock, parse_task_manifest,
};

#[test]
fn full_lifecycle_to_awaiting_human_then_denied() {
    let to_awaiting = || {
        let state = ok(Lifecycle::Proposed.transition(LifecycleEvent::WorkspaceCreated));
        let state = ok(state.transition(LifecycleEvent::ChecksComplete));
        let state = ok(state.transition(LifecycleEvent::EvaluationComplete));
        ok(state.transition(LifecycleEvent::GatesSatisfied))
    };
    assert_eq!(to_awaiting(), Lifecycle::AwaitingHuman);
    // Denied approval rejects.
    let rejected = ok(to_awaiting().transition(LifecycleEvent::ApprovalDenied));
    assert_eq!(rejected, Lifecycle::Rejected);
    // Expired approval rejects too.
    let rejected = ok(to_awaiting().transition(LifecycleEvent::ApprovalExpired));
    assert_eq!(rejected, Lifecycle::Rejected);
}

#[test]
fn manifest_to_record_round_trip() {
    let manifest = ok(parse_task_manifest(
        &eval_text("evals/public/rust-cli-parse-001.toml"),
        "evals/public/rust-cli-parse-001.toml",
    ));
    assert_eq!(manifest.id, "rust-cli-parse-001");
    assert_eq!(manifest.checks.len(), 3);
    assert!(manifest.checks.iter().all(|c| c.required));

    let mut record = ok(EvaluationRecord::new(RecordParams {
        experiment_id: manifest.id.clone(),
        baseline_revision: "abc123".to_string(),
        workspace_digest: "workspace-digest-001".to_string(),
        operator_config_digest: "operator-config-digest-001".to_string(),
        model_ids: vec!["test-model-1".to_string()],
        toolchain_versions: vec!["rustc 1.90.0".to_string()],
        limits: vec!["budgets.toml defaults".to_string()],
        stop_reason: "trial complete".to_string(),
    }));
    // Constructor defaults: nothing verified, nothing eligible, nothing approved.
    assert!(!record.verified());
    assert!(!record.promotion_eligible());
    assert!(!record.human_approved());
    assert_eq!(record.schema_version(), 1);

    ok(record.record_event("task admitted"));
    for check in &manifest.checks {
        ok(record.record_check(ok(CheckRecord::new(
            &check.name,
            check.argv.clone(),
            true,
            check.required,
        ))));
    }
    assert_eq!(record.check_count(), 3);

    let json = ok(record.to_json());
    for expected in [
        "\"schema_version\": 1",
        "\"experiment_id\": \"rust-cli-parse-001\"",
        "\"status\": \"experimental\"",
        "\"verified\": false",
        "\"eligible\": false",
        "\"human_approved\": false",
        "\"stop_reason\": \"trial complete\"",
    ] {
        assert!(json.contains(expected), "record JSON missing {expected}");
    }
}

#[test]
fn promotion_with_valid_approval_and_evidence() {
    let proposal = ok(ImprovementProposal::new(proposal_params()));
    let approval = ok(HumanApproval::from_operator_record(&operator_record()));
    let evidence = complete_evidence();
    let record = ok(PromotionGate::promote(
        &proposal,
        approval,
        &evidence,
        test_agent(),
        &test_clock(),
        &test_registry(),
        &mut test_consumed(),
    ));
    assert_eq!(record.operator(), TEST_OPERATOR);
    assert_eq!(record.approval_id(), "APR-TEST-0001");
    assert_eq!(record.rollback_target(), "abc123");
    assert_eq!(record.evidence_checks(), 1);
}

#[test]
fn distinct_approval_id_promotes_after_first() {
    // The replay store keys on approval_id: a second, distinct approval
    // from the same operator promotes fine — no false replay.
    let proposal = ok(ImprovementProposal::new(proposal_params()));
    let evidence = complete_evidence();
    let agent = test_agent();
    let clock = test_clock();
    let registry = test_registry();
    let mut consumed = test_consumed();
    for (id, label) in [("APR-TEST-0010", "first"), ("APR-TEST-0011", "second")] {
        let record_text = signed_operator_record(
            TEST_OPERATOR,
            id,
            "9f2b3c4d5e6f708192a3b4c5d6e7f809",
            "phlow-experiment/test",
            TEST_EXPIRES_MS,
            test_keypair(),
        );
        let approval = ok(HumanApproval::from_operator_record(&record_text));
        let record = ok(PromotionGate::promote(
            &proposal,
            approval,
            &evidence,
            agent,
            &clock,
            &registry,
            &mut consumed,
        ));
        assert_eq!(record.approval_id(), id, "{label} promotion id mismatch");
    }
}

#[test]
fn second_enrolled_operator_promotes() {
    // The registry holds more than one operator; each operator's genuine
    // approvals verify against their own pinned keys.
    let path = write_registry_file(&[
        RegistryEntry {
            name: TEST_OPERATOR,
            keypair: test_keypair(),
            revoked: false,
        },
        RegistryEntry {
            name: "op-bravo",
            keypair: test_keypair_b(),
            revoked: false,
        },
    ]);
    let registry = ok(OperatorRegistry::load(&path));
    let proposal = ok(ImprovementProposal::new(proposal_params()));
    let evidence = complete_evidence();
    let record_text = signed_operator_record(
        "op-bravo",
        "APR-TEST-0012",
        "9f2b3c4d5e6f708192a3b4c5d6e7f809",
        "phlow-experiment/test",
        TEST_EXPIRES_MS,
        test_keypair_b(),
    );
    let approval = ok(HumanApproval::from_operator_record(&record_text));
    let record = ok(PromotionGate::promote(
        &proposal,
        approval,
        &evidence,
        test_agent(),
        &test_clock(),
        &registry,
        &mut test_consumed(),
    ));
    assert_eq!(record.operator(), "op-bravo");
}

#[test]
fn max_length_fields_promote() {
    // Bounds are inclusive: 64-char operator/approval_id, 128-hex
    // candidate, 128-char scope all verify and promote.
    let operator = "o".repeat(64);
    let record_text = signed_operator_record(
        &operator,
        &"a".repeat(64),
        &"c".repeat(128),
        &"s".repeat(128),
        TEST_EXPIRES_MS,
        test_keypair(),
    );
    let path = write_registry_file(&[RegistryEntry {
        name: &operator,
        keypair: test_keypair(),
        revoked: false,
    }]);
    let registry = ok(OperatorRegistry::load(&path));
    let proposal = ok(ImprovementProposal::new(proposal_params()));
    let approval = ok(HumanApproval::from_operator_record(&record_text));
    let record = ok(PromotionGate::promote(
        &proposal,
        approval,
        &complete_evidence(),
        test_agent(),
        &test_clock(),
        &registry,
        &mut test_consumed(),
    ));
    assert_eq!(record.operator(), operator);
}

#[test]
fn manual_clock_determinism() {
    // One millisecond before expiry the genuine approval promotes; at
    // expiry it is dead. Deterministic clocks make the boundary exact.
    let proposal = ok(ImprovementProposal::new(proposal_params()));
    let evidence = complete_evidence();
    let agent = test_agent();
    let registry = test_registry();
    let live = ManualClock::new(TEST_EXPIRES_MS - 1);
    let approval = ok(HumanApproval::from_operator_record(&operator_record()));
    let promoted = PromotionGate::promote(
        &proposal,
        approval,
        &evidence,
        agent,
        &live,
        &registry,
        &mut test_consumed(),
    );
    assert!(promoted.is_ok(), "approval just before expiry rejected");
    let dead = ManualClock::new(TEST_EXPIRES_MS);
    let approval = ok(HumanApproval::from_operator_record(&operator_record()));
    let result = PromotionGate::promote(
        &proposal,
        approval,
        &evidence,
        agent,
        &dead,
        &registry,
        &mut test_consumed(),
    );
    assert!(matches!(result, Err(ExperimentError::ApprovalExpired)));
}

#[test]
fn system_clock_reports_plausible_time() {
    // The production clock reads the process wall clock; it must be
    // plausible (not saturated to the fail-closed zero, not far future).
    // No exact value is asserted — wall clocks move.
    let now_ms = SystemClock.now_ms();
    assert!(
        now_ms > 1_700_000_000_000,
        "system clock implausibly small: {now_ms}"
    );
    assert!(
        now_ms < 2_000_000_000_000,
        "system clock implausibly large: {now_ms}"
    );
}

#[test]
fn canonical_tamper_fails_signature() {
    // Regression pin for the combiner: both halves sign the same
    // canonical bytes, so flipping one byte breaks the dual signature.
    let record = operator_record().replacen(
        "scope: phlow-experiment/test",
        "scope: phlow-experiment/tesu",
        1,
    );
    let proposal = ok(ImprovementProposal::new(proposal_params()));
    let approval = ok(HumanApproval::from_operator_record(&record));
    let result = PromotionGate::promote(
        &proposal,
        approval,
        &complete_evidence(),
        test_agent(),
        &test_clock(),
        &test_registry(),
        &mut test_consumed(),
    );
    assert!(
        matches!(result, Err(ExperimentError::BadSignature { .. })),
        "one-byte canonical tamper was not rejected: {result:?}"
    );
}

#[test]
fn consumed_approvals_evict_expired() {
    // Expired ids leave the replay store: it stays bounded by the TTL
    // window instead of growing forever.
    let mut consumed = test_consumed();
    consumed.insert("APR-OLD".to_string(), 1_000);
    consumed.insert("APR-NEW".to_string(), 9_000);
    assert!(consumed.contains("APR-OLD"));
    consumed.evict_expired(2_000);
    assert!(!consumed.contains("APR-OLD"));
    assert!(consumed.contains("APR-NEW"));
}

#[test]
fn consumed_approvals_jsonl_round_trip() {
    // The replay store persists as append-only JSONL; loading drops
    // expired entries so a restart cannot resurrect a dead approval.
    let mut consumed = test_consumed();
    consumed.insert("APR-A".to_string(), 5_000);
    consumed.insert("APR-B".to_string(), 1_000);
    let path = std::env::temp_dir().join(format!(
        "phlow-test-consumed-{}-{}.jsonl",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    ));
    ok(consumed.save(&path));
    let loaded = ok(ConsumedApprovals::load(&path, 2_000));
    assert!(loaded.contains("APR-A"));
    assert!(!loaded.contains("APR-B"));
    std::fs::remove_file(&path).unwrap_or(());
}

#[test]
fn registry_toml_round_trip() {
    // TOML load → lookup → pinned keys; the revoked entry and unknown
    // names resolve to their distinct errors.
    let path = write_registry_file(&[
        RegistryEntry {
            name: TEST_OPERATOR,
            keypair: test_keypair(),
            revoked: false,
        },
        RegistryEntry {
            name: "op-revoked",
            keypair: test_keypair_b(),
            revoked: true,
        },
    ]);
    let registry = ok(OperatorRegistry::load(&path));
    let key = ok(registry.lookup(TEST_OPERATOR));
    assert_eq!(key.ed25519_pk, test_keypair().ed_pk);
    assert_eq!(key.mldsa65_pk, test_keypair().pq_pk);
    assert!(!key.revoked);
    assert!(matches!(
        registry.lookup("op-revoked"),
        Err(ExperimentError::RevokedOperator { .. })
    ));
    assert!(matches!(
        registry.lookup("nobody"),
        Err(ExperimentError::UnknownOperator { .. })
    ));
}

#[test]
fn registry_fingerprint_mismatch_fails_closed() {
    // The fingerprint is the enrollment-ceremony check: a registry entry
    // whose keys do not hash to the recorded fingerprint is tampered and
    // the whole file fails closed.
    let print = "00".repeat(32);
    let toml = format!(
        "[operators.\"{TEST_OPERATOR}\"]\n\
         ed25519_pubkey = \"{}\"\n\
         mldsa65_pubkey = \"{}\"\n\
         fingerprint = \"{print}\"\n\
         enrolled_ms = 1750000000000\n\
         revoked = false\n",
        hex(&test_keypair().ed_pk),
        hex(&test_keypair().pq_pk),
    );
    let dir =
        std::env::temp_dir().join(format!("phlow-test-registry-badfp-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap_or(());
    let path = dir.join("operators.toml");
    std::fs::write(&path, &toml).unwrap_or(());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap_or(());
    }
    let result = OperatorRegistry::load(&path);
    assert!(
        matches!(
            result,
            Err(ExperimentError::ApprovalRejected { reason }) if reason == "registry fingerprint mismatch"
        ),
        "tampered registry fingerprint was not rejected: {result:?}"
    );
}

#[test]
fn registry_rejects_world_readable_file() {
    // The registry is a trust root: group/world-readable permissions fail
    // closed at load. Tiger Style: fail closed on the trust root.
    let path = write_registry_file(&[RegistryEntry {
        name: TEST_OPERATOR,
        keypair: test_keypair(),
        revoked: false,
    }]);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
            .unwrap_or_else(|error| panic!("cannot chmod test registry: {error}"));
    }
    let result = OperatorRegistry::load(&path);
    #[cfg(unix)]
    assert!(
        matches!(result, Err(ExperimentError::ApprovalRejected { .. })),
        "world-readable registry was not rejected: {result:?}"
    );
    #[cfg(not(unix))]
    let _ = result;
}

#[test]
fn registry_rejects_malformed_entries() {
    // Each malformed entry fails the load: short keys, non-hex keys, a
    // missing field, a wrongly-typed field. Fail closed, never partial.
    let good_ed = hex(&test_keypair().ed_pk);
    let good_pq = hex(&test_keypair().pq_pk);
    let good_fp = {
        use sha2::Digest as _;
        let mut hasher = sha2::Sha256::new();
        hasher.update(test_keypair().ed_pk);
        hasher.update(test_keypair().pq_pk);
        hex(&hasher.finalize())
    };
    let cases: &[(&str, String)] = &[
        (
            "short ed25519 key",
            format!(
                "[operators.op-x]\ned25519_pubkey = \"abcd\"\nmldsa65_pubkey = \"{good_pq}\"\n\
                 fingerprint = \"{good_fp}\"\nenrolled_ms = 1\nrevoked = false\n"
            ),
        ),
        (
            "non-hex pq key",
            format!(
                "[operators.op-x]\ned25519_pubkey = \"{good_ed}\"\nmldsa65_pubkey = \"{}\"\n\
                 fingerprint = \"{good_fp}\"\nenrolled_ms = 1\nrevoked = false\n",
                "z".repeat(3904),
            ),
        ),
        (
            "missing revoked",
            format!(
                "[operators.op-x]\ned25519_pubkey = \"{good_ed}\"\nmldsa65_pubkey = \"{good_pq}\"\n\
                 fingerprint = \"{good_fp}\"\nenrolled_ms = 1\n"
            ),
        ),
        (
            "wrongly typed enrolled_ms",
            format!(
                "[operators.op-x]\ned25519_pubkey = \"{good_ed}\"\nmldsa65_pubkey = \"{good_pq}\"\n\
                 fingerprint = \"{good_fp}\"\nenrolled_ms = \"yesterday\"\nrevoked = false\n"
            ),
        ),
    ];
    for (label, toml) in cases {
        let dir = std::env::temp_dir().join(format!(
            "phlow-test-registry-malformed-{}-{}",
            std::process::id(),
            label.replace(' ', "_"),
        ));
        std::fs::create_dir_all(&dir).unwrap_or(());
        let path = dir.join("operators.toml");
        std::fs::write(&path, toml).unwrap_or(());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap_or(());
        }
        assert!(
            OperatorRegistry::load(&path).is_err(),
            "malformed registry accepted ({label})"
        );
    }
}

#[test]
fn scheduler_admit_publish_flow() {
    let mut scheduler = test_scheduler();
    ok(scheduler.admit(test_node("n1")));
    assert_eq!(scheduler.queue_len(), 1);
    let node_id = ok(NodeId::new("n1"));
    ok(scheduler.publish_result(&node_id, 0, "result-digest-1", NodeState::Succeeded));
    assert_eq!(scheduler.queue_len(), 0);
    assert_eq!(scheduler.published_count(), 1);
    let node = ok(scheduler
        .node(&node_id)
        .ok_or(ExperimentError::UnknownNode {
            id: "n1".to_string(),
        }));
    assert_eq!(node.state(), NodeState::Succeeded);
    assert_eq!(node.result_digest(), Some("result-digest-1"));
}

#[test]
fn evaluator_stage_order() {
    let mut evaluator = Evaluator::new(test_budget());
    // Out-of-order stages are rejected.
    let early = evaluator.prepare();
    assert!(matches!(early, Err(ExperimentError::BadStageOrder { .. })));
    // In order: validate -> prepare -> execute -> verify -> review -> promote.
    ok(evaluator.validate());
    ok(evaluator.prepare());
    ok(evaluator.execute(1, 16));
    let verified = ok(evaluator.verify(&complete_evidence()));
    assert!(verified);
    ok(evaluator.review());
    ok(evaluator.promote());
    // Promote is terminal: a second call is rejected.
    let again = evaluator.promote();
    assert!(matches!(again, Err(ExperimentError::BadStageOrder { .. })));
}
