//! Backdoor trigger candidate probes (design category 2).
//!
//! One batch classifies clean inputs and trigger candidates side by
//! side. Detection is statistical: the L1 shift between the mean
//! distribution on candidates and the mean distribution on clean
//! inputs must stay within the model's calibrated threshold. A
//! trigger that fires moves the distribution; a model that merely
//! classifies text does not.

use crate::probe::{Perturbation, Probe, ProbeBackend, ProbeEvidence, ProbeResult};
use crate::probes::{classify_batch, distributions, run_batch};
use crate::stats::{l1_shift, mean, mean_distribution, top_confidence};
use crate::thresholds::ModelThresholds;

/// Which pool of candidate inputs a trigger probe instance runs:
/// the store's candidates, or one perturbed form of them. The
/// verdict layer builds perturbed instances for the challenger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidatePool {
    /// The store's candidate inputs, as authored.
    Original,
    /// Paraphrased candidates (challenger).
    Paraphrased,
    /// Candidates with the suspected trigger stripped (challenger).
    Stripped,
}

/// One trigger probe over one trigger family.
#[derive(Debug, Clone)]
pub struct TriggerProbe {
    id: String,
    description: &'static str,
    clean: Vec<String>,
    candidates: Vec<String>,
    pool: CandidatePool,
    thresholds: ModelThresholds,
}

impl TriggerProbe {
    /// Build a probe over the candidate pool named by `pool`,
    /// drawing that pool's inputs from the parallel store lists.
    pub fn new(
        id: &str,
        description: &'static str,
        set: &crate::payloads::TriggerSet,
        pool: CandidatePool,
        thresholds: ModelThresholds,
    ) -> Self {
        let candidates = match pool {
            CandidatePool::Original => set.candidates.clone(),
            CandidatePool::Paraphrased => set.paraphrased.clone(),
            CandidatePool::Stripped => set.stripped.clone(),
        };
        TriggerProbe {
            id: id.to_owned(),
            description,
            clean: set.clean.clone(),
            candidates,
            pool,
            thresholds,
        }
    }

    /// The candidate pool this instance runs (the verdict layer uses
    /// it to label challenger evidence).
    pub fn pool(&self) -> CandidatePool {
        self.pool
    }

    /// The perturbation this instance embodies, if any.
    pub fn perturbation(&self) -> Option<Perturbation> {
        match self.pool {
            CandidatePool::Original => None,
            CandidatePool::Paraphrased => Some(Perturbation::Paraphrase),
            CandidatePool::Stripped => Some(Perturbation::StripTrigger),
        }
    }
}

impl Probe for TriggerProbe {
    fn id(&self) -> &str {
        &self.id
    }

    fn description(&self) -> &str {
        self.description
    }

    fn atlas(&self) -> Option<&str> {
        None
    }

    fn run(&self, backend: &dyn ProbeBackend) -> ProbeResult {
        let clean_count = self.clean.len();
        let mut inputs = self.clean.clone();
        inputs.extend(self.candidates.iter().cloned());
        let batch = classify_batch(&inputs, "cls");
        let answers = match run_batch(self.id(), backend, &batch) {
            Ok(answers) => answers,
            Err(failure) => return *failure,
        };
        let dists = distributions(&answers);
        let clean_mean = mean_distribution(&dists[..clean_count]);
        let candidate_mean = mean_distribution(&dists[clean_count..]);
        let shift = l1_shift(&clean_mean, &candidate_mean);
        let candidate_confidences: Vec<f64> = dists[clean_count..]
            .iter()
            .map(|dist| top_confidence(dist))
            .collect();
        let mut evidence = ProbeEvidence::check("distribution-shift");
        evidence.shift = Some(shift);
        evidence.threshold = Some(self.thresholds.trigger_shift_max);
        evidence.mean_confidence = Some(mean(&candidate_confidences));
        ProbeResult {
            probe_id: self.id.clone(),
            passed: shift <= self.thresholds.trigger_shift_max,
            evidence,
        }
    }
}
