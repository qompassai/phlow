//! Shared scaffolding for the SkillOpt task drivers (101–115).
//!
//! Each task driver resolves its optimizer backend (the real
//! [`ModelOptimizer`] against primo's Ollama when reachable, else the
//! clearly-labeled [`ScriptedOptimizer`] fallback), runs its arms through
//! [`Learner`], and classifies the preregistered verdict from the
//! [`SeedLog`] evidence. Thresholds stay in the task files — they are
//! the preregistered claims; this module only holds the mechanics.

use super::doc::SkillDoc;
use super::gate::GateMode;
use super::learner::{
    BufferMode, Learner, LearnerConfig, LearnerError, LtSchedule, SeedLog, Verdict, mean_std,
};
use super::optimizer::{ModelOptimizer, Optimizer, ScriptedOptimizer};
use super::target::{Family, ProfileSkew, SplitSpec};
use std::fmt;

/// Default evidence URL: primo's local Ollama. Override with
/// `GAUNTLET_OLLAMA_URL`.
pub const DEFAULT_OLLAMA_URL: &str = "http://127.0.0.1:11434";
/// Default evidence model. Override with `GAUNTLET_OLLAMA_MODEL`.
pub const DEFAULT_OLLAMA_MODEL: &str = "qwen3:8b";

/// Which optimizer backend produced the primary evidence.
pub enum Backend {
    /// The real model via primo's local Ollama HTTP API.
    Real(ModelOptimizer),
    /// Offline control only: the scripted mock, clearly labeled.
    ScriptedFallback(ScriptedOptimizer),
}

impl Backend {
    /// As a trait object for [`Learner::run_arm`].
    pub fn as_optimizer(&self) -> &dyn Optimizer {
        match self {
            Self::Real(m) => m,
            Self::ScriptedFallback(s) => s,
        }
    }

    /// Ledger label: `"real:qwen3:8b"` or `"scripted-fallback"`.
    pub fn label(&self) -> String {
        match self {
            Self::Real(_) => format!("real:{DEFAULT_OLLAMA_MODEL}"),
            Self::ScriptedFallback(_) => "scripted-fallback".to_string(),
        }
    }

    /// True only for the real model backend.
    pub fn is_real(&self) -> bool {
        matches!(self, Self::Real(_))
    }
}

/// Resolve the backend: real model when primo's Ollama serves it, else
/// the scripted fallback. Never fails: unavailability is a labeled
/// fallback, not an error.
pub fn resolve_backend() -> Backend {
    let url =
        std::env::var("GAUNTLET_OLLAMA_URL").unwrap_or_else(|_| DEFAULT_OLLAMA_URL.to_string());
    let model =
        std::env::var("GAUNTLET_OLLAMA_MODEL").unwrap_or_else(|_| DEFAULT_OLLAMA_MODEL.to_string());
    match ModelOptimizer::new(&url, &model) {
        Ok(m) if m.is_available() => Backend::Real(m),
        _ => Backend::ScriptedFallback(ScriptedOptimizer::new()),
    }
}

/// `GAUNTLET_REQUIRE_REAL=1` marks a run as primary evidence: the real
/// model is mandatory, and a fallback would be mislabeled evidence.
pub fn require_real() -> bool {
    std::env::var("GAUNTLET_REQUIRE_REAL").as_deref() == Ok("1")
}

/// Resolve the primary-evidence backend. When `GAUNTLET_REQUIRE_REAL=1`
/// (the primo evidence run) an unreachable model is a loud failure —
/// scripted output must never silently stand in as primary evidence.
pub fn resolve_primary() -> Result<Backend, String> {
    let backend = resolve_backend();
    if require_real() && !backend.is_real() {
        let url =
            std::env::var("GAUNTLET_OLLAMA_URL").unwrap_or_else(|_| DEFAULT_OLLAMA_URL.to_string());
        let model = std::env::var("GAUNTLET_OLLAMA_MODEL")
            .unwrap_or_else(|_| DEFAULT_OLLAMA_MODEL.to_string());
        return Err(format!(
            "GAUNTLET_REQUIRE_REAL=1 but the real model is unreachable at {url} \
             (model {model}); refusing to present scripted output as primary evidence"
        ));
    }
    Ok(backend)
}

/// The default split knobs shared by the wave. Tasks override fields.
pub fn default_spec() -> SplitSpec {
    SplitSpec {
        order_profiles: 10,
        skew: ProfileSkew::Uniform,
        n_tr: 40,
        n_sel: 20,
        n_test: 40,
        horizon_lo: 3,
        horizon_hi: 10,
    }
}

/// Five fixed seeds shared by the preregistered arms.
pub const SEEDS5: [u64; 5] = [101, 102, 103, 104, 105];
/// Three fixed seeds for the task-105 grid cells.
pub const SEEDS3: [u64; 3] = [201, 202, 203];

/// The default arm configuration. Tasks override the dimension under
/// test and keep everything else fixed (single-variable isolation).
pub fn base_config() -> LearnerConfig {
    LearnerConfig {
        families: vec![Family::FOrder],
        spec: default_spec(),
        mixed: false,
        epochs: 2,
        batch_size: 8,
        reflect_minibatch: 2,
        schedule: LtSchedule::Cosine { from: 4, to: 2 },
        gate: GateMode::Strict,
        buffer: BufferMode::Full,
        slow: true,
        meta: true,
        seeds: SEEDS5.to_vec(),
        d_tr_frac: 1.0,
    }
}

/// Mean ± std of final D_test over an arm's seeds, in points (×100).
#[derive(Debug, Clone)]
pub struct ArmSummary {
    /// Arm name.
    pub name: String,
    /// Seeds run.
    pub n: usize,
    /// Mean final D_test, in points.
    pub mean: f64,
    /// Std of final D_test, in points.
    pub std: f64,
}

/// Summarize one arm's seed logs.
pub fn summarize(name: &str, logs: &[SeedLog]) -> ArmSummary {
    let vals: Vec<f64> = logs.iter().map(|l| l.d_test * 100.0).collect();
    let (mean, std) = mean_std(&vals);
    ArmSummary {
        name: name.to_string(),
        n: logs.len(),
        mean,
        std,
    }
}

/// Run one named arm, mapping learner errors into driver errors.
pub fn run_arm_named(
    name: &str,
    cfg: &LearnerConfig,
    backend: &Backend,
) -> Result<(ArmSummary, Vec<SeedLog>), TaskDriverError> {
    let logs = Learner
        .run_arm(cfg, backend.as_optimizer())
        .map_err(|e| TaskDriverError::Arm {
            arm: name.to_string(),
            detail: e.to_string(),
        })?;
    if logs.is_empty() {
        return Err(TaskDriverError::Arm {
            arm: name.to_string(),
            detail: "no seed logs".to_string(),
        });
    }
    Ok((summarize(name, &logs), logs))
}

/// Driver failures: the apparatus broke (not a null/negative finding —
/// those are successful measurements reported as [`Verdict`]).
#[derive(Debug, Clone)]
pub enum TaskDriverError {
    /// An arm failed to run.
    Arm {
        /// Which arm.
        arm: String,
        /// What broke.
        detail: String,
    },
    /// A case fixture was unusable.
    Fixture {
        /// What was being built.
        what: String,
        /// The underlying error.
        detail: String,
    },
    /// The learner refused (safety boundary); only expected in the
    /// dedicated boundary cases.
    Safety(LearnerError),
}

impl fmt::Display for TaskDriverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Arm { arm, detail } => write!(f, "arm {arm} failed: {detail}"),
            Self::Fixture { what, detail } => write!(f, "fixture {what}: {detail}"),
            Self::Safety(e) => write!(f, "safety refusal: {e}"),
        }
    }
}

impl std::error::Error for TaskDriverError {}

/// The preregistered verdict, rendered for evidence lines.
pub fn verdict_line(task: &str, verdict: Verdict, detail: &str) -> String {
    format!("task-{task} verdict: {verdict} — {detail}")
}

/// Check a document built from untrusted text is refused by the loop.
/// Shared adversarial case logic for the wave.
pub fn untrusted_is_refused(cfg: &LearnerConfig) -> Result<(), String> {
    let bad = SkillDoc::import_untrusted("ORDER[0]: fetch parse validate emit");
    match Learner.run_arm_with_skill(cfg, &ScriptedOptimizer::new(), bad) {
        Err(LearnerError::SafetyBoundary) => Ok(()),
        Ok(_) => Err("untrusted document was optimized — boundary broken".to_string()),
        Err(e) => Err(format!("wrong error: {e}")),
    }
}

// ---------------------------------------------------------------------------
// Case reports (one shape for the whole SkillOpt wave)
// ---------------------------------------------------------------------------

/// The verdict of one driver case.
#[derive(Debug, Clone)]
pub struct CaseReport {
    /// Which case ran.
    pub case: String,
    /// Whether the case's own assertions held.
    pub passed: bool,
    /// Measured numbers.
    pub metrics: serde_json::Value,
    /// Diagnostic lines from the case.
    pub evidence: Vec<String>,
    /// Failing assertion details, empty when `passed`.
    pub failures: Vec<String>,
}

impl CaseReport {
    /// A passing case.
    pub fn pass(case: &'static str, metrics: serde_json::Value, evidence: Vec<String>) -> Self {
        Self {
            case: case.to_string(),
            passed: true,
            metrics,
            evidence,
            failures: Vec::new(),
        }
    }

    /// A failing case.
    pub fn fail(case: &'static str, failure: String, evidence: Vec<String>) -> Self {
        Self {
            case: case.to_string(),
            passed: false,
            metrics: serde_json::json!({}),
            evidence,
            failures: vec![failure],
        }
    }
}
