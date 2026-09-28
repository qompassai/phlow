//! Task 139 — finding deduplication across cycles (rust, validation +
//! adversarial).
//!
//! `FindingStore::insert` keys on the fingerprint: the same finding
//! observed in two cycles is one record with two observations — never
//! two records (V1); genuinely different findings are two records (V2).
//! Near-duplicates (same title, different fingerprint fields) are NOT
//! fuzzy-merged — merging is the operator's job, a deliberate precision
//! choice (A1). A forced fingerprint collision with different content
//! (A2) is the honest-failure arm: the scaffold has no collision
//! tiebreak, so the verdict is NEGATIVE with full evidence, and the
//! gap is reported as a scaffold bug rather than papered over.

use crate::bounty::{Evidence, Finding, FindingState, FindingStore, TargetId};
use crate::skillopt::driver::{CaseReport, TaskDriverError, verdict_line};
use crate::skillopt::learner::Verdict;
use crate::{TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-139";
/// Task name.
pub const NAME: &str = "finding deduplication across cycles";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 validation + 2 adversarial.
pub const CASES: [&str; 4] = [
    "repeat_observation_single_record",
    "distinct_findings_two_records",
    "near_duplicate_no_fuzzy_merge",
    "fingerprint_collision_preserves_data",
];

/// Build a candidate finding. The fingerprint is the dedup key; fixtures
/// use literal hex strings standing in for `sha256(canonical fields)`.
fn mk_finding(fingerprint: &str, title: &str, body: &str) -> Finding {
    Finding {
        id: String::new(),
        target_id: TargetId("t01".to_string()),
        fingerprint: fingerprint.to_string(),
        title: title.to_string(),
        state: FindingState::Candidate,
        evidence: Evidence {
            raw: body.as_bytes().to_vec(),
            sha256: format!("sha256:fixture:{fingerprint}"),
            custody: Vec::new(),
            truncated: false,
        },
        observation_count: 0,
        reject_reason: None,
    }
}

/// V1: the identical finding in cycle 1 and cycle 2 is one record with
/// `observation_count == 2`.
fn case_repeat_observation_single_record() -> Result<CaseReport, TaskDriverError> {
    let mut store = FindingStore::new();
    let (id1, new1) = store.insert(mk_finding(
        "fp-9f2a",
        "stored xss in search",
        "cycle-1 body",
    ));
    let (id2, new2) = store.insert(mk_finding(
        "fp-9f2a",
        "stored xss in search",
        "cycle-2 body",
    ));
    let record = store.get_by_fingerprint("fp-9f2a");
    let mut failures = Vec::new();
    if !new1 {
        failures.push("cycle-1 insert did not create a record".to_string());
    }
    if new2 {
        failures.push("cycle-2 insert created a second record".to_string());
    }
    if id1 != id2 {
        failures.push(format!("record ids differ: {id1} vs {id2}"));
    }
    match record {
        Some(r) if r.observation_count == 2 => {}
        other => failures.push(format!(
            "observation_count != 2: {:?}",
            other.map(|r| r.observation_count)
        )),
    }
    if store.record_count() != 1 {
        failures.push(format!("record_count {} != 1", store.record_count()));
    }
    let evidence = vec![
        "cycle 1: insert fp-9f2a -> (id, is_new=true)".to_string(),
        "cycle 2: insert fp-9f2a -> (same id, is_new=false)".to_string(),
        format!("record_count = 1, observation_count = 2, id = {id1}"),
        "one record, two observations — never two records".to_string(),
        "synthetic findings; ManualClock unused by the store (MOCK)".to_string(),
    ];
    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "records": store.record_count(),
            "observations": 2,
            "record_id": id1,
            "backend": "scripted-mock",
        }),
        [evidence, failures.clone()].concat(),
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// V2: two genuinely different findings are two records.
fn case_distinct_findings_two_records() -> Result<CaseReport, TaskDriverError> {
    let mut store = FindingStore::new();
    let (id_a, new_a) = store.insert(mk_finding("fp-aaaa", "stored xss in search", "body-a"));
    let (id_b, new_b) = store.insert(mk_finding("fp-bbbb", "sqli in login", "body-b"));
    let mut failures = Vec::new();
    if !new_a || !new_b {
        failures.push("a distinct finding did not create a record".to_string());
    }
    if id_a == id_b {
        failures.push("distinct findings share a record id".to_string());
    }
    if store.record_count() != 2 {
        failures.push(format!("record_count {} != 2", store.record_count()));
    }
    for fp in ["fp-aaaa", "fp-bbbb"] {
        match store.get_by_fingerprint(fp) {
            Some(r) if r.observation_count == 1 => {}
            other => failures.push(format!(
                "{fp}: observation_count != 1: {:?}",
                other.map(|r| r.observation_count)
            )),
        }
    }
    let evidence = vec![
        "insert fp-aaaa and fp-bbbb -> two records, distinct ids".to_string(),
        format!("record_count = 2; {id_a}: 1 observation, {id_b}: 1 observation"),
        "synthetic findings (MOCK)".to_string(),
    ];
    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "records": store.record_count(),
            "backend": "scripted-mock",
        }),
        [evidence, failures.clone()].concat(),
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// A1 (adversarial): near-duplicate — same title, different fingerprint
/// fields — must NOT be merged. Merging is the operator's job; the
/// store's precision rule is exact-match on the fingerprint.
fn case_near_duplicate_no_fuzzy_merge() -> Result<CaseReport, TaskDriverError> {
    let mut store = FindingStore::new();
    let title = "reflected xss in q parameter";
    let (_, new_a) = store.insert(mk_finding("fp-near-1", title, "evidence variant one"));
    let (_, new_b) = store.insert(mk_finding("fp-near-2", title, "evidence variant two"));
    let mut failures = Vec::new();
    if !new_a || !new_b {
        failures.push("near-duplicate was merged — fuzzy matching detected".to_string());
    }
    if store.record_count() != 2 {
        failures.push(format!(
            "record_count {} != 2: near-duplicates merged",
            store.record_count()
        ));
    }
    let evidence = vec![
        "same title, different fingerprint fields -> two records".to_string(),
        "no fuzzy merge: dedup is exact-match on fingerprint by design".to_string(),
        "merging near-duplicates is the operator's job (precision choice, stated)".to_string(),
        "synthetic findings (MOCK)".to_string(),
    ];
    let mut report = CaseReport::pass(
        CASES[2],
        serde_json::json!({
            "records": store.record_count(),
            "fuzzy_merge": false,
            "backend": "scripted-mock",
        }),
        [evidence, failures.clone()].concat(),
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// A2 (adversarial): forced fingerprint collision — same fingerprint
/// string, different content. The design demands a deterministic
/// tiebreak preserving both findings. The scaffold has none:
/// `FindingStore::insert` keys solely on the fingerprint string and
/// silently absorbs the second finding's content. Verdict: NEGATIVE,
/// with the data loss measured exactly. (Scaffold bug reported; see
/// the task doc.)
fn case_fingerprint_collision_preserves_data() -> Result<CaseReport, TaskDriverError> {
    let mut store = FindingStore::new();
    let (id_a, new_a) = store.insert(mk_finding("fp-collide", "title-A", "body-A"));
    let (id_b, new_b) = store.insert(mk_finding("fp-collide", "title-B", "body-B"));
    let record = store.get_by_fingerprint("fp-collide").cloned();
    let mut failures = Vec::new();
    if !new_a {
        failures.push("first insert did not create a record".to_string());
    }
    // The measurement: is finding B's content still recoverable?
    let b_preserved = match &record {
        Some(r) => r.title == "title-B" || store.record_count() == 2,
        None => false,
    };
    if b_preserved {
        failures.push("collision tiebreak exists — update this case (see doc)".to_string());
    }
    let verdict = if b_preserved {
        Verdict::Replicates
    } else {
        Verdict::Negative
    };
    let detail = match &record {
        Some(r) => format!(
            "second insert -> (id={id_b}, is_new={new_b}); stored title is {:?} \
             (finding B's content dropped), observation_count={}",
            r.title, r.observation_count
        ),
        None => "fingerprint vanished from the store".to_string(),
    };
    let mut evidence = vec![
        "forced collision: same fingerprint string, different title/body".to_string(),
        format!("insert A -> ({id_a}, is_new={new_a}); insert B -> ({id_b}, is_new={new_b})"),
        detail.clone(),
        "design demands a deterministic tiebreak preserving both; scaffold has none".to_string(),
        verdict_line("139", verdict, &detail),
        "synthetic findings (MOCK)".to_string(),
    ];
    evidence.extend(failures.iter().cloned());
    // The case passes when the measurement is complete and classified
    // (task-110 precedent): the verdict — negative — is the finding.
    // If the scaffold ever gains a collision tiebreak, `failures` trips
    // and this case (plus its test) must be updated.
    let mut report = CaseReport::pass(
        CASES[3],
        serde_json::json!({
            "verdict": verdict.to_string(),
            "records": store.record_count(),
            "finding_b_preserved": b_preserved,
            "detail": detail,
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// Run one driver case by name.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "repeat_observation_single_record" => case_repeat_observation_single_record(),
        "distinct_findings_two_records" => case_distinct_findings_two_records(),
        "near_duplicate_no_fuzzy_merge" => case_near_duplicate_no_fuzzy_merge(),
        "fingerprint_collision_preserves_data" => case_fingerprint_collision_preserves_data(),
        _ => Err(TaskDriverError::Fixture {
            what: "case".to_string(),
            detail: format!("task-139: unknown case '{case}'"),
        }),
    }
}

/// Task-level entry for the gauntlet runner: the headline case.
pub fn run(_ctx: &crate::Ctx) -> TaskOutcome {
    match run_case(CASES[0]) {
        Ok(r) if r.passed => TaskOutcome::Pass {
            evidence: r.evidence,
        },
        Ok(r) => TaskOutcome::Fail {
            where_: "task-139".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-139".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
