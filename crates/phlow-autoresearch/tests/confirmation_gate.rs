//! Adversarial invariant, proven against the real artifact: a full
//! run of the autoresearch loop — keeps, discards, crashes and all —
//! leaves trainlab's confirmation gate exactly as it found it. The
//! loop has no code path that records a selection or opens the
//! confirmation split; this test makes that a checked fact rather
//! than a claim, using `phlow_trainlab::gate::ConfirmationGate`
//! itself as the oracle.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use phlow_autoresearch::changeset::{ChangeKind, ChangeSet};
use phlow_autoresearch::clock::ManualClock;
use phlow_autoresearch::evaluator::{Evaluation, ScriptedEvaluator, ScriptedOutcome};
use phlow_autoresearch::ledger::Ledger;
use phlow_autoresearch::proposer::{ScriptedProposal, ScriptedProposer};
use phlow_autoresearch::research_loop::{LoopConfig, run_loop};
use phlow_trainlab::gate::ConfirmationGate;

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_root(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "phlow-autoresearch-gate-{tag}-{}-{n}",
        std::process::id()
    ));
    std::fs::create_dir_all(&path).expect("create temp root");
    path
}

fn config_change_set(id: &str) -> ScriptedProposal {
    ScriptedProposal::ChangeSet(ChangeSet {
        id: id.to_string(),
        kind: ChangeKind::TrainlabConfig,
        paths: Vec::new(),
        payload: "{}".to_string(),
        rationale: "gate invariant probe".to_string(),
    })
}

#[test]
fn loop_run_never_touches_the_confirmation_gate() {
    let root = temp_root("confirm");
    let worktree = root.join("worktree");
    let ledger_dir = root.join("ledger");
    std::fs::create_dir_all(&worktree).expect("mkdir worktree");
    std::fs::create_dir_all(&ledger_dir).expect("mkdir ledger");

    // A fresh, real confirmation gate living next to the experiment.
    let gate_path = root.join("confirmation-gate.json");
    let gate_before = ConfirmationGate::load(&gate_path).expect("load fresh gate");
    let state_before = gate_before.state().clone();
    assert_eq!(state_before.confirm_open_count, 0);
    assert_eq!(state_before.selection_id, None);
    drop(gate_before);

    // A full loop run: baseline keep, improvement keep, one crash.
    let config = LoopConfig::new(worktree, ledger_dir.clone());
    let mut proposer = ScriptedProposer::new(vec![
        config_change_set("base"),
        config_change_set("better"),
        config_change_set("broken"),
    ]);
    let mut evaluator = ScriptedEvaluator::new(vec![
        ScriptedOutcome::Evaluation(Evaluation::metric_only(0.50, "baseline")),
        ScriptedOutcome::Evaluation(Evaluation::metric_only(0.61, "improvement")),
        ScriptedOutcome::Fail(
            phlow_autoresearch::FailureClass::EvaluationFailed,
            "sampler down".to_string(),
        ),
    ]);
    let mut ledger = Ledger::open(&ledger_dir).expect("open ledger");
    let clock = ManualClock::new();
    let report =
        run_loop(&config, &mut proposer, &mut evaluator, &mut ledger, &clock).expect("loop runs");
    assert_eq!(report.keeps, 2);
    assert_eq!(report.crashes, 1);
    drop(ledger);

    // The gate is byte-for-byte the same gate: no selection recorded,
    // confirmation never opened, and if a state file was materialized
    // at all it still describes a fresh gate.
    let gate_after = ConfirmationGate::load(&gate_path).expect("reload gate");
    assert_eq!(gate_after.state(), &state_before);
    assert_eq!(gate_after.state().confirm_open_count, 0);
    assert_eq!(gate_after.state().confirm_opened_for, None);

    let _ = std::fs::remove_dir_all(&root);
}
