//! The SkillOpt learning loop: rollout → reflect → propose → gate → apply.
//!
//! One [`Learner::run_arm`] call runs a full arm (all seeds) of an
//! ablation: fixed splits per seed, epochs of minibatch steps, the
//! D_sel acceptance gate, the rejected-edit buffer, and epoch-end
//! slow/meta updates. Every step is recorded in a [`SeedLog`] so tasks
//! compute their preregistered metrics from evidence, not summaries.
//!
//! The loop enforces the paper's structure (arXiv 2605.23904v2 §II):
//! per-step propose → merge → global rank → truncate to L_t → gate on
//! D_sel (strictly greater; ties rejected) → apply; epoch-end slow
//! update (replay under previous vs current skill, classify, write
//! longitudinal guidance to the protected section) and meta update
//! (per-category outcome stats, optimizer-private).

use super::doc::{Edit, Provenance, SkillDoc};
use super::gate::{GateMode, decide};
use super::optimizer::{
    CatStats, Optimizer, OptimizerError, ReflectCtx, TrajSummary, render_meta_canonical, sign_meta,
    verify_meta,
};
use super::rng::XorShift;
use super::target::{
    Family, MixedTarget, ScriptedTarget, SplitSpec, Splits, Target, TaskCase, initial_skill,
    is_canonical_line, make_mixed_splits, make_splits,
};
use std::collections::HashMap;
use std::fmt;

/// The textual learning rate schedule (paper §II.4).
#[derive(Debug, Clone, Copy)]
pub enum LtSchedule {
    /// Fixed bound every step.
    Constant(usize),
    /// Cosine decay from `from` to `to` over the arm's total steps.
    Cosine {
        /// Starting bound.
        from: usize,
        /// Ending bound.
        to: usize,
    },
    /// No bound: every proposed edit is applied, no gate (task-101's
    /// "removing the bound entirely" arm).
    Unbounded,
    /// The loop sets its own bound each step (task-113's
    /// "autonomous" schedule): a bounded additive controller on the
    /// trailing gate signal — after an accepted step the bound rises
    /// by one (up to `cap`), after a rejected step it falls by one
    /// (down to 1), starting at `cap`. The cap is the advertised
    /// ceiling; [`LtSchedule::bound`] returns it so the schedule
    /// still answers the pure query. This is task-113's
    /// operationalization of "autonomous": the design names the
    /// schedule but not the mechanism.
    Autonomous {
        /// Ceiling (and starting value) for the adaptive bound.
        cap: usize,
    },
}

impl LtSchedule {
    /// Bound for `step` of `total_steps`. `usize::MAX` = unbounded.
    /// For [`LtSchedule::Autonomous`] this returns the cap (the
    /// advertised ceiling); the adaptive per-step value is computed by
    /// the loop's controller, which sees the trailing gate signal that
    /// this pure query cannot.
    pub fn bound(&self, step: usize, total_steps: usize) -> usize {
        match *self {
            Self::Constant(n) => n,
            Self::Unbounded => usize::MAX,
            Self::Autonomous { cap } => cap.max(1),
            Self::Cosine { from, to } => {
                if total_steps <= 1 {
                    return from.max(1);
                }
                let t = (step.min(total_steps - 1) as f64) / (total_steps - 1) as f64;
                let cosine = (1.0 + (std::f64::consts::PI * t).cos()) / 2.0;
                let value = to as f64 + (from as f64 - to as f64) * cosine;
                (value.round() as usize).max(1)
            }
        }
    }

    /// One step of the autonomous controller (task-113): bounded
    /// additive increase/decrease on the trailing gate signal.
    ///
    /// - `None` (first step): start at the cap.
    /// - `Some(true)` (previous step accepted): raise by one, capped.
    /// - `Some(false)` (previous step rejected): lower by one, floored
    ///   at 1.
    ///
    /// Pure and total, so the task-113 audit can replay it from the
    /// step logs and the unit test can pin the boundary behavior.
    pub fn autonomous_next(cap: usize, current: usize, prev_accepted: Option<bool>) -> usize {
        let cap = cap.max(1);
        match prev_accepted {
            None => cap,
            Some(true) => (current + 1).min(cap),
            Some(false) => current.saturating_sub(1).max(1),
        }
    }
}

/// The rejected-edit buffer mode (task-103's dimension).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BufferMode {
    /// Record rejections and show them to the optimizer (the paper).
    Full,
    /// Record rejections but never show them (isolates recording from
    /// consulting).
    WriteOnly,
    /// No buffer at all.
    Off,
}

/// One rejected edit, keyed by direction.
#[derive(Debug, Clone)]
pub struct RejectedEntry {
    /// Step the rejection happened on.
    pub step: usize,
    /// Stable direction key.
    pub direction: String,
    /// Rendered edit (for reflection prompts).
    pub text: String,
}

/// The rejected-edit buffer: "the textual equivalent of negative-example
/// memory" (paper §II.5). Bounded; oldest entries drop first.
#[derive(Debug, Clone, Default)]
pub struct RejectedBuffer {
    entries: Vec<RejectedEntry>,
}

/// Buffer capacity in entries.
pub const REJECTED_BUFFER_CAP: usize = 512;

impl RejectedBuffer {
    /// What's visible to the optimizer under `mode`.
    pub fn readable(&self, mode: BufferMode) -> Vec<String> {
        match mode {
            BufferMode::Full => self.entries.iter().map(|e| e.direction.clone()).collect(),
            BufferMode::WriteOnly | BufferMode::Off => Vec::new(),
        }
    }

    /// Record a rejection (no-op under [`BufferMode::Off`]).
    pub fn record(&mut self, mode: BufferMode, step: usize, direction: &str, text: &str) {
        if mode == BufferMode::Off {
            return;
        }
        if self.entries.len() >= REJECTED_BUFFER_CAP {
            self.entries.remove(0);
        }
        self.entries.push(RejectedEntry {
            step,
            direction: direction.to_string(),
            text: text.to_string(),
        });
    }

    /// Number of recorded rejections.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Full arm configuration: one ablation cell.
#[derive(Debug, Clone)]
pub struct LearnerConfig {
    /// Families in the loop (1, or 2 for the task-102 mix).
    pub families: Vec<Family>,
    /// Split construction knobs.
    pub spec: SplitSpec,
    /// Use mixed-family splits.
    pub mixed: bool,
    /// Epochs (one pass over D_tr each).
    pub epochs: usize,
    /// Rollout batch size B.
    pub batch_size: usize,
    /// Reflection minibatch B_m (failures shown to the optimizer).
    pub reflect_minibatch: usize,
    /// Textual learning rate schedule.
    pub schedule: LtSchedule,
    /// Acceptance rule.
    pub gate: GateMode,
    /// Rejected-edit buffer mode.
    pub buffer: BufferMode,
    /// Epoch-end slow update on/off.
    pub slow: bool,
    /// Meta update on/off.
    pub meta: bool,
    /// Seeds (≥5 for the preregistered arms).
    pub seeds: Vec<u64>,
    /// D_tr fraction in (0, 1].
    pub d_tr_frac: f64,
    /// Trajectory poisoning rate in [0, 1] (task-110): fraction of
    /// rollout trajectories whose reflection summary is fabricated
    /// (misleading failure blaming the wrong family) or inflated (a
    /// failure reported as a success). 0.0 = no poisoning. The optimizer
    /// is NOT told; it must fall for the poisoned evidence.
    pub poison_rate: f64,
    /// Seal D_test during training (task-111). When true, the seed
    /// setup does NOT score D_test (the current scaffold otherwise
    /// reads it to populate the initial baseline — a training-time
    /// read the paper's protocol forbids); both D_test evaluations
    /// (initial skill and final skill) happen post-hoc in
    /// [`SeedState::seed_log`], after the last step. Every
    /// training-time D_test read is counted in
    /// [`SeedLog::d_test_reads`]: 0 means the seal held.
    pub sealed_d_test: bool,
    /// Three-split confirmation (task-111). When true, D_sel is split
    /// into two independent halves: D_selA (accept) and D_selB
    /// (confirm). A candidate is accepted only when strictly better on
    /// BOTH (with [`GateMode::Strict`]). The per-step B scores are
    /// recorded in [`StepRecord::confirm_before`]/`confirm_after`.
    pub confirm_split: bool,
    /// Poisoned slow update (task-114, adversarial). When true, the
    /// epoch-end slow update writes wrong `ORDER[p]:` guidance lines
    /// (rotated tool orders, the adversary's best shot at harmful
    /// ungated guidance) instead of KEEP lines.
    pub slow_update_poison: bool,
    /// Gated slow update (task-114). When true, the epoch-end slow
    /// update is a prototype gate: the candidate protected content is
    /// scored on D_sel against the current protected content, and
    /// harmful writes (candidate scores lower) are blocked.
    pub slow_update_gate: bool,
}

/// One recorded step.
#[derive(Debug, Clone)]
pub struct StepRecord {
    /// Global step index.
    pub step: usize,
    /// Epoch index.
    pub epoch: usize,
    /// L_t bound this step (`usize::MAX` = unbounded).
    pub l_t: usize,
    /// Proposals returned by the optimizer.
    pub n_proposed: usize,
    /// Edits applied (0 when rejected).
    pub n_applied: usize,
    /// Whether the candidate was accepted.
    pub accepted: bool,
    /// D_sel before the candidate.
    pub d_sel_before: f64,
    /// D_sel after the candidate (== before when rejected).
    pub d_sel_after: f64,
    /// Incremental per-edit D_sel deltas (edit i applied onto first i).
    pub per_edit_delta: Vec<f64>,
    /// Direction keys proposed this step.
    pub directions: Vec<String>,
    /// Chars added+deleted by applied edits.
    pub churn_chars: usize,
    /// Buffer suppressions this reflection (scripted mock only).
    pub buffer_suppressed: usize,
    /// Proposals this step whose direction was rejected within the
    /// previous three steps — counted for every buffer mode so task-103
    /// compares arms on equal footing.
    pub reproposal_within3: usize,
    /// Optimizer error text, if the step failed to propose.
    pub error: Option<String>,
    /// Largest per-edit token cost among this step's candidate edits
    /// (0 when the candidate was empty). Task-113's accounting
    /// invariant asserts this never exceeds
    /// [`super::doc::PER_EDIT_TOKENS_MAX`] on any step.
    pub max_edit_tokens: usize,
    /// D_selB score before the candidate (three-split confirmation,
    /// task-111). `None` unless `confirm_split` is on.
    pub confirm_before: Option<f64>,
    /// D_selB score after the candidate. `None` unless `confirm_split`.
    pub confirm_after: Option<f64>,
    /// Which component decided the step's fate (task-106 edit ledger):
    /// `accepted:gate-strict` / `accepted:gate-off` when the gate applied
    /// the candidate, `rejected:gate-strict` when the gate vetoed a
    /// well-formed candidate, `rejected:empty` when there was nothing to
    /// judge, `rejected:apply-failed` when the candidate did not apply
    /// cleanly.
    pub veto: String,
}

/// The full evidence of one seed.
#[derive(Debug, Clone)]
pub struct SeedLog {
    /// Seed.
    pub seed: usize,
    /// Per-step records.
    pub steps: Vec<StepRecord>,
    /// Final D_test fraction.
    pub d_test: f64,
    /// Initial D_test fraction (s_0) — the no-learning baseline.
    pub d_test_initial: f64,
    /// Final D_sel fraction.
    pub d_sel_final: f64,
    /// Initial D_sel fraction (s_0).
    pub d_sel_initial: f64,
    /// Final skill body (for retention/ledger inspection).
    pub final_body: String,
    /// Final protected section (KEEP/GUIDE lines). Together with
    /// `final_body` this is the exact frozen artifact bytes for
    /// cross-harness transfer (task-108).
    pub final_protected: String,
    /// Training-time D_test reads during this seed (task-111's access
    /// log). The current scaffold reads D_test once at seed setup to
    /// populate the initial baseline; with `sealed_d_test` that read is
    /// deferred to post-hoc evaluation and this is 0. Any future
    /// training-time read must increment this — the seal test fails
    /// closed if it is nonzero on a sealed arm.
    pub d_test_reads: u64,
    /// Canonical lines accepted during epoch 1 (retention baseline).
    pub epoch1_canonical: Vec<String>,
    /// Every accepted edit line across the run, in acceptance order
    /// (task-106 single-edit-gain analysis).
    pub accepted_edits: Vec<String>,
    /// Final meta category stats (task-114): the real helped/hurt
    /// record the tamper-evident signature covers.
    pub meta_cats: HashMap<String, CatStats>,
    /// Slow-update gate allows (task-114).
    pub slow_gate_allows: u64,
    /// Slow-update gate blocks (task-114).
    pub slow_gate_blocks: u64,
}

/// Learner failures: configuration or safety, never silent.
#[derive(Debug, Clone)]
pub enum LearnerError {
    /// Bad arm configuration.
    Config {
        /// What was wrong.
        detail: String,
    },
    /// Refusal: the document is not experiment-state. The loop never
    /// optimizes untrusted text.
    SafetyBoundary,
    /// The optimizer backend failed.
    Optimizer(OptimizerError),
    /// Tamper-evident meta record failed verification (task-114).
    /// Fail closed: the run aborts, never continues silently.
    MetaTampered {
        /// What failed to verify.
        detail: String,
    },
}

impl fmt::Display for LearnerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config { detail } => write!(f, "learner misconfigured: {detail}"),
            Self::SafetyBoundary => {
                write!(f, "learner refused: skill document is not experiment-state")
            }
            Self::Optimizer(e) => write!(f, "learner: optimizer failed: {e}"),
            Self::MetaTampered { detail } => {
                write!(f, "learner: meta record tampered, aborting: {detail}")
            }
        }
    }
}

impl std::error::Error for LearnerError {}

/// The preregistered verdict states (every task maps its measured
/// effect onto one of these; null/negative are successful measurements).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Effect in the paper's direction at or above the floor.
    Replicates,
    /// |effect| below the floor.
    Null,
    /// Effect in the opposite direction beyond noise.
    Negative,
    /// Between null and replicates bands (reported, not forced).
    Indeterminate,
}

impl fmt::Display for Verdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Replicates => write!(f, "replicates"),
            Self::Null => write!(f, "null"),
            Self::Negative => write!(f, "negative"),
            Self::Indeterminate => write!(f, "indeterminate"),
        }
    }
}

/// Mean and population std of a sample.
pub fn mean_std(values: &[f64]) -> (f64, f64) {
    if values.is_empty() {
        return (0.0, 0.0);
    }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let var = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64;
    (mean, var.sqrt())
}

/// The learning loop.
#[derive(Debug, Clone, Copy)]
pub struct Learner;

impl Learner {
    /// Run one arm (all seeds) from the family's initial skill.
    pub fn run_arm(
        &self,
        cfg: &LearnerConfig,
        optimizer: &dyn Optimizer,
    ) -> Result<Vec<SeedLog>, LearnerError> {
        let mut body = String::new();
        for family in &cfg.families {
            body.push_str(initial_skill(*family).body());
        }
        self.run_arm_with_skill(cfg, optimizer, SkillDoc::experiment(&body))
    }

    /// Run one arm from an explicit document. Refuses non-experiment
    /// documents ([`LearnerError::SafetyBoundary`]).
    pub fn run_arm_with_skill(
        &self,
        cfg: &LearnerConfig,
        optimizer: &dyn Optimizer,
        skill: SkillDoc,
    ) -> Result<Vec<SeedLog>, LearnerError> {
        validate(cfg)?;
        if skill.provenance() != Provenance::Experiment {
            return Err(LearnerError::SafetyBoundary);
        }
        let mut logs = Vec::with_capacity(cfg.seeds.len());
        for seed in cfg.seeds.iter() {
            logs.push(self.run_seed(cfg, optimizer, *seed, skill.clone())?);
        }
        Ok(logs)
    }

    /// One seed: fixed splits, epochs of minibatch steps, epoch-end
    /// slow/meta updates.
    /// One seed: fixed splits, epochs of minibatch steps, epoch-end
    /// slow/meta updates.
    fn run_seed(
        &self,
        cfg: &LearnerConfig,
        optimizer: &dyn Optimizer,
        seed: u64,
        skill: SkillDoc,
    ) -> Result<SeedLog, LearnerError> {
        let mut st = SeedState::new(cfg, seed, skill)?;
        // Same batches every epoch (fixed splits); precompute once.
        let batches: Vec<Vec<TaskCase>> = st
            .splits
            .d_tr
            .chunks(cfg.batch_size)
            .map(<[TaskCase]>::to_vec)
            .collect();
        for epoch in 0..cfg.epochs {
            let skill_start = st.skill.clone();
            st.epoch_accepted.clear();
            for batch in &batches {
                st.step(optimizer, batch, epoch)?;
            }
            st.epoch_end(epoch, &skill_start)?;
        }
        Ok(st.seed_log(seed))
    }
}

/// Mutable per-seed loop state. Keeps `run_seed` to orchestration and
/// each phase under the 70-line house rule.
struct SeedState<'a> {
    cfg: &'a LearnerConfig,
    /// The seed this state runs (task-110: binds attribution to a seed).
    seed: u64,
    splits: Splits,
    /// D_selB (confirm) cases when `confirm_split` is on; empty
    /// otherwise. `splits.d_sel` is then D_selA (accept).
    d_sel_b: Vec<TaskCase>,
    total_steps: usize,
    unbounded: bool,
    /// Current adaptive bound for [`LtSchedule::Autonomous`]
    /// (task-113). Initialized to the cap; the controller in
    /// [`SeedState::step`] moves it on the trailing gate signal.
    auto_l_t: usize,
    skill: SkillDoc,
    rng: XorShift,
    buffer: RejectedBuffer,
    /// Mode-independent (direction, step) log of rejected proposals —
    /// feeds task-103's re-proposal metric for every buffer mode.
    rejected_log: Vec<(String, usize)>,
    /// Per-category outcome stats (task-114: tamper-evident).
    meta_cats: HashMap<String, CatStats>,
    /// Slow-update gate decisions (task-114): allowed writes.
    slow_gate_allows: u64,
    /// Slow-update gate decisions (task-114): blocked writes.
    slow_gate_blocks: u64,
    epoch_accepted: Vec<String>,
    /// Every accepted edit line across the whole run (unlike
    /// `epoch_accepted`, never cleared) — feeds task-106's
    /// single-edit-gain analysis.
    all_accepted: Vec<String>,
    epoch_obs: Vec<(String, f64)>,
    steps: Vec<StepRecord>,
    step_idx: usize,
    epoch1_canonical: Vec<String>,
    d_sel_initial: f64,
    d_test_initial: f64,
    /// Training-time D_test reads (task-111's seal access log).
    d_test_reads: u64,
    /// The initial skill, retained iff `sealed_d_test` so the initial
    /// D_test baseline can be scored post-hoc (after training).
    initial_skill: Option<SkillDoc>,
}

/// A direction counts as recently rejected iff it was logged at or
/// after `lo`.
fn rejected_recently(log: &[(String, usize)], direction: &str, lo: usize) -> bool {
    log.iter().any(|(d, s)| d == direction && *s >= lo)
}

impl<'a> SeedState<'a> {
    fn new(cfg: &'a LearnerConfig, seed: u64, skill: SkillDoc) -> Result<Self, LearnerError> {
        let mut splits = if cfg.mixed {
            make_mixed_splits(seed, cfg.d_tr_frac, &cfg.spec)
        } else {
            make_splits(cfg.families[0], seed, cfg.d_tr_frac, &cfg.spec)
        };
        if splits.d_tr.is_empty() || splits.d_sel.is_empty() {
            return Err(LearnerError::Config {
                detail: "empty D_tr or D_sel".to_string(),
            });
        }
        // Three-split confirmation (task-111): D_selA accepts, D_selB
        // confirms. The halves are independent by construction (the
        // split is seeded); each half keeps the profile mix in
        // expectation. A degenerate half (fewer than 2 cases) cannot
        // confirm anything — fail the configuration instead.
        let d_sel_b = if cfg.confirm_split {
            if splits.d_sel.len() < 4 {
                return Err(LearnerError::Config {
                    detail: "confirm_split needs D_sel with at least 4 cases".to_string(),
                });
            }
            let mid = splits.d_sel.len() / 2;
            splits.d_sel.split_off(mid)
        } else {
            Vec::new()
        };
        let target = MixedTarget;
        let d_sel_initial = target.score(&skill, &splits.d_sel);
        // Sealed D_test (task-111): no training-time read. The initial
        // baseline is scored post-hoc in seed_log, after the last step;
        // the retained clone is the only D_test-adjacent state the
        // training loop carries.
        let (d_test_initial, d_test_reads, initial_skill) = if cfg.sealed_d_test {
            (f64::NAN, 0, Some(skill.clone()))
        } else {
            (target.score(&skill, &splits.d_test), 1, None)
        };
        let auto_l_t = match cfg.schedule {
            LtSchedule::Autonomous { cap } => cap.max(1),
            _ => 0,
        };
        Ok(Self {
            seed,
            total_steps: cfg.epochs * splits.d_tr.len().div_ceil(cfg.batch_size).max(1),
            unbounded: matches!(cfg.schedule, LtSchedule::Unbounded),
            auto_l_t,
            rng: XorShift::new(seed ^ 0x10EA_0001),
            cfg,
            splits,
            d_sel_b,
            skill,
            buffer: RejectedBuffer::default(),
            rejected_log: Vec::new(),
            meta_cats: HashMap::new(),
            slow_gate_allows: 0,
            slow_gate_blocks: 0,
            epoch_accepted: Vec::new(),
            all_accepted: Vec::new(),
            epoch_obs: Vec::new(),
            steps: Vec::new(),
            step_idx: 0,
            epoch1_canonical: Vec::new(),
            d_sel_initial,
            d_test_initial,
            d_test_reads,
            initial_skill,
        })
    }

    /// One minibatch step: roll out, reflect, gate, apply, record.
    fn step(
        &mut self,
        optimizer: &dyn Optimizer,
        batch: &[TaskCase],
        epoch: usize,
    ) -> Result<(), LearnerError> {
        // Task-113's autonomous schedule: the loop sets its own bound
        // from the trailing gate signal (bounded additive controller).
        // All other schedules answer the pure `bound()` query.
        let l_t = match self.cfg.schedule {
            LtSchedule::Autonomous { cap } => {
                let next = LtSchedule::autonomous_next(
                    cap,
                    self.auto_l_t,
                    self.steps.last().map(|r| r.accepted),
                );
                self.auto_l_t = next;
                next
            }
            _ => self.cfg.schedule.bound(self.step_idx, self.total_steps),
        };
        let (ctx, d_sel_before) = self.reflect(batch, l_t, epoch);
        let mut record = StepRecord {
            step: self.step_idx,
            epoch,
            l_t,
            n_proposed: 0,
            n_applied: 0,
            accepted: false,
            d_sel_before,
            d_sel_after: 0.0,
            per_edit_delta: Vec::new(),
            directions: Vec::new(),
            churn_chars: 0,
            buffer_suppressed: 0,
            reproposal_within3: 0,
            error: None,
            max_edit_tokens: 0,
            confirm_before: None,
            confirm_after: None,
            veto: String::new(),
        };
        match optimizer.propose(&self.skill, &ctx, &mut self.rng) {
            Ok(proposals) => self.apply_candidate(proposals, &mut record),
            Err(e) => {
                record.error = Some(e.to_string());
                record.d_sel_after = record.d_sel_before;
                if matches!(e, OptimizerError::Unavailable { .. }) {
                    return Err(LearnerError::Optimizer(e));
                }
            }
        }
        record.buffer_suppressed = optimizer.last_suppressed();
        self.steps.push(record);
        self.step_idx += 1;
        Ok(())
    }

    /// Task-110 trajectory poisoning. Each rollout trajectory is
    /// poisoned independently with probability `cfg.poison_rate`:
    ///
    /// - a *failure* becomes either a **misleading failure** (the trace
    ///   is rewritten to blame the other family — e.g. a bind failure
    ///   reported as an order failure, so the optimizer is pulled toward
    ///   the opposite fix) or a **lucky success** (the failure is
    ///   reported as `reward = 1`, hiding the signal entirely);
    /// - successes are never altered.
    ///
    /// The returned success count is post-poisoning (lucky successes
    /// inflate it). The optimizer is NOT told which summaries are
    /// poisoned — it must fall for them at the reflection stage; the
    /// test is whether the downstream gate and rejected-edit buffer
    /// contain the damage.
    fn poison_view(
        &mut self,
        batch: &[TaskCase],
        trajs: Vec<super::target::Trajectory>,
    ) -> (Vec<super::target::Trajectory>, usize) {
        if self.cfg.poison_rate <= 0.0 {
            let n_succ = trajs.iter().filter(|t| t.reward == 1).count();
            return (trajs, n_succ);
        }
        let mut n_succ = 0usize;
        let mut out = Vec::with_capacity(trajs.len());
        for mut t in trajs {
            if t.reward == 1 {
                n_succ += 1;
                out.push(t);
                continue;
            }
            if self.rng.next_f64() >= self.cfg.poison_rate {
                out.push(t);
                continue;
            }
            // Poison this failure: misleading blame or lucky success.
            if self.rng.next_f64() < 0.5 {
                let case = batch.iter().find(|c| c.id == t.case_id);
                let blamed = match case.map(|c| c.family) {
                    Some(Family::FBind) => Family::FOrder,
                    _ => Family::FBind,
                };
                t.poisoned = true;
                match blamed {
                    Family::FOrder => {
                        // A bind failure rewritten as an order failure:
                        // the emitted tool order is "wrong" for the
                        // blamed profile, so the mock proposes the
                        // opposite fix.
                        let p = (self.rng.next_u64() % 10) as u8;
                        t.expected = format!("ORDER[{p}]: ...");
                        t.got = format!("ORDER[{p}]: wrong order");
                    }
                    Family::FBind => {
                        // Shaped so the double reads it as a bind
                        // failure (its `got.starts_with('~')` cue):
                        // poisoned evidence blaming binding for what was
                        // really an order failure.
                        t.expected = "verbatim city span".to_string();
                        t.got = "~paraphrased span~".to_string();
                    }
                    Family::FLedger => {
                        t.expected = "ledger(h=?, twist=None)".to_string();
                        t.got = "broken order".to_string();
                    }
                }
                out.push(t);
            } else {
                // Lucky success: the failure is hidden from reflection.
                t.reward = 1;
                n_succ += 1;
                out.push(t);
            }
        }
        (out, n_succ)
    }

    /// Roll out the batch and build the reflection context.
    fn reflect(&mut self, batch: &[TaskCase], l_t: usize, epoch: usize) -> (ReflectCtx, f64) {
        let target = MixedTarget;
        let trajs: Vec<_> = batch
            .iter()
            .map(|c| ScriptedTarget::new(c.family).rollout(&self.skill, c))
            .collect();
        // Task-110: poison the reflection view BEFORE the minibatch take,
        // so poisoned summaries compete for the optimizer's attention
        // exactly like real evidence would.
        let (trajs, n_succ) = self.poison_view(batch, trajs);
        let fail: Vec<TrajSummary> = trajs
            .iter()
            .filter(|t| t.reward == 0)
            .take(self.cfg.reflect_minibatch)
            .map(|t| {
                let profile = batch
                    .iter()
                    .find(|c| c.id == t.case_id)
                    .map(|c| c.profile)
                    .unwrap_or(0);
                TrajSummary {
                    case_id: t.case_id,
                    profile,
                    expected: t.expected.clone(),
                    got: t.got.clone(),
                    poisoned: t.poisoned,
                }
            })
            .collect();
        let ctx = ReflectCtx {
            skill_text: self.skill.render_for_optimizer(),
            keep_lines: self.skill.keep_lines(),
            epoch_accepted_lines: self.epoch_accepted.clone(),
            n_succ,
            fail,
            rejected: self.buffer.readable(self.cfg.buffer),
            meta_text: render_meta(&self.meta_cats),
            meta_cats: self.meta_cats.clone(),
            l_t,
            step: self.step_idx,
            seed: self.seed as usize,
            epoch,
            families: self.cfg.families.clone(),
        };
        (ctx, target.score(&self.skill, &self.splits.d_sel))
    }

    /// Gate one candidate: probe per-edit deltas, decide, apply or
    /// record the rejection. Updates `record` in place.
    fn apply_candidate(&mut self, proposals: Vec<Edit>, record: &mut StepRecord) {
        let target = MixedTarget;
        record.n_proposed = proposals.len();
        record.directions = proposals.iter().map(|e| e.direction.clone()).collect();
        let lo = self.step_idx.saturating_sub(3);
        record.reproposal_within3 = record
            .directions
            .iter()
            .filter(|d| rejected_recently(&self.rejected_log, d, lo))
            .count();
        // Write-only means the optimizer never sees the rejections:
        // recording alone must not change behavior (the design isolates
        // consulting, not recording). The mode-independent rejected_log
        // still feeds the re-proposal metric.
        let take = if self.unbounded {
            proposals.len()
        } else {
            proposals.len().min(record.l_t)
        };
        let candidate: Vec<Edit> = proposals.into_iter().take(take).collect();
        // Task-113 accounting invariant, asserted on EVERY step: the
        // truncation above is the only path from proposals to applied
        // edits, so applied_ops ≤ L_t holds by construction; the assert
        // makes it a checked invariant rather than a convention.
        // (`record.l_t` is `usize::MAX` when unbounded.)
        assert!(
            candidate.len() <= record.l_t,
            "L_t truncation violated: {} > {}",
            candidate.len(),
            record.l_t
        );
        // Per-edit token accounting (task-113): the largest token cost
        // among this step's candidate edits, for the audit that every
        // step satisfies tokens_per_edit ≤ PER_EDIT_TOKENS_MAX.
        // `SkillDoc::apply` additionally rejects oversized payloads
        // with a typed error; this records what the step considered.
        record.max_edit_tokens = candidate.iter().map(Edit::tokens).max().unwrap_or(0);
        // Three-split confirmation (task-111): score the candidate on
        // D_selB alongside D_selA. Acceptance needs strict improvement
        // on both; the B scores are recorded for the confirmation
        // analysis either way.
        let confirm = self.cfg.confirm_split;
        let mut probe_b = self.skill.clone();
        let mut running_b = if confirm {
            target.score(&self.skill, &self.d_sel_b)
        } else {
            0.0
        };
        record.confirm_before = confirm.then_some(running_b);
        // Incremental per-edit D_sel deltas (evidence for meta and for
        // post-hoc harm measurement).
        let mut probe = self.skill.clone();
        let mut running = record.d_sel_before;
        for edit in &candidate {
            let before = running;
            if probe.apply(edit).is_ok() {
                running = target.score(&probe, &self.splits.d_sel);
                if confirm && probe_b.apply(edit).is_ok() {
                    running_b = target.score(&probe_b, &self.d_sel_b);
                }
            }
            record.per_edit_delta.push(running - before);
        }
        // The bound varies only the truncation: acceptance still goes
        // through the configured gate (task-101 isolates the bound;
        // task-102 isolates the gate). With three-split confirmation
        // (task-111) the gate must also clear D_selB: strictly better
        // on both, no partial credit.
        let accepted_a = if candidate.is_empty() {
            false
        } else if self.cfg.gate == GateMode::Off {
            true
        } else {
            decide(self.cfg.gate, record.d_sel_before, running)
        };
        let accepted = if confirm {
            accepted_a
                && decide(
                    self.cfg.gate,
                    record.confirm_before.unwrap_or(0.0),
                    running_b,
                )
        } else {
            accepted_a
        };
        record.confirm_after = confirm.then_some(running_b);
        // The candidate applies cleanly iff the probe did.
        let mut cand_skill = self.skill.clone();
        let applies = cand_skill.apply_all(&candidate).is_ok();
        // Task-106 ledger: name the deciding component for every step.
        record.veto = if candidate.is_empty() {
            "rejected:empty".to_string()
        } else if !applies {
            "rejected:apply-failed".to_string()
        } else if accepted {
            match self.cfg.gate {
                GateMode::Strict => "accepted:gate-strict".to_string(),
                GateMode::TieAccepts => "accepted:gate-tie-accepts".to_string(),
                GateMode::Off => "accepted:gate-off".to_string(),
            }
        } else {
            "rejected:gate-strict".to_string()
        };
        if accepted && applies {
            record.accepted = true;
            record.n_applied = candidate.len();
            record.d_sel_after = running;
            record.churn_chars = candidate.iter().map(|e| e.chars()).sum();
            for (edit, delta) in candidate.iter().zip(record.per_edit_delta.iter()) {
                self.epoch_obs.push((edit.category().to_string(), *delta));
                track_accepted_line(&mut self.epoch_accepted, edit);
                track_accepted_line(&mut self.all_accepted, edit);
            }
            self.skill = cand_skill;
        } else {
            record.d_sel_after = record.d_sel_before;
            for (edit, delta) in candidate.iter().zip(record.per_edit_delta.iter()) {
                self.buffer.record(
                    self.cfg.buffer,
                    self.step_idx,
                    &edit.direction,
                    &edit.render(),
                );
                self.rejected_log
                    .push((edit.direction.clone(), self.step_idx));
                self.epoch_obs
                    .push((edit.category().to_string(), delta.min(0.0)));
            }
        }
    }

    /// Epoch-end: snapshot epoch-1 canonical rules, then slow/meta
    /// updates (paper §II.6).
    /// Verify the persisted meta record (task-114). The skill doc's
    /// meta section carries the signed record from the previous
    /// epoch-end; a checksum mismatch means the persisted guidance was
    /// modified out-of-band. Fail closed: return
    /// [`LearnerError::MetaTampered`], never continue silently. A doc
    /// with no signed record yet (bootstrap) has nothing to verify.
    fn verify_meta_record(&self) -> Result<(), LearnerError> {
        let record = self.skill.meta();
        if !record.lines().any(|l| l.starts_with("SIG:")) {
            return Ok(());
        }
        verify_meta(record)
            .map(|_| ())
            .map_err(|e| LearnerError::MetaTampered {
                detail: format!("epoch-start meta verification failed: {e}"),
            })
    }

    fn epoch_end(&mut self, epoch: usize, skill_start: &SkillDoc) -> Result<(), LearnerError> {
        if epoch == 0 {
            self.epoch1_canonical = canonical_lines(&self.skill, &self.cfg.families);
        }
        // Tamper-evident meta (task-114): verify the record the
        // previous epoch-end persisted before doing anything else.
        self.verify_meta_record()?;
        if self.cfg.slow {
            let candidate = slow_update_lines(
                &self.skill,
                skill_start,
                &self.splits.d_sel,
                epoch,
                self.cfg.slow_update_poison,
            );
            if self.cfg.slow_update_gate {
                // Prototype gate: score the candidate protected content
                // on D_sel against the current content; block harmful
                // writes.
                let target = MixedTarget;
                let before = target.score(&self.skill, &self.splits.d_sel);
                let mut probe = self.skill.clone();
                probe.set_protected(&candidate.join("\n"));
                let after = target.score(&probe, &self.splits.d_sel);
                if after < before {
                    self.slow_gate_blocks += 1;
                } else {
                    self.slow_gate_allows += 1;
                    self.skill.set_protected(&candidate.join("\n"));
                }
            } else {
                self.skill.set_protected(&candidate.join("\n"));
            }
        }
        if self.cfg.meta {
            for (cat, delta) in self.epoch_obs.drain(..) {
                self.meta_cats.entry(cat).or_default().observe(delta);
            }
            let meta = sign_meta(&self.meta_cats);
            self.skill.set_meta(&meta);
        } else {
            self.epoch_obs.clear();
        }
        Ok(())
    }

    /// Assemble the seed evidence.
    fn seed_log(self, seed: u64) -> SeedLog {
        let target = MixedTarget;
        // Sealed D_test (task-111): the initial baseline is scored here,
        // after the last training step — never during training. The
        // training-time read count stays 0; these post-hoc evaluations
        // are the seal's one legitimate exception.
        let d_test_initial = match &self.initial_skill {
            Some(init) => target.score(init, &self.splits.d_test),
            None => self.d_test_initial,
        };
        SeedLog {
            seed: seed as usize,
            steps: self.steps,
            d_test: target.score(&self.skill, &self.splits.d_test),
            d_test_initial,
            d_sel_final: target.score(&self.skill, &self.splits.d_sel),
            d_sel_initial: self.d_sel_initial,
            final_body: self.skill.body().to_string(),
            final_protected: self.skill.protected().to_string(),
            epoch1_canonical: self.epoch1_canonical,
            accepted_edits: self.all_accepted,
            d_test_reads: self.d_test_reads,
            meta_cats: self.meta_cats,
            slow_gate_allows: self.slow_gate_allows,
            slow_gate_blocks: self.slow_gate_blocks,
        }
    }
}

/// Validate an arm configuration before running.
fn validate(cfg: &LearnerConfig) -> Result<(), LearnerError> {
    if cfg.families.is_empty() {
        return Err(LearnerError::Config {
            detail: "no families".to_string(),
        });
    }
    if cfg.epochs == 0 || cfg.batch_size == 0 || cfg.reflect_minibatch == 0 {
        return Err(LearnerError::Config {
            detail: "epochs/batch_size/reflect_minibatch must be > 0".to_string(),
        });
    }
    if cfg.seeds.is_empty() {
        return Err(LearnerError::Config {
            detail: "no seeds".to_string(),
        });
    }
    if !(0.0 < cfg.d_tr_frac && cfg.d_tr_frac <= 1.0) {
        return Err(LearnerError::Config {
            detail: "d_tr_frac must be in (0, 1]".to_string(),
        });
    }
    if !(0.0 <= cfg.poison_rate && cfg.poison_rate <= 1.0) {
        return Err(LearnerError::Config {
            detail: "poison_rate must be in [0, 1]".to_string(),
        });
    }
    if let LtSchedule::Constant(0) = cfg.schedule {
        return Err(LearnerError::Config {
            detail: "L_t constant 0 learns nothing".to_string(),
        });
    }
    if let LtSchedule::Autonomous { cap: 0 } = cfg.schedule {
        return Err(LearnerError::Config {
            detail: "autonomous cap 0 learns nothing".to_string(),
        });
    }
    Ok(())
}

/// Lines introduced by an accepted edit (recency prior).
fn track_accepted_line(accepted: &mut Vec<String>, edit: &super::doc::Edit) {
    use super::doc::EditOp;
    match &edit.op {
        EditOp::Append { line } => accepted.push(line.clone()),
        EditOp::InsertAfter { line, .. } => accepted.push(line.clone()),
        EditOp::Replace { new, .. } => accepted.extend(new.lines().map(str::to_string)),
        EditOp::Delete { .. } => {}
    }
}

/// Canonical (load-bearing) lines currently in the skill body.
fn canonical_lines(skill: &SkillDoc, families: &[Family]) -> Vec<String> {
    let mut out = Vec::new();
    for line in skill.body().lines() {
        if families.iter().any(|f| is_canonical_line(*f, line)) {
            out.push(line.to_string());
        }
    }
    out
}

/// Render meta category stats as optimizer-readable text.
/// Render meta category stats as optimizer-readable text (unsigned;
/// the persisted doc record uses [`sign_meta`]).
fn render_meta(cats: &HashMap<String, CatStats>) -> String {
    render_meta_canonical(cats)
}

/// Epoch-end slow update (paper §II.6): replay D_sel under the
/// epoch-start vs epoch-end skill, classify newly-fixed /
/// newly-regressed / consistently-correct / consistently-incorrect,
/// and write longitudinal guidance to the protected section:
/// KEEP lines for canonical rules behind newly-fixed cases, GUIDE
/// lines naming persistently-broken twist kinds.
/// Epoch-end slow update (paper §II.6), refactored for task-114 to
/// return the candidate protected lines instead of writing them: the
/// caller applies them directly, or scores them on D_sel first when
/// the prototype gate (`slow_update_gate`) is on. When `poison` is
/// true, the update writes the adversary's best shot at harmful
/// ungated guidance — wrong `ORDER[p]:` lines (rotated tool orders)
/// for every profile — instead of KEEP lines.
fn slow_update_lines(
    skill: &SkillDoc,
    prev: &SkillDoc,
    d_sel: &[TaskCase],
    epoch: usize,
    poison: bool,
) -> Vec<String> {
    let mut newly_fixed = 0usize;
    let mut newly_regressed = 0usize;
    let mut cons_correct = 0usize;
    let mut cons_wrong = 0usize;
    let mut twist_wrong = false;
    for case in d_sel {
        let r0 = ScriptedTarget::new(case.family).rollout(prev, case).reward;
        let r1 = ScriptedTarget::new(case.family).rollout(skill, case).reward;
        match (r0, r1) {
            (0, 1) => newly_fixed += 1,
            (1, 0) => newly_regressed += 1,
            (1, 1) => cons_correct += 1,
            _ => {
                cons_wrong += 1;
                if case.twist != super::target::Twist::None {
                    twist_wrong = true;
                }
            }
        }
    }
    // Carry existing guidance; add KEEP for canonical rules that are
    // new since the epoch start (they are behind the newly-fixed cases
    // by construction: nothing else changed the skill).
    let mut lines: Vec<String> = skill.protected().lines().map(str::to_string).collect();
    if poison {
        // Adversarial: the poisoned epoch-end batch. Wrong ORDER[p]
        // guidance for every profile (rotated tool orders, the same
        // wrong orders the optimizer's own distractors use). This is
        // the adversary's best shot at harmful ungated guidance: a
        // protected ORDER[p] line is the first match for any case
        // whose profile the body has not learned yet.
        for (p, req) in super::target::ORDER_REQUIRED.iter().enumerate() {
            let mut wrong: Vec<&str> = req.to_vec();
            wrong.rotate_left(1);
            lines.push(format!("ORDER[{p}]: {}", wrong.join(" ")));
        }
    } else {
        for line in skill.body().lines() {
            if super::target::is_canonical_line(Family::FOrder, line)
                || super::target::is_canonical_line(Family::FBind, line)
                || super::target::is_canonical_line(Family::FLedger, line)
            {
                let keep = format!("KEEP: {line}");
                if !prev.has_line(line) && !lines.iter().any(|l| l == &keep) {
                    lines.push(keep);
                }
            }
        }
        if twist_wrong
            && !lines
                .iter()
                .any(|l| l.starts_with("GUIDE:") && l.contains("twist"))
        {
            lines.push(
                "GUIDE: twist cases (empty-frontier, revisit) stay broken; propose dedicated LEDGER twist rules".to_string(),
            );
        }
    }
    lines.push(format!(
        "CYCLE {epoch}: newly_fixed={newly_fixed} newly_regressed={newly_regressed} consistent_ok={cons_correct} consistent_bad={cons_wrong}"
    ));
    // Bound the protected section: longitudinal memory, not a log.
    const PROTECTED_LINES_MAX: usize = 64;
    if lines.len() > PROTECTED_LINES_MAX {
        let drop = lines.len() - PROTECTED_LINES_MAX;
        lines.drain(0..drop);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::{BufferMode, Learner, LearnerConfig, LearnerError, LtSchedule, Verdict, mean_std};
    use crate::skillopt::doc::SkillDoc;
    use crate::skillopt::gate::GateMode;
    use crate::skillopt::optimizer::ScriptedOptimizer;
    use crate::skillopt::target::{Family, ProfileSkew, SplitSpec};

    fn arm(family: Family) -> LearnerConfig {
        LearnerConfig {
            families: vec![family],
            spec: SplitSpec {
                order_profiles: 10,
                skew: ProfileSkew::Uniform,
                n_tr: 40,
                n_sel: 20,
                n_test: 40,
                horizon_lo: 3,
                horizon_hi: 10,
            },
            mixed: false,
            epochs: 2,
            batch_size: 8,
            reflect_minibatch: 2,
            schedule: LtSchedule::Constant(4),
            gate: GateMode::Strict,
            buffer: BufferMode::Full,
            slow: true,
            meta: true,
            seeds: vec![101, 102, 103],
            d_tr_frac: 1.0,
            poison_rate: 0.0,
            sealed_d_test: false,
            confirm_split: false,
            slow_update_poison: false,
            slow_update_gate: false,
        }
    }

    /// Validation: a full arm improves D_test over s_0 on F-order.
    #[test]
    fn arm_improves_f_order() {
        let cfg = arm(Family::FOrder);
        let logs = Learner.run_arm(&cfg, &ScriptedOptimizer::new()).unwrap();
        assert_eq!(logs.len(), 3);
        for log in &logs {
            assert!(
                log.d_test >= log.d_test_initial,
                "seed {}: final d_test {} < initial d_test {} (same split)",
                log.seed,
                log.d_test,
                log.d_test_initial
            );
            assert!(!log.steps.is_empty());
        }
    }

    /// Validation: cosine schedule interpolates 4 -> 2.
    #[test]
    fn cosine_schedule() {
        let s = LtSchedule::Cosine { from: 4, to: 2 };
        assert_eq!(s.bound(0, 100), 4);
        assert_eq!(s.bound(99, 100), 2);
        let mid = s.bound(50, 100);
        assert!(mid == 2 || mid == 3, "mid = {mid}");
    }

    /// Validation: mean_std is exact on a known sample.
    #[test]
    fn mean_std_known() {
        let (m, sd) = mean_std(&[2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]);
        assert!((m - 5.0).abs() < 1e-12);
        assert!((sd - 2.0).abs() < 1e-12);
    }

    /// Validation: determinism — the same arm twice gives identical logs.
    #[test]
    fn arm_deterministic() {
        let cfg = arm(Family::FBind);
        let opt = ScriptedOptimizer::new();
        let (a, b) = (
            Learner.run_arm(&cfg, &opt).unwrap(),
            Learner.run_arm(&cfg, &opt).unwrap(),
        );
        for (x, y) in a.iter().zip(b.iter()) {
            assert_eq!(x.d_test, y.d_test);
            assert_eq!(x.steps.len(), y.steps.len());
        }
    }

    /// Adversarial: untrusted documents are refused before any rollout.
    #[test]
    fn untrusted_doc_refused() {
        let cfg = arm(Family::FOrder);
        let bad = SkillDoc::import_untrusted("ORDER[0]: fetch parse validate emit");
        let r = Learner.run_arm_with_skill(&cfg, &ScriptedOptimizer::new(), bad);
        assert!(
            matches!(r, Err(LearnerError::SafetyBoundary)),
            "untrusted doc must be refused, got {r:?}"
        );
    }

    /// Adversarial: degenerate configs fail loudly, not silently.
    #[test]
    fn degenerate_config_rejected() {
        let mut cfg = arm(Family::FOrder);
        cfg.schedule = LtSchedule::Constant(0);
        assert!(Learner.run_arm(&cfg, &ScriptedOptimizer::new()).is_err());
        let mut cfg = arm(Family::FOrder);
        cfg.seeds.clear();
        assert!(Learner.run_arm(&cfg, &ScriptedOptimizer::new()).is_err());
        let mut cfg = arm(Family::FOrder);
        cfg.d_tr_frac = 0.0;
        assert!(Learner.run_arm(&cfg, &ScriptedOptimizer::new()).is_err());
    }

    /// Adversarial: verdict display covers all preregistered states.
    #[test]
    fn verdicts_display() {
        for (v, s) in [
            (Verdict::Replicates, "replicates"),
            (Verdict::Null, "null"),
            (Verdict::Negative, "negative"),
            (Verdict::Indeterminate, "indeterminate"),
        ] {
            assert_eq!(v.to_string(), s);
        }
    }

    /// Task-114: the learner fails closed when the persisted meta
    /// record is tampered with. Uses private access to flip the doc's
    /// meta under a valid signature, then verifies the epoch-end check
    /// aborts with MetaTampered instead of continuing silently.
    #[test]
    fn tampered_meta_fails_closed() {
        use super::{sign_meta, verify_meta};
        use crate::skillopt::optimizer::CatStats;
        use std::collections::HashMap;
        let cfg = arm(Family::FOrder);
        let skill = SkillDoc::experiment("body");
        let mut st = super::SeedState::new(&cfg, 1, skill).unwrap();
        // Write a valid signed record.
        let mut cats = HashMap::new();
        cats.insert(
            "append".to_string(),
            CatStats {
                n: 5,
                mean: 0.05,
                var: 0.001,
            },
        );
        let record = sign_meta(&cats);
        st.skill.set_meta(&record);
        // Sanity: untampered verifies.
        assert!(st.verify_meta_record().is_ok());
        // Tamper: flip the mean, keep the signature.
        let sig = record.lines().last().unwrap();
        let tampered = format!("META: append n=5 mean=-0.050 var=0.001\n{sig}");
        st.skill.set_meta(&tampered);
        let err = st.verify_meta_record().unwrap_err();
        assert!(
            matches!(err, LearnerError::MetaTampered { .. }),
            "expected MetaTampered, got {err}"
        );
        // And the raw verify API agrees.
        assert!(verify_meta(&tampered).is_err());
    }

    /// Task-114: poisoned slow-update lines contain wrong ORDER[p]
    /// guidance for every profile (the adversary's best shot).
    #[test]
    fn poisoned_slow_update_writes_wrong_orders() {
        use super::slow_update_lines;
        let skill = SkillDoc::experiment("body");
        let prev = SkillDoc::experiment("body");
        let lines = slow_update_lines(&skill, &prev, &[], 0, true);
        let orders: Vec<&String> = lines.iter().filter(|l| l.starts_with("ORDER[")).collect();
        assert_eq!(orders.len(), 10);
        // None may match the canonical required orders.
        for (p, line) in orders.iter().enumerate() {
            let required = format!(
                "ORDER[{p}]: {}",
                crate::skillopt::target::ORDER_REQUIRED[p].join(" ")
            );
            assert_ne!(*line, &required, "poison wrote a correct line for {p}");
        }
    }
}
