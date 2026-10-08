//! The loop itself: propose → validate → evaluate → score → record,
//! iterated under explicit budgets, with every outcome written to the
//! hash-chained ledger. This module owns the state machine and the
//! halt decisions; it contains no model, training, or git code — those
//! live behind the [`Proposer`] and [`Evaluator`] traits.
//!
//! Invariants the implementation maintains:
//! - the incumbent changes only at a recorded keep;
//! - a discard is normal science: it neither increments nor resets
//!   the consecutive-*failure* counter (only crashes increment it, and
//!   any completed evaluation resets it);
//! - a metric that is non-finite or backed by malformed evidence is a
//!   `metric_missing` failure, never a score;
//! - gate violations are counted across the whole ledger (including
//!   entries from earlier runs of a resumed ledger), and the second
//!   one halts the run.

use std::path::PathBuf;

use crate::changeset::{ChangeSet, validate_change_set};
use crate::clock::Clock;
use crate::error::{AutoresearchError, FailureClass};
use crate::evaluator::{Evaluation, Evaluator};
use crate::ledger::{Decision, EntryDraft, EntryKind, Ledger, is_sha256_hex};
use crate::proposer::{ProposeContext, ProposeError, Proposer};

/// Maximum iterations in one run of the loop.
pub const ITERATIONS_MAX: u32 = 32;
/// Default per-experiment wall-clock budget (15 minutes).
pub const PER_EXPERIMENT_WALL_CLOCK_MAX_MS: u64 = 900_000;
/// Default total wall-clock budget for one run (4 hours).
pub const TOTAL_RUN_WALL_CLOCK_MAX_MS: u64 = 14_400_000;
/// Hard ceiling for a configured per-experiment budget (1 hour).
pub const PER_EXPERIMENT_WALL_CLOCK_HARD_MAX_MS: u64 = 3_600_000;
/// Hard ceiling for a configured total budget (24 hours).
pub const TOTAL_RUN_WALL_CLOCK_HARD_MAX_MS: u64 = 86_400_000;
/// Default consecutive-failure budget.
pub const CONSECUTIVE_FAILURES_MAX: u32 = 3;
/// Gate violations that halt a run: the second one, always.
pub const GATE_VIOLATIONS_HALT: u32 = 2;
/// Default minimum improvement that counts as signal (pass@1 points).
pub const METRIC_EPSILON: f64 = 0.01;
/// Name of the operator stop file inside the ledger directory.
pub const STOP_FILE_NAME: &str = "STOP";

/// Configuration for one run of the loop. All budgets are explicit;
/// [`LoopConfig::validate`] enforces their bounds before anything runs.
#[derive(Debug, Clone, PartialEq)]
pub struct LoopConfig {
    /// Experiment worktree: the only root change-sets may touch.
    pub worktree_root: PathBuf,
    /// Directory of the hash-chained ledger (also holds `STOP`).
    pub ledger_dir: PathBuf,
    /// Iteration cap for this run, `1..=ITERATIONS_MAX`.
    pub iterations_max: u32,
    /// Per-experiment wall-clock budget in milliseconds.
    pub per_experiment_wall_clock_max_ms: u64,
    /// Total run wall-clock budget in milliseconds.
    pub total_run_wall_clock_max_ms: u64,
    /// Consecutive failed iterations that halt the run.
    pub consecutive_failures_max: u32,
    /// Minimum metric gain that counts as an improvement.
    pub metric_epsilon: f64,
}

impl LoopConfig {
    /// A configuration with the design's default budgets.
    #[must_use]
    pub fn new(worktree_root: PathBuf, ledger_dir: PathBuf) -> Self {
        LoopConfig {
            worktree_root,
            ledger_dir,
            iterations_max: ITERATIONS_MAX,
            per_experiment_wall_clock_max_ms: PER_EXPERIMENT_WALL_CLOCK_MAX_MS,
            total_run_wall_clock_max_ms: TOTAL_RUN_WALL_CLOCK_MAX_MS,
            consecutive_failures_max: CONSECUTIVE_FAILURES_MAX,
            metric_epsilon: METRIC_EPSILON,
        }
    }

    /// Check every bound before a run.
    ///
    /// # Errors
    /// [`AutoresearchError::Invalid`] or
    /// [`AutoresearchError::LimitExceeded`] naming the offending field.
    pub fn validate(&self) -> Result<(), AutoresearchError> {
        if self.iterations_max == 0 || self.iterations_max > ITERATIONS_MAX {
            return Err(AutoresearchError::LimitExceeded(format!(
                "iterations_max {} outside 1..={ITERATIONS_MAX}",
                self.iterations_max
            )));
        }
        if self.per_experiment_wall_clock_max_ms == 0
            || self.per_experiment_wall_clock_max_ms > PER_EXPERIMENT_WALL_CLOCK_HARD_MAX_MS
        {
            return Err(AutoresearchError::LimitExceeded(format!(
                "per-experiment budget {} ms outside 1..={PER_EXPERIMENT_WALL_CLOCK_HARD_MAX_MS}",
                self.per_experiment_wall_clock_max_ms
            )));
        }
        if self.total_run_wall_clock_max_ms < self.per_experiment_wall_clock_max_ms
            || self.total_run_wall_clock_max_ms > TOTAL_RUN_WALL_CLOCK_HARD_MAX_MS
        {
            return Err(AutoresearchError::LimitExceeded(format!(
                "total budget {} ms must be >= the per-experiment budget and <= {TOTAL_RUN_WALL_CLOCK_HARD_MAX_MS}",
                self.total_run_wall_clock_max_ms
            )));
        }
        if self.consecutive_failures_max == 0 || self.consecutive_failures_max > 16 {
            return Err(AutoresearchError::LimitExceeded(format!(
                "consecutive_failures_max {} outside 1..=16",
                self.consecutive_failures_max
            )));
        }
        if !self.metric_epsilon.is_finite() || !(0.0..=0.5).contains(&self.metric_epsilon) {
            return Err(AutoresearchError::Invalid(format!(
                "metric_epsilon {} outside 0.0..=0.5",
                self.metric_epsilon
            )));
        }
        if !self.worktree_root.is_dir() {
            return Err(AutoresearchError::Invalid(format!(
                "worktree root {} is not a directory",
                self.worktree_root.display()
            )));
        }
        Ok(())
    }
}

/// Why a run stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HaltReason {
    /// The iteration cap was reached.
    IterationBudget,
    /// The total wall-clock budget was reached.
    TotalWallClock,
    /// The consecutive-failure budget was reached.
    ConsecutiveFailures,
    /// The second gate violation occurred.
    GateViolation,
    /// The operator's `STOP` file was present.
    StopFile,
    /// The proposer reported no further experiments.
    NoMoreProposals,
}

impl HaltReason {
    /// Stable snake-case name, identical to the serialized form.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            HaltReason::IterationBudget => "iteration_budget",
            HaltReason::TotalWallClock => "total_wall_clock",
            HaltReason::ConsecutiveFailures => "consecutive_failures",
            HaltReason::GateViolation => "gate_violation",
            HaltReason::StopFile => "stop_file",
            HaltReason::NoMoreProposals => "no_more_proposals",
        }
    }

    /// The failure class recorded on the terminal ledger entry, if the
    /// halt is a failure-shaped stop.
    fn ledger_failure(self) -> Option<FailureClass> {
        match self {
            HaltReason::IterationBudget | HaltReason::TotalWallClock => {
                Some(FailureClass::BudgetExhausted)
            }
            HaltReason::ConsecutiveFailures => Some(FailureClass::BudgetExhausted),
            HaltReason::GateViolation => Some(FailureClass::GateViolation),
            HaltReason::StopFile | HaltReason::NoMoreProposals => None,
        }
    }
}

/// The outcome of one run of the loop.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct LoopReport {
    /// Iterations attempted this run (proposals that produced a
    /// ledger iteration entry, including failed ones).
    pub iterations_run: u32,
    /// Keeps this run (including a baseline keep).
    pub keeps: u32,
    /// Discards this run.
    pub discards: u32,
    /// Failed iterations this run.
    pub crashes: u32,
    /// Why the run stopped.
    pub halt_reason: HaltReason,
    /// The incumbent metric at the end of the run.
    pub incumbent_metric: Option<f64>,
    /// Ledger chain head after the terminal entry.
    pub ledger_head_sha256: Option<String>,
}

/// Mutable per-run bookkeeping, kept in one struct so the state
/// transitions stay auditable in one place.
struct RunState {
    incumbent_metric: Option<f64>,
    consecutive_failures: u32,
    gate_violations: u32,
    iterations_run: u32,
    keeps: u32,
    discards: u32,
    crashes: u32,
}

impl RunState {
    /// Record a failed iteration; return `Some(halt)` when a failure
    /// budget is now exhausted.
    fn record_failure(&mut self, class: FailureClass, config: &LoopConfig) -> Option<HaltReason> {
        self.iterations_run += 1;
        self.crashes += 1;
        self.consecutive_failures += 1;
        if class == FailureClass::GateViolation {
            self.gate_violations += 1;
            if self.gate_violations >= GATE_VIOLATIONS_HALT {
                return Some(HaltReason::GateViolation);
            }
        }
        if self.consecutive_failures >= config.consecutive_failures_max {
            return Some(HaltReason::ConsecutiveFailures);
        }
        None
    }

    /// Record a completed evaluation (keep or discard): the pipeline
    /// worked, so the consecutive-failure streak ends.
    fn record_completion(&mut self, kept: bool) {
        self.iterations_run += 1;
        self.consecutive_failures = 0;
        if kept {
            self.keeps += 1;
        } else {
            self.discards += 1;
        }
    }
}

/// Run the loop until a halt condition. Every iteration outcome —
/// keep, discard, crash — is appended to `ledger` before the next
/// iteration begins, and a terminal halt entry closes the run.
///
/// The ledger is assumed already opened (and therefore verified) by
/// the caller; a corrupt ledger never reaches this function.
///
/// # Errors
/// [`AutoresearchError`] on config validation failure or a ledger
/// write failure mid-run. All experiment-level failures are ledger
/// entries, not errors.
pub fn run_loop(
    config: &LoopConfig,
    proposer: &mut dyn Proposer,
    evaluator: &mut dyn Evaluator,
    ledger: &mut Ledger,
    clock: &dyn Clock,
) -> Result<LoopReport, AutoresearchError> {
    config.validate()?;
    let mut state = RunState {
        incumbent_metric: ledger.incumbent_metric(),
        consecutive_failures: 0,
        gate_violations: count_gate_violations(ledger),
        iterations_run: 0,
        keeps: 0,
        discards: 0,
        crashes: 0,
    };
    let run_started_ms = clock.now_ms();
    let mut halt_reason = HaltReason::IterationBudget;

    for _ in 0..config.iterations_max {
        if config.ledger_dir.join(STOP_FILE_NAME).exists() {
            halt_reason = HaltReason::StopFile;
            break;
        }
        if clock.now_ms().saturating_sub(run_started_ms) >= config.total_run_wall_clock_max_ms {
            halt_reason = HaltReason::TotalWallClock;
            break;
        }
        match run_one_iteration(config, proposer, evaluator, ledger, clock, &mut state)? {
            IterationFlow::Continue => {}
            IterationFlow::Halt(reason) => {
                halt_reason = reason;
                break;
            }
        }
        if state.iterations_run >= config.iterations_max {
            halt_reason = HaltReason::IterationBudget;
            break;
        }
    }

    ledger.append(EntryDraft {
        kind: EntryKind::Halt,
        change_set_id: String::new(),
        change_set_sha256: None,
        metric_before: state.incumbent_metric,
        metric_after: state.incumbent_metric,
        decision: Decision::Halt,
        failure: halt_reason.ledger_failure(),
        trainlab_receipt_sha256: None,
        trainer_receipt_sha256: None,
        wall_clock_ms: clock.now_ms().saturating_sub(run_started_ms),
        description: format!("run halted: {}", halt_reason.name()),
    })?;

    Ok(LoopReport {
        iterations_run: state.iterations_run,
        keeps: state.keeps,
        discards: state.discards,
        crashes: state.crashes,
        halt_reason,
        incumbent_metric: state.incumbent_metric,
        ledger_head_sha256: ledger.head_sha256().map(str::to_string),
    })
}

/// Whether the caller should continue or stop after one iteration.
enum IterationFlow {
    Continue,
    Halt(HaltReason),
}

/// One iteration: propose, validate, evaluate, score, record.
#[allow(clippy::too_many_arguments)]
fn run_one_iteration(
    config: &LoopConfig,
    proposer: &mut dyn Proposer,
    evaluator: &mut dyn Evaluator,
    ledger: &mut Ledger,
    clock: &dyn Clock,
    state: &mut RunState,
) -> Result<IterationFlow, AutoresearchError> {
    let context = ProposeContext {
        iteration: state.iterations_run + 1,
        incumbent_metric: state.incumbent_metric,
        ledger_head_sha256: ledger.head_sha256().map(str::to_string),
    };
    let change_set = match proposer.propose(&context) {
        Ok(change_set) => change_set,
        Err(ProposeError::Exhausted) => {
            return Ok(IterationFlow::Halt(HaltReason::NoMoreProposals));
        }
        Err(ProposeError::Invalid(message)) => {
            let halt = record_failure_entry(
                ledger,
                state,
                config,
                FailureClass::ProposalInvalid,
                String::new(),
                None,
                0,
                &message,
            )?;
            return Ok(halt.map_or(IterationFlow::Continue, IterationFlow::Halt));
        }
    };

    if let Err(surface) =
        validate_change_set(&change_set, &config.worktree_root, &config.ledger_dir)
    {
        let halt = record_failure_entry(
            ledger,
            state,
            config,
            surface.class,
            change_set.id.clone(),
            Some(change_set.sha256()),
            0,
            &surface.message,
        )?;
        return Ok(halt.map_or(IterationFlow::Continue, IterationFlow::Halt));
    }

    let started_ms = clock.now_ms();
    let result = evaluator.evaluate(&change_set, clock);
    let elapsed_ms = clock.now_ms().saturating_sub(started_ms);

    if elapsed_ms > config.per_experiment_wall_clock_max_ms {
        let halt = record_failure_entry(
            ledger,
            state,
            config,
            FailureClass::EvaluationTimeout,
            change_set.id.clone(),
            Some(change_set.sha256()),
            elapsed_ms,
            "per-experiment wall-clock budget exceeded",
        )?;
        return Ok(halt.map_or(IterationFlow::Continue, IterationFlow::Halt));
    }

    match result {
        Err(eval_error) => {
            let class = match eval_error.class {
                FailureClass::EvaluationFailed | FailureClass::EvaluationTimeout => {
                    eval_error.class
                }
                _ => FailureClass::EvaluationFailed,
            };
            let halt = record_failure_entry(
                ledger,
                state,
                config,
                class,
                change_set.id.clone(),
                Some(change_set.sha256()),
                elapsed_ms,
                &eval_error.message,
            )?;
            Ok(halt.map_or(IterationFlow::Continue, IterationFlow::Halt))
        }
        Ok(evaluation) => {
            score_evaluation(config, ledger, state, &change_set, &evaluation, elapsed_ms)
        }
    }
}

/// Validate an evaluation's evidence, then score it against the
/// incumbent and record the outcome.
fn score_evaluation(
    config: &LoopConfig,
    ledger: &mut Ledger,
    state: &mut RunState,
    change_set: &ChangeSet,
    evaluation: &Evaluation,
    elapsed_ms: u64,
) -> Result<IterationFlow, AutoresearchError> {
    if let Some(problem) = evidence_problem(evaluation) {
        let halt = record_failure_entry(
            ledger,
            state,
            config,
            FailureClass::MetricMissing,
            change_set.id.clone(),
            Some(change_set.sha256()),
            elapsed_ms,
            problem,
        )?;
        return Ok(halt.map_or(IterationFlow::Continue, IterationFlow::Halt));
    }
    let metric_before = state.incumbent_metric;
    let kept = match metric_before {
        None => true, // first completed evaluation establishes the baseline
        Some(incumbent) => evaluation.metric > incumbent + config.metric_epsilon,
    };
    if kept {
        state.incumbent_metric = Some(evaluation.metric);
    }
    state.record_completion(kept);
    ledger.append(EntryDraft {
        kind: EntryKind::Iteration,
        change_set_id: change_set.id.clone(),
        change_set_sha256: Some(change_set.sha256()),
        metric_before,
        metric_after: Some(evaluation.metric),
        decision: if kept {
            Decision::Keep
        } else {
            Decision::Discard
        },
        failure: None,
        trainlab_receipt_sha256: evaluation.trainlab_receipt_sha256.clone(),
        trainer_receipt_sha256: evaluation.trainer_receipt_sha256.clone(),
        wall_clock_ms: elapsed_ms,
        description: evaluation.description.clone(),
    })?;
    Ok(IterationFlow::Continue)
}

/// Append a crash entry for a failed iteration and update counters.
/// Returns the halt reason when a failure budget is now exhausted.
#[allow(clippy::too_many_arguments)]
fn record_failure_entry(
    ledger: &mut Ledger,
    state: &mut RunState,
    config: &LoopConfig,
    class: FailureClass,
    change_set_id: String,
    change_set_sha256: Option<String>,
    wall_clock_ms: u64,
    message: &str,
) -> Result<Option<HaltReason>, AutoresearchError> {
    let metric = state.incumbent_metric;
    ledger.append(EntryDraft {
        kind: EntryKind::Iteration,
        change_set_id,
        change_set_sha256,
        metric_before: metric,
        metric_after: None,
        decision: Decision::Crash,
        failure: Some(class),
        trainlab_receipt_sha256: None,
        trainer_receipt_sha256: None,
        wall_clock_ms,
        description: message.to_string(),
    })?;
    Ok(state.record_failure(class, config))
}

/// Why an evaluation's evidence is unusable, if it is.
fn evidence_problem(evaluation: &Evaluation) -> Option<&'static str> {
    if !evaluation.metric.is_finite() {
        return Some("evaluation metric is not finite");
    }
    for hash in [
        evaluation.trainlab_receipt_sha256.as_deref(),
        evaluation.trainer_receipt_sha256.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        if !is_sha256_hex(hash) {
            return Some("receipt hash in evaluation is not a SHA-256 hex digest");
        }
    }
    None
}

/// Gate violations already on record in a (possibly resumed) ledger.
fn count_gate_violations(ledger: &Ledger) -> u32 {
    let count = ledger
        .entries()
        .iter()
        .filter(|entry| entry.failure == Some(FailureClass::GateViolation))
        .count();
    u32::try_from(count).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changeset::ChangeKind;
    use crate::clock::ManualClock;
    use crate::evaluator::{ScriptedEvaluator, ScriptedOutcome};
    use crate::proposer::{ScriptedProposal, ScriptedProposer};
    use crate::testsupport::{TestDir, test_dir};

    struct Fixture {
        _root: TestDir,
        worktree: PathBuf,
        ledger_dir: PathBuf,
        config: LoopConfig,
        clock: ManualClock,
    }

    fn fixture() -> Fixture {
        let root = test_dir("loop");
        let worktree = root.path().join("worktree");
        let ledger_dir = root.path().join("ledger");
        std::fs::create_dir_all(worktree.join("src")).expect("mkdir worktree");
        std::fs::create_dir_all(&ledger_dir).expect("mkdir ledger");
        let config = LoopConfig::new(worktree.clone(), ledger_dir.clone());
        Fixture {
            _root: root,
            worktree,
            ledger_dir,
            config,
            clock: ManualClock::new(),
        }
    }

    fn config_cs(id: &str) -> ScriptedProposal {
        ScriptedProposal::ChangeSet(ChangeSet {
            id: id.to_string(),
            kind: ChangeKind::TrainlabConfig,
            paths: Vec::new(),
            payload: "{}".to_string(),
            rationale: "scripted".to_string(),
        })
    }

    fn patch_cs(id: &str, path: &str) -> ScriptedProposal {
        ScriptedProposal::ChangeSet(ChangeSet {
            id: id.to_string(),
            kind: ChangeKind::FilePatch,
            paths: vec![path.to_string()],
            payload: "diff".to_string(),
            rationale: "scripted".to_string(),
        })
    }

    fn metric(value: f64) -> ScriptedOutcome {
        ScriptedOutcome::Evaluation(Evaluation::metric_only(value, "scripted metric"))
    }

    fn run(
        fixture: &Fixture,
        proposals: Vec<ScriptedProposal>,
        outcomes: Vec<ScriptedOutcome>,
    ) -> (LoopReport, Ledger) {
        let mut proposer = ScriptedProposer::new(proposals);
        let mut evaluator = ScriptedEvaluator::new(outcomes);
        let mut ledger = Ledger::open(&fixture.ledger_dir).expect("open ledger");
        let report = run_loop(
            &fixture.config,
            &mut proposer,
            &mut evaluator,
            &mut ledger,
            &fixture.clock,
        )
        .expect("loop runs");
        (report, ledger)
    }

    #[test]
    fn baseline_then_improvement_keeps_and_advances() {
        let fixture = fixture();
        let (report, ledger) = run(
            &fixture,
            vec![config_cs("a"), config_cs("b")],
            vec![metric(0.50), metric(0.62)],
        );
        assert_eq!(report.keeps, 2);
        assert_eq!(report.incumbent_metric, Some(0.62));
        assert_eq!(report.halt_reason, HaltReason::NoMoreProposals);
        // 2 iteration entries + 1 halt entry, chain re-verifies.
        assert_eq!(ledger.len(), 3);
        let reopened = Ledger::open(&fixture.ledger_dir).expect("reopen");
        assert_eq!(reopened.incumbent_metric(), Some(0.62));
    }

    #[test]
    fn regression_and_sub_epsilon_gains_discard() {
        let fixture = fixture();
        let (report, _ledger) = run(
            &fixture,
            vec![config_cs("a"), config_cs("b"), config_cs("c")],
            vec![metric(0.50), metric(0.40), metric(0.505)],
        );
        assert_eq!(report.keeps, 1);
        assert_eq!(report.discards, 2);
        assert_eq!(report.incumbent_metric, Some(0.50));
    }

    #[test]
    fn iteration_budget_halts() {
        let mut fixture = fixture();
        fixture.config.iterations_max = 2;
        let proposals: Vec<ScriptedProposal> =
            (0..5).map(|i| config_cs(&format!("c{i}"))).collect();
        let outcomes: Vec<ScriptedOutcome> = (0..5).map(|i| metric(0.5 + f64::from(i))).collect();
        let (report, _ledger) = run(&fixture, proposals, outcomes);
        assert_eq!(report.iterations_run, 2);
        assert_eq!(report.halt_reason, HaltReason::IterationBudget);
    }

    #[test]
    fn total_wall_clock_budget_halts() {
        let mut fixture = fixture();
        fixture.config.total_run_wall_clock_max_ms = 1_000_000;
        let slow = ScriptedOutcome::AdvanceThenMetric {
            advance_ms: 600_000,
            metric: 0.5,
        };
        let (report, _ledger) = run(
            &fixture,
            vec![config_cs("a"), config_cs("b"), config_cs("c")],
            vec![slow.clone(), slow.clone(), slow],
        );
        assert_eq!(report.iterations_run, 2);
        assert_eq!(report.halt_reason, HaltReason::TotalWallClock);
    }

    #[test]
    fn per_experiment_timeout_is_a_failure_not_a_score() {
        let fixture = fixture();
        let slow = ScriptedOutcome::AdvanceThenMetric {
            advance_ms: PER_EXPERIMENT_WALL_CLOCK_MAX_MS + 1,
            metric: 0.99,
        };
        let (report, ledger) = run(
            &fixture,
            vec![config_cs("slow"), config_cs("base")],
            vec![slow, metric(0.50)],
        );
        assert_eq!(report.crashes, 1);
        assert_eq!(report.keeps, 1);
        assert_eq!(report.incumbent_metric, Some(0.50));
        assert_eq!(
            ledger.entries()[0].failure,
            Some(FailureClass::EvaluationTimeout)
        );
    }

    #[test]
    fn consecutive_crashes_halt_the_run() {
        let fixture = fixture();
        let fail = || ScriptedOutcome::Fail(FailureClass::EvaluationFailed, "boom".to_string());
        let (report, ledger) = run(
            &fixture,
            vec![
                config_cs("a"),
                config_cs("b"),
                config_cs("c"),
                config_cs("d"),
            ],
            vec![fail(), fail(), fail(), fail()],
        );
        assert_eq!(report.crashes, 3);
        assert_eq!(report.iterations_run, 3);
        assert_eq!(report.halt_reason, HaltReason::ConsecutiveFailures);
        assert_eq!(ledger.len(), 4); // 3 crashes + halt entry
    }

    #[test]
    fn two_gate_violations_halt_regardless_of_failure_budget() {
        let mut fixture = fixture();
        fixture.config.consecutive_failures_max = 16;
        let (report, _ledger) = run(
            &fixture,
            vec![
                patch_cs("e1", "../escape.rs"),
                patch_cs("e2", "crates/phlow-trainlab/src/lib.rs"),
                config_cs("never"),
            ],
            vec![metric(0.5)],
        );
        assert_eq!(report.crashes, 2);
        assert_eq!(report.halt_reason, HaltReason::GateViolation);
        // Nothing was ever written outside the worktree.
        assert!(!fixture.worktree.join("../escape.rs").exists());
    }

    #[test]
    fn nan_metric_is_metric_missing_never_a_keep() {
        let fixture = fixture();
        let (report, ledger) = run(
            &fixture,
            vec![config_cs("nan"), config_cs("base")],
            vec![metric(f64::NAN), metric(0.50)],
        );
        assert_eq!(report.crashes, 1);
        assert_eq!(report.incumbent_metric, Some(0.50));
        assert_eq!(
            ledger.entries()[0].failure,
            Some(FailureClass::MetricMissing)
        );
    }

    #[test]
    fn malformed_receipt_hash_is_metric_missing() {
        let fixture = fixture();
        let bad = ScriptedOutcome::Evaluation(Evaluation {
            metric: 0.9,
            trainlab_receipt_sha256: Some("not-a-hash".to_string()),
            trainer_receipt_sha256: None,
            description: "forged evidence".to_string(),
        });
        let (report, _ledger) = run(&fixture, vec![config_cs("x")], vec![bad]);
        assert_eq!(report.crashes, 1);
        assert_eq!(report.incumbent_metric, None);
    }

    #[test]
    fn invalid_proposal_counts_as_failure() {
        let fixture = fixture();
        let (report, ledger) = run(
            &fixture,
            vec![
                ScriptedProposal::Invalid("not json".to_string()),
                config_cs("base"),
            ],
            vec![metric(0.5)],
        );
        assert_eq!(report.crashes, 1);
        assert_eq!(report.keeps, 1);
        assert_eq!(
            ledger.entries()[0].failure,
            Some(FailureClass::ProposalInvalid)
        );
    }

    #[test]
    fn stop_file_halts_before_any_iteration() {
        let fixture = fixture();
        std::fs::write(fixture.ledger_dir.join(STOP_FILE_NAME), "").expect("write STOP");
        let (report, ledger) = run(&fixture, vec![config_cs("a")], vec![metric(0.5)]);
        assert_eq!(report.iterations_run, 0);
        assert_eq!(report.halt_reason, HaltReason::StopFile);
        assert_eq!(ledger.len(), 1); // halt entry only
    }

    #[test]
    fn resumed_ledger_restores_the_incumbent() {
        let fixture = fixture();
        let (first, _ledger) = run(&fixture, vec![config_cs("a")], vec![metric(0.50)]);
        assert_eq!(first.incumbent_metric, Some(0.50));
        let (second, ledger) = run(&fixture, vec![config_cs("b")], vec![metric(0.55)]);
        assert_eq!(second.keeps, 1);
        assert_eq!(second.incumbent_metric, Some(0.55));
        let keep_entry = ledger
            .entries()
            .iter()
            .rev()
            .find(|entry| entry.kind == EntryKind::Iteration)
            .expect("an iteration entry");
        assert_eq!(keep_entry.metric_before, Some(0.50));
    }

    #[test]
    fn evaluator_cannot_mint_taxonomy_entries() {
        let fixture = fixture();
        let forged = ScriptedOutcome::Fail(FailureClass::GateViolation, "trust me".to_string());
        let (report, ledger) = run(
            &fixture,
            vec![config_cs("a"), config_cs("b")],
            vec![forged, metric(0.5)],
        );
        assert_eq!(report.crashes, 1);
        assert_eq!(
            ledger.entries()[0].failure,
            Some(FailureClass::EvaluationFailed),
            "evaluator-supplied classes are coerced"
        );
    }
}
