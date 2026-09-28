//! Shared test helpers: explicit inputs, no hidden state.
//!
//! Every helper builds its value from literal arguments so each test states
//! exactly what it feeds the code under test.
//!
//! This module is compiled separately into each integration-test binary and
//! each binary uses only a subset, so per-binary dead-code analysis would
//! flag the rest. The allow below is scoped to that artifact: every helper
//! here is used by at least one test binary (verified by grep 2026-09-28),
//! so it silences subset noise, not genuine disuse.
#![allow(dead_code)]

use ml_dsa::{Keypair, MlDsa65, Signer as _};
use phlow_experiment::{
    ArtifactDigest, BudgetTracker, CapabilitySet, CheckRun, ConsumedApprovals, EvidenceBundle,
    ExperimentError, ExperimentId, ManualClock, NodeId, NodeParams, OperatorRegistry,
    ProposalBudgets, ProposalParams, ReviewDecision, ReviewerDecision, RiskClass, RunId, Scheduler,
    SchedulerLimits, SchedulerNode, VerificationOutcome, WorkerRole,
};
use std::path::PathBuf;
use std::sync::OnceLock;

/// Unwraps a `Result`, panicking with the typed error on failure.
///
/// Tests use this instead of `unwrap()` so a failure prints the
/// domain error, not a bare "called unwrap on Err".
pub fn ok<T>(result: Result<T, ExperimentError>) -> T {
    match result {
        Ok(value) => value,
        Err(error) => panic!("expected Ok, got error: {error}"),
    }
}

/// Reads a shipped manifest from `manifests/` by file name.
pub fn manifest_text(name: &str) -> String {
    let path = format!("{}/manifests/{name}", env!("CARGO_MANIFEST_DIR"));
    match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) => panic!("cannot read manifest {path}: {error}"),
    }
}

/// Reads a shipped eval file by crate-relative path.
pub fn eval_text(relative: &str) -> String {
    let path = format!("{}/{relative}", env!("CARGO_MANIFEST_DIR"));
    match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) => panic!("cannot read eval file {path}: {error}"),
    }
}

/// A well-formed v2 operator approval record for tests: the exact eight
/// keys, dual-signed with the deterministic test keypair below.
///
/// Shape: `v: 2`, operator `gauntlet-test-operator`, a 32-hex-char
/// candidate digest, a positive `expires_ms`, a 128-hex-char Ed25519
/// signature and a 6618-hex-char ML-DSA-65 signature over the canonical
/// bytes. The keypair seeds are fixed test constants — never real key
/// material — so fixtures are reproducible without randomness.
pub fn operator_record() -> String {
    signed_operator_record(
        TEST_OPERATOR,
        "APR-TEST-0001",
        "9f2b3c4d5e6f708192a3b4c5d6e7f809",
        "phlow-experiment/test",
        TEST_EXPIRES_MS,
        test_keypair(),
    )
}

/// Operator name enrolled in every test registry. Deliberately not a real
/// person's identity: Matt's real operator key enrolls separately via the
/// documented ceremony; tests must never invent it.
pub const TEST_OPERATOR: &str = "gauntlet-test-operator";

/// `expires_ms` used by the standard genuine fixture (2030-01-01T00:00:00Z).
pub const TEST_EXPIRES_MS: u64 = 1_893_456_000_000;

/// A `ManualClock` reading just before [`TEST_EXPIRES_MS`], so the
/// standard fixture is live but near expiry.
pub fn test_clock() -> ManualClock {
    ManualClock::new(TEST_EXPIRES_MS - 1_000)
}

/// The acting-agent identity for tests. Must differ from [`TEST_OPERATOR`]
/// (the self-approval tests override it deliberately).
pub fn test_agent() -> &'static str {
    "test-agent-x"
}

/// A fresh, empty replay store.
pub fn test_consumed() -> ConsumedApprovals {
    ConsumedApprovals::new()
}

/// A deterministic Ed25519 + ML-DSA-65 keypair for tests, generated once
/// per test process from fixed seeds. `from_seed`/`from_bytes` need no RNG,
/// so fixtures are reproducible and fast.
pub struct TestKeypair {
    /// Ed25519 signing key (classical half).
    pub ed_sk: ed25519_dalek::SigningKey,
    /// ML-DSA-65 signing key (PQ half).
    pub pq_sk: ml_dsa::SigningKey<MlDsa65>,
    /// Raw Ed25519 public key (32 bytes).
    pub ed_pk: [u8; 32],
    /// Raw ML-DSA-65 public key (1952 bytes).
    pub pq_pk: [u8; 1952],
}

/// The primary test keypair (seeds `0x42` / `0x24`).
pub fn test_keypair() -> &'static TestKeypair {
    static KEYPAIR: OnceLock<TestKeypair> = OnceLock::new();
    KEYPAIR.get_or_init(|| make_keypair([0x42; 32], [0x24; 32]))
}

/// A second, independent test keypair (seeds `0x43` / `0x25`) for
/// wrong-key adversarial tests.
pub fn test_keypair_b() -> &'static TestKeypair {
    static KEYPAIR_B: OnceLock<TestKeypair> = OnceLock::new();
    KEYPAIR_B.get_or_init(|| make_keypair([0x43; 32], [0x25; 32]))
}

fn make_keypair(ed_seed: [u8; 32], pq_seed: [u8; 32]) -> TestKeypair {
    let ed_sk = ed25519_dalek::SigningKey::from_bytes(&ed_seed);
    let seed = ml_dsa::Seed::from(pq_seed);
    let pq_sk = ml_dsa::SigningKey::<MlDsa65>::from_seed(&seed);
    let ed_pk = ed_sk.verifying_key().to_bytes();
    let mut pq_pk = [0u8; 1952];
    pq_pk.copy_from_slice(pq_sk.verifying_key().encode().as_slice());
    TestKeypair {
        ed_sk,
        pq_sk,
        ed_pk,
        pq_pk,
    }
}

/// The exact canonical bytes the gate signs/verifies: the six
/// non-signature fields in fixed order, `key: value` lines joined by LF,
/// no trailing newline. Rebuilt here independently of the implementation
/// so a canonicalization divergence fails loudly.
fn canonical_bytes(
    operator: &str,
    approval_id: &str,
    candidate: &str,
    scope: &str,
    expires_ms: u64,
) -> Vec<u8> {
    format!(
        "v: 2\n\
         operator: {operator}\n\
         approval_id: {approval_id}\n\
         candidate: {candidate}\n\
         scope: {scope}\n\
         expires_ms: {expires_ms}"
    )
    .into_bytes()
}

/// Lowercase hex encoding (test-only).
pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

/// Decodes even-length lowercase hex; panics on bad input (test-only, so
/// fixtures stay honest about what they feed the parser).
pub fn decode_hex(hex: &str) -> Vec<u8> {
    fn nibble(byte: u8) -> u8 {
        match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            _ => panic!("bad hex digit in test fixture: {byte:#x}"),
        }
    }
    let bytes = hex.as_bytes();
    assert!(
        bytes.len().is_multiple_of(2),
        "odd hex length in test fixture"
    );
    let (chunks, _) = bytes.as_chunks::<2>();
    chunks
        .iter()
        .map(|pair| (nibble(pair[0]) << 4) | nibble(pair[1]))
        .collect()
}

/// Builds a v2 operator record dual-signed with `keypair`:
///
/// 1. `ed_sig = Ed25519.Sign(canonical)`
/// 2. `pq_sig = ML-DSA-65.Sign(canonical || ed_sig)` (nested binding)
pub fn signed_operator_record(
    operator: &str,
    approval_id: &str,
    candidate: &str,
    scope: &str,
    expires_ms: u64,
    keypair: &TestKeypair,
) -> String {
    let canonical = canonical_bytes(operator, approval_id, candidate, scope, expires_ms);
    let ed_sig = keypair.ed_sk.sign(&canonical);
    let mut pq_message = canonical;
    pq_message.extend_from_slice(&ed_sig.to_bytes());
    let pq_sig = keypair.pq_sk.sign(&pq_message);
    format!(
        "v: 2\n\
         operator: {operator}\n\
         approval_id: {approval_id}\n\
         candidate: {candidate}\n\
         scope: {scope}\n\
         expires_ms: {expires_ms}\n\
         signature_ed25519: {ed_hex}\n\
         signature_mldsa65: {pq_hex}\n",
        ed_hex = hex(&ed_sig.to_bytes()),
        pq_hex = hex(pq_sig.encode().as_slice()),
    )
}

/// SHA-256 over `ed_pk || pq_pk`: the enrollment fingerprint.
fn fingerprint(ed_pk: &[u8; 32], pq_pk: &[u8; 1952]) -> [u8; 32] {
    use sha2::Digest as _;
    let mut hasher = sha2::Sha256::new();
    hasher.update(ed_pk);
    hasher.update(pq_pk);
    hasher.finalize().into()
}

/// One operator entry for a test registry file.
pub struct RegistryEntry<'a> {
    /// Operator name.
    pub name: &'a str,
    /// Keypair whose public keys are enrolled.
    pub keypair: &'a TestKeypair,
    /// Whether the entry is revoked.
    pub revoked: bool,
}

/// Writes a TOML operator registry enrolling `entries` to a fresh
/// directory under the system temp dir, sets owner-only (0600)
/// permissions, and returns the file path. The loader under test reads
/// this file, so the round trip exercises the real parse path.
pub fn write_registry_file(entries: &[RegistryEntry<'_>]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "phlow-test-registry-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    ));
    std::fs::create_dir_all(&dir)
        .unwrap_or_else(|error| panic!("cannot create test registry dir: {error}"));
    let path = dir.join("operators.toml");
    let mut toml = String::new();
    for entry in entries {
        let print = fingerprint(&entry.keypair.ed_pk, &entry.keypair.pq_pk);
        toml.push_str(&format!(
            "[operators.\"{}\"]\n\
             ed25519_pubkey = \"{}\"\n\
             mldsa65_pubkey = \"{}\"\n\
             fingerprint = \"{}\"\n\
             enrolled_ms = 1750000000000\n\
             revoked = {}\n",
            entry.name,
            hex(&entry.keypair.ed_pk),
            hex(&entry.keypair.pq_pk),
            hex(&print),
            entry.revoked,
        ));
    }
    std::fs::write(&path, &toml)
        .unwrap_or_else(|error| panic!("cannot write test registry: {error}"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .unwrap_or_else(|error| panic!("cannot chmod test registry: {error}"));
    }
    path
}

/// Loads a test [`OperatorRegistry`] enrolling [`TEST_OPERATOR`] with the
/// primary test keypair (not revoked).
pub fn test_registry() -> OperatorRegistry {
    let path = write_registry_file(&[RegistryEntry {
        name: TEST_OPERATOR,
        keypair: test_keypair(),
        revoked: false,
    }]);
    OperatorRegistry::load(&path)
        .unwrap_or_else(|error| panic!("test registry failed to load: {error}"))
}

/// A root capability set for tests: two tools, two paths, positive budgets.
pub fn root_capabilities() -> CapabilitySet {
    ok(CapabilitySet::new(
        vec!["read".to_string(), "exec-check".to_string()],
        vec!["workspace/".to_string(), "fixtures/".to_string()],
        64,
        1_048_576,
    ))
}

/// Constructor params for a test node with id `node`.
pub fn node_params(node: &str) -> NodeParams {
    NodeParams {
        run_id: ok(RunId::new("run-test-001")),
        experiment_id: ok(ExperimentId::new("exp-test-001")),
        baseline_revision: "abc123".to_string(),
        workspace_snapshot: "snap-001".to_string(),
        node_id: ok(NodeId::new(node)),
        parent_node_id: None,
        role: WorkerRole::Implementer,
        capabilities: root_capabilities(),
        input_digest: "input-digest-001".to_string(),
        dependency_ids: Vec::new(),
        generation: 0,
        attempt: 0,
        deadline_ms: 300_000,
        cpu_budget_ms: 60_000,
        memory_budget_bytes: 536_870_912,
        output_bytes_max: 1_048_576,
        tool_calls_remaining: 64,
    }
}

/// A valid test node with id `node`, in `Proposed` state.
pub fn test_node(node: &str) -> SchedulerNode {
    ok(SchedulerNode::new(node_params(node)))
}

/// A scheduler with default limits.
pub fn test_scheduler() -> Scheduler {
    ok(Scheduler::new(SchedulerLimits::default()))
}

/// A complete evidence bundle: one passing check with exact argv, one
/// artifact digest, and a verified outcome with coverage.
pub fn complete_evidence() -> EvidenceBundle {
    let checks = vec![ok(CheckRun::new(
        "tests",
        vec![
            "cargo".to_string(),
            "test".to_string(),
            "--locked".to_string(),
        ],
        true,
    ))];
    let artifacts = vec![ok(ArtifactDigest::new("candidate.diff", "9f2b3c4d"))];
    let verification = ok(VerificationOutcome::new(
        true,
        vec!["src/lib.rs".to_string()],
    ));
    ok(EvidenceBundle::new(checks, artifacts, verification))
}

/// A budget tracker with headroom: 64 tool calls, 1 MiB output, 5 min.
pub fn test_budget() -> BudgetTracker {
    ok(BudgetTracker::new(64, 1_048_576, 300_000))
}

/// Valid proposal params: one changed file, one approving reviewer.
pub fn proposal_params() -> ProposalParams {
    ProposalParams {
        trigger: "REG-2026-001".to_string(),
        failure_category: "timeout-handling".to_string(),
        baseline_revision: "abc123".to_string(),
        candidate_diff_summary: "Tighten the check timeout path.".to_string(),
        changed_surface: vec!["src/checks.rs".to_string()],
        expected_benefit: "Fewer hung checks.".to_string(),
        risk_class: RiskClass::Normal,
        budgets: ProposalBudgets {
            tool_calls_max: 64,
            wall_ms_max: 300_000,
        },
        test_version: "suites-1".to_string(),
        evaluator_version: "evaluator-1".to_string(),
        results_summary: "All gates green.".to_string(),
        reviewer_decisions: vec![ReviewerDecision {
            reviewer: WorkerRole::SecurityReviewer,
            decision: ReviewDecision::Approve,
        }],
        rollback_target: "abc123".to_string(),
    }
}

/// Builds a proposal whose changed surface is exactly `paths`.
pub fn proposal_with_surface(paths: Vec<String>) -> phlow_experiment::ImprovementProposal {
    let mut params = proposal_params();
    params.changed_surface = paths;
    ok(phlow_experiment::ImprovementProposal::new(params))
}
