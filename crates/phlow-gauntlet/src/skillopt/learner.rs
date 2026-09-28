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
use super::optimizer::{CatStats, Optimizer, OptimizerError, ReflectCtx, TrajSummary};
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
}

impl LtSchedule {
    /// Bound for `step` of `total_steps`. `usize::MAX` = unbounded.
    pub fn bound(&self, step: usize, total_steps: usize) -> usize {
        match *self {
            Self::Constant(n) => n,
            Self::Unbounded => usize::MAX,
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
    /// Canonical lines accepted during epoch 1 (retention baseline).
    pub epoch1_canonical: Vec<String>,
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
}

impl fmt::Display for LearnerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config { detail } => write!(f, "learner misconfigured: {detail}"),
            Self::SafetyBoundary => {
                write!(f, "learner refused: skill document is not experiment-state")
            }
            Self::Optimizer(e) => write!(f, "learner: optimizer failed: {e}"),
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
            st.epoch_end(epoch, &skill_start);
        }
        Ok(st.seed_log(seed))
    }
}

/// Mutable per-seed loop state. Keeps `run_seed` to orchestration and
/// each phase under the 70-line house rule.
struct SeedState<'a> {
    cfg: &'a LearnerConfig,
    splits: Splits,
    total_steps: usize,
    unbounded: bool,
    skill: SkillDoc,
    rng: XorShift,
    buffer: RejectedBuffer,
    /// Mode-independent (direction, step) log of rejected proposals —
    /// feeds task-103's re-proposal metric for every buffer mode.
    rejected_log: Vec<(String, usize)>,
    meta_cats: HashMap<String, CatStats>,
    epoch_accepted: Vec<String>,
    epoch_obs: Vec<(String, f64)>,
    steps: Vec<StepRecord>,
    step_idx: usize,
    epoch1_canonical: Vec<String>,
    d_sel_initial: f64,
    d_test_initial: f64,
}

/// A direction counts as recently rejected iff it was logged at or
/// after `lo`.
fn rejected_recently(log: &[(String, usize)], direction: &str, lo: usize) -> bool {
    log.iter().any(|(d, s)| d == direction && *s >= lo)
}

impl<'a> SeedState<'a> {
    fn new(cfg: &'a LearnerConfig, seed: u64, skill: SkillDoc) -> Result<Self, LearnerError> {
        let splits = if cfg.mixed {
            make_mixed_splits(seed, cfg.d_tr_frac, &cfg.spec)
        } else {
            make_splits(cfg.families[0], seed, cfg.d_tr_frac, &cfg.spec)
        };
        if splits.d_tr.is_empty() || splits.d_sel.is_empty() {
            return Err(LearnerError::Config {
                detail: "empty D_tr or D_sel".to_string(),
            });
        }
        let target = MixedTarget;
        let d_sel_initial = target.score(&skill, &splits.d_sel);
        let d_test_initial = target.score(&skill, &splits.d_test);
        Ok(Self {
            total_steps: cfg.epochs * splits.d_tr.len().div_ceil(cfg.batch_size).max(1),
            unbounded: matches!(cfg.schedule, LtSchedule::Unbounded),
            rng: XorShift::new(seed ^ 0x10EA_0001),
            cfg,
            splits,
            skill,
            buffer: RejectedBuffer::default(),
            rejected_log: Vec::new(),
            meta_cats: HashMap::new(),
            epoch_accepted: Vec::new(),
            epoch_obs: Vec::new(),
            steps: Vec::new(),
            step_idx: 0,
            epoch1_canonical: Vec::new(),
            d_sel_initial,
            d_test_initial,
        })
    }

    /// One minibatch step: roll out, reflect, gate, apply, record.
    fn step(
        &mut self,
        optimizer: &dyn Optimizer,
        batch: &[TaskCase],
        epoch: usize,
    ) -> Result<(), LearnerError> {
        let l_t = self.cfg.schedule.bound(self.step_idx, self.total_steps);
        let (ctx, d_sel_before) = self.reflect(batch, l_t);
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

    /// Roll out the batch and build the reflection context.
    fn reflect(&self, batch: &[TaskCase], l_t: usize) -> (ReflectCtx, f64) {
        let target = MixedTarget;
        let trajs: Vec<_> = batch
            .iter()
            .map(|c| ScriptedTarget::new(c.family).rollout(&self.skill, c))
            .collect();
        let n_succ = trajs.iter().filter(|t| t.reward == 1).count();
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
        // Incremental per-edit D_sel deltas (evidence for meta and for
        // post-hoc harm measurement).
        let mut probe = self.skill.clone();
        let mut running = record.d_sel_before;
        for edit in &candidate {
            let before = running;
            if probe.apply(edit).is_ok() {
                running = target.score(&probe, &self.splits.d_sel);
            }
            record.per_edit_delta.push(running - before);
        }
        // The bound varies only the truncation: acceptance still goes
        // through the configured gate (task-101 isolates the bound;
        // task-102 isolates the gate).
        let accepted = if candidate.is_empty() {
            false
        } else if self.cfg.gate == GateMode::Off {
            true
        } else {
            decide(self.cfg.gate, record.d_sel_before, running)
        };
        // The candidate applies cleanly iff the probe did.
        let mut cand_skill = self.skill.clone();
        let applies = cand_skill.apply_all(&candidate).is_ok();
        if accepted && applies {
            record.accepted = true;
            record.n_applied = candidate.len();
            record.d_sel_after = running;
            record.churn_chars = candidate.iter().map(|e| e.chars()).sum();
            for (edit, delta) in candidate.iter().zip(record.per_edit_delta.iter()) {
                self.epoch_obs.push((edit.category().to_string(), *delta));
                track_accepted_line(&mut self.epoch_accepted, edit);
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
    fn epoch_end(&mut self, epoch: usize, skill_start: &SkillDoc) {
        if epoch == 0 {
            self.epoch1_canonical = canonical_lines(&self.skill, &self.cfg.families);
        }
        if self.cfg.slow {
            slow_update(&mut self.skill, skill_start, &self.splits.d_sel, epoch);
        }
        if self.cfg.meta {
            for (cat, delta) in self.epoch_obs.drain(..) {
                self.meta_cats.entry(cat).or_default().observe(delta);
            }
            let meta = render_meta(&self.meta_cats);
            self.skill.set_meta(&meta);
        } else {
            self.epoch_obs.clear();
        }
    }

    /// Assemble the seed evidence.
    fn seed_log(self, seed: u64) -> SeedLog {
        let target = MixedTarget;
        SeedLog {
            seed: seed as usize,
            steps: self.steps,
            d_test: target.score(&self.skill, &self.splits.d_test),
            d_test_initial: self.d_test_initial,
            d_sel_final: target.score(&self.skill, &self.splits.d_sel),
            d_sel_initial: self.d_sel_initial,
            final_body: self.skill.body().to_string(),
            epoch1_canonical: self.epoch1_canonical,
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
    if let LtSchedule::Constant(0) = cfg.schedule {
        return Err(LearnerError::Config {
            detail: "L_t constant 0 learns nothing".to_string(),
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
fn render_meta(cats: &HashMap<String, CatStats>) -> String {
    if cats.is_empty() {
        return String::new();
    }
    let mut kinds: Vec<&String> = cats.keys().collect();
    kinds.sort();
    let parts: Vec<String> = kinds
        .iter()
        .map(|k| {
            let s = &cats[*k];
            format!("{} n={} mean={:+.3} var={:.3}", k, s.n, s.mean, s.var)
        })
        .collect();
    format!("META: {}", parts.join(" | "))
}

/// Epoch-end slow update (paper §II.6): replay D_sel under the
/// epoch-start vs epoch-end skill, classify newly-fixed /
/// newly-regressed / consistently-correct / consistently-incorrect,
/// and write longitudinal guidance to the protected section:
/// KEEP lines for canonical rules behind newly-fixed cases, GUIDE
/// lines naming persistently-broken twist kinds.
fn slow_update(skill: &mut SkillDoc, prev: &SkillDoc, d_sel: &[TaskCase], epoch: usize) {
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
    lines.push(format!(
        "CYCLE {epoch}: newly_fixed={newly_fixed} newly_regressed={newly_regressed} consistent_ok={cons_correct} consistent_bad={cons_wrong}"
    ));
    // Bound the protected section: longitudinal memory, not a log.
    const PROTECTED_LINES_MAX: usize = 64;
    if lines.len() > PROTECTED_LINES_MAX {
        let drop = lines.len() - PROTECTED_LINES_MAX;
        lines.drain(0..drop);
    }
    skill.set_protected(&lines.join("\n"));
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
}
