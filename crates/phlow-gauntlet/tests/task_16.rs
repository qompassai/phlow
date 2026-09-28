//! Integration tests for task-16: manifest validation.
//!
//! 50/50 split against phlow-experiment's real manifest parsers
//! (`phlow_experiment::manifest::parse_*_manifest`).
//!
//! - V1: the driver battery passes — 4 valid manifests parse, 17
//!   malformed/adversarial manifests are rejected with typed errors.
//! - V2: valid manifests parse independently, including the crate's
//!   shipped `manifests/*.toml` and one real eval manifest.
//! - A1: type-confused inputs (string/float/negative/bool-as-int) are
//!   rejected with `ManifestInvalid` — values are never coerced.
//! - A2: size bombs and injected keys are rejected, and every rejection
//!   carries the caller's file label plus a non-empty key and reason.

use phlow_experiment::{
    ExperimentError, RiskClass, parse_budget_manifest, parse_promotion_manifest,
    parse_suite_manifest, parse_task_manifest,
};
use phlow_gauntlet::tasks::task_16;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;

/// Build a `Ctx` for one test. This task drives no Neovim, so the
/// binary/diver paths are documented placeholders.
fn test_ctx() -> Ctx {
    Ctx::new(
        PathBuf::from("unused: task-16 is TaskKind::Rust, no nvim involved"),
        PathBuf::from("unused: task-16 is TaskKind::Rust, no diver lua involved"),
        std::env::temp_dir().join("gauntlet-task-16"),
    )
    .expect("gauntlet test: Ctx::new rejected non-empty paths")
}

/// Run the driver; unwrap the Pass outcome or fail with the driver's own
/// evidence attached.
fn run_pass() -> Vec<String> {
    match task_16::run(&test_ctx()) {
        TaskOutcome::Pass { evidence } => evidence,
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => panic!("task-16 driver failed at {where_}: {how}\nevidence: {evidence:?}"),
    }
}

/// Assert one manifest is rejected with `ManifestInvalid` whose file label,
/// key, and reason match. The label is deliberately distinct from the
/// driver's own label: the parsers must echo whatever label the caller
/// passes, so using a fresh one proves the propagation contract.
fn assert_rejected(
    toml: &str,
    parse: impl Fn(&str, &'static str) -> Result<(), ExperimentError>,
    want_key: &str,
    want_reason: &str,
) {
    const LABEL: &str = "task-16-adversarial";
    match parse(toml, LABEL) {
        Ok(()) => panic!("malformed manifest accepted (want key {want_key:?}): {toml}"),
        Err(ExperimentError::ManifestInvalid { file, key, reason }) => {
            assert_eq!(file, LABEL, "rejection did not echo the file label");
            assert!(
                key.contains(want_key),
                "key {key:?} does not contain {want_key:?}"
            );
            assert!(
                reason.contains(want_reason),
                "reason {reason:?} does not contain {want_reason:?}"
            );
            assert!(!key.is_empty() && !reason.is_empty());
            let rendered = format!(
                "{}",
                ExperimentError::ManifestInvalid {
                    file,
                    key: key.clone(),
                    reason: reason.clone(),
                }
            );
            assert!(
                rendered.contains(LABEL) && rendered.contains(&key),
                "Display lost the file/key: {rendered}"
            );
        }
        Err(other) => panic!("untyped rejection for key {want_key:?}: {other:?}"),
    }
}

/// V1: the driver battery passes — valid manifests parse, malformed and
/// adversarial ones are rejected with typed errors.
#[test]
fn v_driver_battery_passes() {
    let evidence = run_pass();
    let ok = evidence.iter().filter(|l| l.starts_with("ok: ")).count();
    let rejected = evidence
        .iter()
        .filter(|l| l.starts_with("reject: "))
        .count();
    assert_eq!(ok, 4, "expected 4 valid-parse evidence lines: {evidence:?}");
    assert_eq!(
        rejected, 17,
        "expected 17 rejection evidence lines: {evidence:?}"
    );
    let joined = evidence.join("\n");
    for kind in ["suite", "budget", "promotion", "task"] {
        assert!(
            joined.contains(&format!("ok: {kind} manifest parses")),
            "no valid-parse evidence for {kind}: {joined}"
        );
    }
}

/// V2: valid manifests parse independently — including the manifests that
/// actually ship with phlow-experiment, so a contract drift in the real
/// files breaks this test instead of silently passing.
#[test]
fn v_valid_manifests_parse() {
    let suites = parse_suite_manifest(
        "schema_version = 1\n\n[[suite]]\nid = \"probe\"\ndescription = \"Probe.\"\nrequired = true\n",
        "task-16",
    )
    .expect("valid suite manifest rejected");
    assert_eq!(suites.suites.len(), 1);
    assert_eq!(suites.suites[0].id, "probe");
    assert!(suites.suites[0].required);

    let budgets = parse_budget_manifest(
        "schema_version = 1\n\n[defaults]\nworkers_max = 2\nqueue_capacity = 4\nchildren_per_task_max = 2\ndepth_max = 2\ntask_deadline_ms = 1000\ntool_calls_max = 8\noutput_bytes_max = 1024\nmemory_bytes_max = 4096\nmodel_turns_max = 3\nchanged_files_max = 2\nchanged_bytes_max = 2048\n",
        "task-16",
    )
    .expect("valid budget manifest rejected");
    assert_eq!(budgets.defaults.workers_max, 2);
    assert_eq!(budgets.defaults.depth_max, 2);

    let promotion = parse_promotion_manifest(
        "schema_version = 1\n\n[thresholds]\ncritical_safety_pass_pct = 100\nallow_new_failures_on_prior_successes = false\nhidden_holdout_gain_required = true\np95_latency_increase_pct_max = 10\ncost_increase_pct_max = 20\ncost_increase_requires_approval = true\nrollback_rehearsal_required = true\n",
        "task-16",
    )
    .expect("valid promotion manifest rejected");
    assert_eq!(promotion.thresholds.critical_safety_pass_pct, 100);

    let task = parse_task_manifest(
        "schema_version = 1\nid = \"t\"\nlanguage = \"rust\"\nkind = \"cli\"\nrisk = \"critical\"\nworkspace_fixture = \"f\"\ntask = \"T\"\n[budget]\nwall_ms = 1\nmodel_turns = 1\ntool_calls = 1\nchanged_files = 0\nchanged_bytes = 0\nworkers = 1\nqueue_depth = 1\n[[checks]]\nname = \"c\"\nargv = [\"true\"]\nrequired = false\nkind = \"test\"\n[acceptance]\nrequired_files = []\nforbidden_paths = []\nmax_new_dependencies = 0\n",
        "task-16",
    )
    .expect("valid task manifest rejected");
    assert_eq!(task.risk, RiskClass::Critical);

    // The shipped manifests must keep parsing against the current code.
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../phlow-experiment/manifests");
    let suites_text = std::fs::read_to_string(crate_dir.join("suites.toml"))
        .expect("cannot read shipped suites.toml");
    let shipped_suites =
        parse_suite_manifest(&suites_text, "shipped-suites").expect("shipped suites.toml rejected");
    assert_eq!(
        shipped_suites.suites.len(),
        6,
        "shipped suites.toml changed shape"
    );

    let budgets_text = std::fs::read_to_string(crate_dir.join("budgets.toml"))
        .expect("cannot read shipped budgets.toml");
    let shipped_budgets = parse_budget_manifest(&budgets_text, "shipped-budgets")
        .expect("shipped budgets.toml rejected");
    assert_eq!(shipped_budgets.defaults.workers_max, 4);

    let promotion_text = std::fs::read_to_string(crate_dir.join("promotion.toml"))
        .expect("cannot read shipped promotion.toml");
    let shipped_promotion = parse_promotion_manifest(&promotion_text, "shipped-promotion")
        .expect("shipped promotion.toml rejected");
    assert_eq!(shipped_promotion.thresholds.critical_safety_pass_pct, 100);

    let eval_text = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../phlow-experiment/evals/public/rust-cli-parse-001.toml"),
    )
    .expect("cannot read shipped eval manifest");
    let eval =
        parse_task_manifest(&eval_text, "shipped-eval").expect("shipped eval manifest rejected");
    assert_eq!(eval.id, "rust-cli-parse-001");
}

/// A1: type-confused inputs are rejected, never coerced. Each case is a
/// shape an attacker (or a sloppy generator) would actually produce:
/// version as string, budgets as floats, booleans as integers or strings,
/// negative integers, unknown enum spellings.
#[test]
fn a_type_confusion_rejected_not_coerced() {
    let suite = |t: &str, f: &'static str| parse_suite_manifest(t, f).map(|_| ());
    let budget = |t: &str, f: &'static str| parse_budget_manifest(t, f).map(|_| ());
    let task = |t: &str, f: &'static str| parse_task_manifest(t, f).map(|_| ());

    // schema_version = "1": a string must not coerce to the accepted integer.
    assert_rejected(
        "schema_version = \"1\"\n[[suite]]\nid = \"x\"\ndescription = \"d\"\nrequired = true\n",
        suite,
        "manifest.schema_version",
        "must be an integer",
    );
    // workers_max = 4.5: a float must not truncate.
    assert_rejected(
        "schema_version = 1\n[defaults]\nworkers_max = 4.5\n",
        budget,
        "defaults.workers_max",
        "must be an integer",
    );
    // workers_max = "4": numeric strings must not parse.
    assert_rejected(
        "schema_version = 1\n[defaults]\nworkers_max = \"4\"\n",
        budget,
        "defaults.workers_max",
        "must be an integer",
    );
    // required = 1: an integer must not stand in for a boolean.
    assert_rejected(
        "schema_version = 1\n[[suite]]\nid = \"x\"\ndescription = \"d\"\nrequired = 1\n",
        suite,
        "suite[0].required",
        "must be a boolean",
    );
    // required = "yes": truthy strings must not stand in for booleans.
    assert_rejected(
        "schema_version = 1\n[[suite]]\nid = \"x\"\ndescription = \"d\"\nrequired = \"yes\"\n",
        suite,
        "suite[0].required",
        "must be a boolean",
    );
    // workers_max = -5: negative budgets are rejected, not wrapped.
    assert_rejected(
        "schema_version = 1\n[defaults]\nworkers_max = -5\n",
        budget,
        "defaults.workers_max",
        "must not be negative",
    );
    // Empty id: the empty string is not a name.
    assert_rejected(
        "schema_version = 1\n[[suite]]\nid = \"\"\ndescription = \"d\"\nrequired = true\n",
        suite,
        "suite[0].id",
        "must not be empty",
    );
    // risk = "LOW": enum spellings are exact, not case-folded.
    assert_rejected(
        "schema_version = 1\nid = \"t\"\nlanguage = \"rust\"\nkind = \"cli\"\nrisk = \"LOW\"\nworkspace_fixture = \"f\"\ntask = \"T\"\n[budget]\nwall_ms = 1\nmodel_turns = 1\ntool_calls = 1\nchanged_files = 0\nchanged_bytes = 0\nworkers = 1\nqueue_depth = 1\n[[checks]]\nname = \"c\"\nargv = [\"true\"]\nrequired = false\nkind = \"test\"\n[acceptance]\nrequired_files = []\nforbidden_paths = []\nmax_new_dependencies = 0\n",
        task,
        "risk",
        "unknown risk class",
    );
}

/// A2: size bombs and injected keys are rejected, and every rejection
/// honors the error contract: the caller's file label, a non-empty key,
/// and a non-empty reason — so a misbehaving manifest can always be
/// located, never fails anonymously.
#[test]
fn a_size_bombs_and_injected_keys_rejected() {
    let suite = |t: &str, f: &'static str| parse_suite_manifest(t, f).map(|_| ());
    let budget = |t: &str, f: &'static str| parse_budget_manifest(t, f).map(|_| ());
    let task = |t: &str, f: &'static str| parse_task_manifest(t, f).map(|_| ());

    // 17 suites against the 16-suite bound: the array bound holds before
    // per-item work, so a flood of entries cannot exhaust memory.
    let mut bomb = String::from("schema_version = 1\n");
    for index in 0..17 {
        bomb.push_str(&format!(
            "\n[[suite]]\nid = \"s{index}\"\ndescription = \"d\"\nrequired = true\n"
        ));
    }
    assert_rejected(&bomb, suite, "suite", "at most 16 items allowed");

    // 129-char name against the 128-char bound.
    assert_rejected(
        &format!(
            "schema_version = 1\n[[suite]]\nid = \"{}\"\ndescription = \"d\"\nrequired = true\n",
            "n".repeat(129)
        ),
        suite,
        "suite[0].id",
        "at most 128 characters",
    );

    // 65 required_files against the 64-entry bound in a task manifest.
    let mut files = String::from(
        "schema_version = 1\nid = \"t\"\nlanguage = \"rust\"\nkind = \"cli\"\nrisk = \"normal\"\nworkspace_fixture = \"f\"\ntask = \"T\"\n[budget]\nwall_ms = 1\nmodel_turns = 1\ntool_calls = 1\nchanged_files = 0\nchanged_bytes = 0\nworkers = 1\nqueue_depth = 1\n[[checks]]\nname = \"c\"\nargv = [\"true\"]\nrequired = false\nkind = \"test\"\n[acceptance]\nrequired_files = [",
    );
    for index in 0..65 {
        if index > 0 {
            files.push_str(", ");
        }
        files.push_str(&format!("\"f{index}\""));
    }
    files.push_str("]\nforbidden_paths = []\nmax_new_dependencies = 0\n");
    assert_rejected(
        &files,
        task,
        "acceptance.required_files",
        "at most 64 items allowed",
    );

    // Injected unknown key inside a nested section: the stray key is named
    // with its full path, so it cannot smuggle configuration past review.
    assert_rejected(
        "schema_version = 1\n[[suite]]\nid = \"x\"\ndescription = \"d\"\nrequired = true\nevil = 1\n",
        suite,
        "suite[0].evil",
        "unknown key",
    );
    assert_rejected(
        "schema_version = 1\n[defaults]\nworkers_max = 4\nevil = 1\n",
        budget,
        "defaults.evil",
        "unknown key",
    );

    // Duplicate key: rejected at TOML parse time, still as the typed error.
    assert_rejected(
        "schema_version = 1\nschema_version = 1\n",
        suite,
        "document",
        "duplicate",
    );
}

/// Task metadata (ID/NAME/KIND) is intact.
#[test]
fn task_metadata_intact() {
    assert_eq!(task_16::ID, "task-16");
    assert_eq!(task_16::NAME, "manifest validation");
    assert!(matches!(task_16::KIND, TaskKind::Rust));
}
