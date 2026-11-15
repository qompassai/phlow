//! PH-9 — self-improvement safety case, `phlow-self-improve` suite.
//! One test per denial that lives in this crate; the full case is
//! `docs/SELF_IMPROVEMENT_SAFETY.md`. If a future change weakens a
//! denial, it must delete or rewrite the matching test here — which
//! is the visibility the safety case exists to provide.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use phlow_self_improve::divergence::{DivergenceStore, PlanDivergence};
use phlow_self_improve::escape::{
    Escape, EscapeStore, cluster_escapes, queue_check_proposal, validate_check,
};
use phlow_self_improve::lessons::LessonStore;
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

fn divergence(run_id: &str, content_label: &str) -> PlanDivergence {
    PlanDivergence {
        run_id: run_id.to_owned(),
        task_class: "code_fix".to_owned(),
        planned_steps: 1,
        executed_steps: 0,
        skipped_steps: vec!["Replace the value in `src/a.rs`.".to_owned()],
        untracked_steps: 0,
        planned_files_untouched: vec!["src/a.rs".to_owned()],
        added_files: Vec::new(),
        repair_cycles: 0,
        origin_lane: "phlow".to_owned(),
        content_label: content_label.to_owned(),
    }
}

#[test]
fn divergence_records_and_lesson_cards_are_append_only() {
    // PH-6's append-only rule (safety case §6): plan-divergence
    // records and lesson cards are evidence. A card is superseded
    // by appending the next version, never by rewriting one — the
    // triggers refuse UPDATE/DELETE through any connection.
    let dir = temp_dir("ph6-append-only");
    let db = dir.join("feedback.db");
    let divergences = DivergenceStore::open(&db).unwrap();
    divergences.record(&divergence("run-1", "nsfw")).unwrap();
    let lessons = LessonStore::open(&db).unwrap();
    lessons.build_card(&divergences, "code_fix").unwrap();
    let other = rusqlite::Connection::open(&db).unwrap();
    assert!(other
        .execute("UPDATE plan_divergences SET repair_cycles = 9 WHERE run_id = 'run-1'", [])
        .is_err());
    assert!(other
        .execute("DELETE FROM plan_divergences WHERE run_id = 'run-1'", [])
        .is_err());
    assert!(other
        .execute("UPDATE lesson_cards SET version = 9 WHERE task_class = 'code_fix'", [])
        .is_err());
    assert!(other
        .execute("DELETE FROM lesson_cards WHERE task_class = 'code_fix'", [])
        .is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn escapes_are_append_only_and_require_a_clean_run() {
    // PH-3's recording rule (safety case §7): an escape exists only
    // where the approved checks all passed. A run with a failed
    // check caught its defect — recording it as an escape is
    // refused — and a recorded escape cannot be rewritten.
    let dir = temp_dir("ph3-recording");
    let db = dir.join("feedback.db");
    let outcomes = OutcomeStore::open(&db).unwrap();
    let mut row = RunOutcome {
        run_id: "run-caught".to_owned(),
        task_class: "code_fix".to_owned(),
        plan: "plan".to_owned(),
        checks_json: "[{\"name\":\"tests\",\"status\":\"failed\",\"returncode\":1}]".to_owned(),
        reviewer_verdict: Some("not_approved".to_owned()),
        cycles: 1,
        tool_calls: 1,
        model_calls: 1,
        context_chars: None,
        wall_ms: 1,
        model_ids: Vec::new(),
        status: "failed".to_owned(),
        verification_strength: VerificationStrength::SelfReported,
        origin_lane: "phlow".to_owned(),
        content_label: "nsfw".to_owned(),
        evidence: "safety case".to_owned(),
    };
    outcomes.record(&row).unwrap();
    let escapes = EscapeStore::open(&db).unwrap();
    let escape = Escape {
        escape_id: "esc-1".to_owned(),
        run_id: "run-caught".to_owned(),
        defect: "wrong constant".to_owned(),
        found_by: "reviewer".to_owned(),
        check_name: "osc_bounds".to_owned(),
        check_argv: vec!["sh".to_owned(), "check.sh".to_owned()],
    };
    assert!(matches!(
        escapes.record(&outcomes, &escape),
        Err(SelfImproveError::EscapeInvalid {
            field: "checks_json",
            ..
        })
    ));
    // The same escape against a clean run records — and is then
    // append-only against any connection.
    row.run_id = "run-clean".to_owned();
    row.checks_json = "[{\"name\":\"tests\",\"status\":\"ok\",\"returncode\":0}]".to_owned();
    outcomes.record(&row).unwrap();
    let mut escape = escape.clone();
    escape.run_id = "run-clean".to_owned();
    escapes.record(&outcomes, &escape).unwrap();
    let other = rusqlite::Connection::open(&db).unwrap();
    assert!(other
        .execute("UPDATE escapes SET defect = 'rewritten' WHERE escape_id = 'esc-1'", [])
        .is_err());
    assert!(other
        .execute("DELETE FROM escapes WHERE escape_id = 'esc-1'", [])
        .is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_check_proposal_never_admits_itself() {
    // PH-3's admission rule (safety case §7): the whole proposal
    // path — record, cluster, validate, queue — ends at a PENDING
    // approval record and a proposal file. It writes no config,
    // admits no check, and no actor but an allowlisted operator can
    // move the record.
    let dir = temp_dir("ph3-admission");
    let db = dir.join("feedback.db");
    // A bystander config file the pipeline must never touch.
    let config = dir.join("config.toml");
    let config_bytes = b"[checks.tests]\ncmd = [\"true\"]\n".to_vec();
    std::fs::write(&config, &config_bytes).unwrap();

    let outcomes = OutcomeStore::open(&db).unwrap();
    outcomes
        .record(&RunOutcome {
            run_id: "run-1".to_owned(),
            task_class: "code_fix".to_owned(),
            plan: "plan".to_owned(),
            checks_json: "[{\"name\":\"tests\",\"status\":\"ok\",\"returncode\":0}]".to_owned(),
            reviewer_verdict: Some("approved".to_owned()),
            cycles: 1,
            tool_calls: 1,
            model_calls: 1,
            context_chars: None,
            wall_ms: 1,
            model_ids: Vec::new(),
            status: "ok".to_owned(),
            verification_strength: VerificationStrength::ToolChecked,
            origin_lane: "phlow".to_owned(),
            content_label: "nsfw".to_owned(),
            evidence: "safety case".to_owned(),
        })
        .unwrap();
    let escapes = EscapeStore::open(&db).unwrap();
    escapes
        .record(
            &outcomes,
            &Escape {
                escape_id: "esc-1".to_owned(),
                run_id: "run-1".to_owned(),
                defect: "wrong constant shipped".to_owned(),
                found_by: "reviewer".to_owned(),
                check_name: "osc_bounds".to_owned(),
                check_argv: vec!["sh".to_owned(), "check.sh".to_owned()],
            },
        )
        .unwrap();
    let clusters = cluster_escapes(&escapes.list(10).unwrap());
    assert_eq!(clusters.len(), 1);

    // Validate against the bracketing revisions.
    let escaped = dir.join("escaped");
    let fixed = dir.join("fixed");
    std::fs::create_dir_all(&escaped).unwrap();
    std::fs::create_dir_all(&fixed).unwrap();
    std::fs::write(escaped.join("check.sh"), "exit 1\n").unwrap();
    std::fs::write(fixed.join("check.sh"), "exit 0\n").unwrap();
    let validation = validate_check(&clusters[0].check_argv, &escaped, &fixed);
    assert!(validation.gate_holds);

    // Queue it: the record is pending, and stays pending.
    let mut queue = phlow_approval::ApprovalQueue::new(&["matt"]).unwrap();
    let record = queue_check_proposal(&mut queue, &clusters[0], &validation).unwrap();
    assert_eq!(record.state, phlow_approval::State::Pending);
    // A non-operator cannot decide it; a reserved actor name
    // cannot decide it; the loop itself holds no decision path.
    assert!(matches!(
        queue.decide(&record.id, phlow_approval::HumanVerdict::Approve, Some("mallory")),
        Err(phlow_approval::Error::ActorRefused)
    ));
    assert!(matches!(
        queue.decide(&record.id, phlow_approval::HumanVerdict::Approve, Some("phlow")),
        Err(phlow_approval::Error::ActorRefused)
    ));
    assert!(matches!(
        queue.decide(&record.id, phlow_approval::HumanVerdict::Approve, None),
        Err(phlow_approval::Error::ActorRefused)
    ));
    assert_eq!(
        queue.get(&record.id).unwrap().state,
        phlow_approval::State::Pending
    );
    // The bystander config is byte-identical: nothing was admitted
    // anywhere by the pipeline.
    assert_eq!(std::fs::read(&config).unwrap(), config_bytes);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn nsfw_lessons_are_invisible_to_sfw_retrieval() {
    // PH-6's lane rule (safety case §6, Bloch): a card built from
    // nsfw-labelled records carries the nsfw label and cannot be
    // retrieved under an sfw scope — lessons never cross the lane
    // boundary silently.
    let dir = temp_dir("ph6-lanes");
    let db = dir.join("feedback.db");
    let divergences = DivergenceStore::open(&db).unwrap();
    divergences.record(&divergence("run-1", "nsfw")).unwrap();
    let lessons = LessonStore::open(&db).unwrap();
    let card = lessons
        .build_card(&divergences, "code_fix")
        .unwrap()
        .expect("card builds");
    assert_eq!(card.content_label, "nsfw");
    assert!(lessons.current_card("code_fix", "sfw").unwrap().is_none());
    assert!(lessons.current_card("code_fix", "nsfw").unwrap().is_some());
    let _ = std::fs::remove_dir_all(&dir);
}
