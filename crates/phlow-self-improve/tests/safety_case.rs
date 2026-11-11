//! PH-9 — self-improvement safety case, `phlow-self-improve` suite.
//! One test per denial that lives in this crate; the full case is
//! `docs/SELF_IMPROVEMENT_SAFETY.md`. If a future change weakens a
//! denial, it must delete or rewrite the matching test here — which
//! is the visibility the safety case exists to provide.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use phlow_self_improve::outcome::{RunOutcome, VerificationStrength};
use phlow_self_improve::{OutcomeStore, PromptEvolver, SelfImproveError, SkillStore};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_dir(prefix: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "phlow-safety-test-{prefix}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&dir).expect("test setup: create temp dir");
    dir
}

#[test]
fn prompt_evolution_is_denied() {
    // Denial 1: the loop may never edit its own prompts (PH-4's
    // automatic mutation stays OFF).
    let evolver = PromptEvolver::new();
    let error = evolver.evolve().unwrap_err();
    assert!(matches!(error, SelfImproveError::EvolutionDisabled));
    assert_eq!(
        error.to_string(),
        "Automatic prompt evolution and implicit Git commits are disabled. \
         Review and edit operator-owned prompts manually."
    );
}

#[test]
fn skill_mutation_is_denied() {
    // Denial 4: the loop may never mutate its own skills.
    let dir = temp_dir("skills");
    let store = SkillStore::open(&dir).unwrap();
    let error = store.save_skill("anything", "content").unwrap_err();
    assert!(matches!(error, SelfImproveError::SkillMutationDisabled));
    assert_eq!(
        error.to_string(),
        "Automatic skill mutation is disabled; edit manually"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn history_is_neither_read_nor_rewritten() {
    // Denial 5a: the loop may never touch history — reverting through
    // Git is denied, and history is not even read (always empty).
    let dir = temp_dir("history");
    let store = SkillStore::open(&dir).unwrap();
    let error = store.revert_skill("anything").unwrap_err();
    assert!(matches!(error, SelfImproveError::GitMutationDisabled));
    assert_eq!(
        error.to_string(),
        "Automatic Git mutation is disabled; revert manually"
    );
    assert!(store.history("anything").is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn run_history_is_append_only_and_exactly_one_row() {
    // Denial 5b: recorded run history cannot be rewritten — a second
    // row for one run is refused outright (the UPDATE/DELETE trigger
    // proof lives in `outcome`'s unit tests, beside the DDL).
    let dir = temp_dir("outcomes");
    let db = dir.join("feedback.db");
    let store = OutcomeStore::open(&db).unwrap();
    let row = RunOutcome {
        run_id: "run-1".to_owned(),
        task_class: "flow_run".to_owned(),
        plan: "plan".to_owned(),
        checks_json: "[]".to_owned(),
        reviewer_verdict: None,
        cycles: 0,
        tool_calls: 0,
        model_calls: 0,
        context_chars: None,
        wall_ms: 1,
        model_ids: Vec::new(),
        status: "abandoned".to_owned(),
        verification_strength: VerificationStrength::SelfReported,
        origin_lane: "phlow".to_owned(),
        content_label: "nsfw".to_owned(),
        evidence: "safety case".to_owned(),
    };
    store.record(&row).unwrap();
    let dup = store.record(&row);
    assert!(matches!(
        dup,
        Err(SelfImproveError::OutcomeDuplicate { .. })
    ));
    assert_eq!(store.count_for_run("run-1").unwrap(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}
