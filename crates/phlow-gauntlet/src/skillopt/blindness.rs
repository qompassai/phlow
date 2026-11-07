//! The verifier-blindness pre-flight (D-1065, Harness2 §4.9).
//!
//! An improvement session (proposer/composer) works label-free: it
//! may see trajectories, diffs, and the loop's own operational
//! telemetry, but never the grader's material — no verdict, rubric,
//! or score artifact for the deliverable under improvement.
//! Harness2 enforces this mechanically (paper App. D.2: a guard
//! refuses to hand any session a working directory containing a
//! grader verdict, rubric, or score); this module is the estate's
//! version: a deterministic scan of the exact context about to be
//! handed over, refusing on a hit. It is a hygiene control, not a
//! performance claim (§4.9's falsifier: "none plausible").
//!
//! Contract:
//! - Input: a bundle of named text artifacts ([`BundleArtifact`]),
//!   or one rendered context string ([`scan_text`]).
//! - Output: `Ok(())` when no grader material is found;
//!   [`BlindnessError`] listing bounded hits otherwise. Refusal is
//!   fail-closed: an artifact too large to scan in full is refused,
//!   never partially certified.
//! - Detection is deliberately narrow (line-leading labels, quoted
//!   JSON keys, section headings, artifact names, two fixed
//!   phrases) so legitimate loop text — trajectory summaries,
//!   rollout counts, the signed optimizer-private meta — does not
//!   trip it. The scan covers the full rendered context, including
//!   the subject document: a skill that itself embeds a rubric
//!   section is refused, the conservative reading of App. D.2.
//!   Determined smuggling (encodings, synonyms) is out of scope:
//!   the guard exists to catch leakage, not to win an obfuscation
//!   contest.
//!
//! Bounds: at most [`ARTIFACTS_MAX`] artifacts, each at most
//! [`ARTIFACT_BYTES_MAX`] bytes; at most [`HITS_REPORTED_MAX`]
//! hits are reported per refusal.

use std::fmt;

/// Max artifacts in one bundle.
pub const ARTIFACTS_MAX: usize = 64;
/// Max bytes per artifact. The eval lane's contexts are
/// prompt-bounded; anything larger is outside the lane's contract
/// and is refused rather than partially scanned.
pub const ARTIFACT_BYTES_MAX: usize = 2 * 1024 * 1024;
/// Max hits reported in one refusal.
pub const HITS_REPORTED_MAX: usize = 8;
/// Max chars of the offending text kept in a hit's excerpt.
pub const EXCERPT_CHARS_MAX: usize = 80;

/// One named text artifact in an evidence bundle.
#[derive(Debug, Clone, Copy)]
pub struct BundleArtifact<'a> {
    /// Artifact name (path or label). Scanned as well as the
    /// content: a verdict file is grader material whatever it
    /// contains.
    pub name: &'a str,
    /// Artifact text.
    pub content: &'a str,
}

/// One grader-material hit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlindnessHit {
    /// Artifact the hit came from.
    pub artifact: String,
    /// 1-based line number; `None` for an artifact-name hit.
    pub line: Option<usize>,
    /// Which rule fired (a static label).
    pub marker: &'static str,
    /// The offending line (or name), trimmed and char-capped.
    pub excerpt: String,
}

/// Refusal: grader material was found, or the bundle exceeded the
/// scan bounds (refused rather than partially certified).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlindnessError {
    /// Hits, in scan order, capped at [`HITS_REPORTED_MAX`].
    pub hits: Vec<BlindnessHit>,
    /// True when further hits existed beyond the cap.
    pub more_hits: bool,
}

impl fmt::Display for BlindnessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "verifier-blindness refusal: {} grader-material hit(s)",
            self.hits.len()
        )?;
        if self.more_hits {
            write!(f, " (more suppressed)")?;
        }
        for hit in &self.hits {
            match hit.line {
                Some(line) => write!(
                    f,
                    "\n  {}:{} [{}] {}",
                    hit.artifact, line, hit.marker, hit.excerpt
                )?,
                None => write!(f, "\n  {} [{}] {}", hit.artifact, hit.marker, hit.excerpt)?,
            }
        }
        Ok(())
    }
}

impl std::error::Error for BlindnessError {}

/// Artifact-name tokens that mark grader material on their own.
const NAME_TOKENS: [&str; 10] = [
    "verdict", "verdicts", "rubric", "rubrics", "grader", "graders", "score", "scores", "grade",
    "grades",
];

/// Line-leading labels (followed, after optional whitespace, by
/// `:` or `=`) that mark grader output.
const LABELS: [&str; 15] = [
    "verdict",
    "verdicts",
    "final verdict",
    "judge verdict",
    "score",
    "scores",
    "final score",
    "overall score",
    "total score",
    "grade",
    "grades",
    "final grade",
    "rubric",
    "rubrics",
    "grader",
];

/// Quoted JSON keys that mark grader output.
const JSON_KEYS: [&str; 6] = [
    "\"verdict\"",
    "\"score\"",
    "\"final_score\"",
    "\"overall_score\"",
    "\"grade\"",
    "\"rubric\"",
];

/// Section-heading tokens that mark a grader section.
const HEADING_TOKENS: [&str; 7] = [
    "verdict", "rubric", "score", "scores", "grade", "grades", "grader",
];

/// Scan a whole evidence bundle. All hits found (up to the report
/// cap) are returned together, so one refusal shows the full
/// picture instead of dribbling out one leak at a time.
pub fn scan_bundle(artifacts: &[BundleArtifact]) -> Result<(), BlindnessError> {
    let mut hits: Vec<BlindnessHit> = Vec::new();
    let mut more_hits = false;
    if artifacts.len() > ARTIFACTS_MAX {
        push_hit(
            &mut hits,
            &mut more_hits,
            BlindnessHit {
                artifact: "<bundle>".to_string(),
                line: None,
                marker: "artifact-count-over-cap",
                excerpt: format!("{} artifacts > cap {ARTIFACTS_MAX}", artifacts.len()),
            },
        );
    }
    for artifact in artifacts.iter().take(ARTIFACTS_MAX) {
        scan_artifact(artifact, &mut hits, &mut more_hits);
    }
    if hits.is_empty() {
        Ok(())
    } else {
        Err(BlindnessError { hits, more_hits })
    }
}

/// Scan one rendered context string (e.g. an optimizer prompt)
/// under the synthetic artifact name `<rendered-context>`.
pub fn scan_text(text: &str) -> Result<(), BlindnessError> {
    scan_bundle(&[BundleArtifact {
        name: "<rendered-context>",
        content: text,
    }])
}

/// Append one hit, respecting the report cap.
fn push_hit(hits: &mut Vec<BlindnessHit>, more_hits: &mut bool, hit: BlindnessHit) {
    if hits.len() < HITS_REPORTED_MAX {
        hits.push(hit);
    } else {
        *more_hits = true;
    }
}

/// Scan one artifact: its name, its size, then its lines.
fn scan_artifact(artifact: &BundleArtifact, hits: &mut Vec<BlindnessHit>, more_hits: &mut bool) {
    if name_marker(artifact.name).is_some() {
        push_hit(
            hits,
            more_hits,
            BlindnessHit {
                artifact: artifact.name.to_string(),
                line: None,
                marker: "name",
                excerpt: excerpt(artifact.name),
            },
        );
    }
    if artifact.content.len() > ARTIFACT_BYTES_MAX {
        push_hit(
            hits,
            more_hits,
            BlindnessHit {
                artifact: artifact.name.to_string(),
                line: None,
                marker: "content-over-scan-cap",
                excerpt: format!(
                    "{} bytes > cap {ARTIFACT_BYTES_MAX}",
                    artifact.content.len()
                ),
            },
        );
        return;
    }
    for (idx, raw_line) in artifact.content.lines().enumerate() {
        if let Some(marker) = line_marker(raw_line) {
            push_hit(
                hits,
                more_hits,
                BlindnessHit {
                    artifact: artifact.name.to_string(),
                    line: Some(idx + 1),
                    marker,
                    excerpt: excerpt(raw_line),
                },
            );
        }
    }
}

/// Tokenize on non-alphanumeric boundaries, lowercased.
fn tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_ascii_lowercase())
        .collect()
}

/// Grader material by artifact name alone: a grader token, or an
/// `answer key` / `ground truth` token pair.
fn name_marker(name: &str) -> Option<()> {
    let toks = tokens(name);
    if toks.iter().any(|t| NAME_TOKENS.contains(&t.as_str())) {
        return Some(());
    }
    let pair = toks.windows(2).any(|w| {
        (w[0] == "answer" && (w[1] == "key" || w[1] == "keys"))
            || (w[0] == "ground" && w[1] == "truth")
    });
    if pair { Some(()) } else { None }
}

/// The first rule that fires on one line, if any.
fn line_marker(raw_line: &str) -> Option<&'static str> {
    let line = raw_line.trim();
    if line.is_empty() {
        return None;
    }
    let lower = line.to_ascii_lowercase();
    // Heading form: "# Scoring Rubric" — a grader section header.
    if lower.starts_with('#') {
        let toks = tokens(&lower);
        if toks.iter().any(|t| HEADING_TOKENS.contains(&t.as_str())) {
            return Some("heading");
        }
    }
    // Label form: "verdict: PASS", "score = 95" at line start
    // (whitespace before the separator is allowed; a following
    // word is not — "score is high" stays prose).
    for label in LABELS {
        if let Some(rest) = lower.strip_prefix(label) {
            let rest = rest.trim_start();
            if rest.starts_with(':') || rest.starts_with('=') {
                return Some(label);
            }
        }
    }
    // JSON key form: {"verdict": ...} anywhere in the line. The
    // leading quote anchors the key, so "final_score" does not
    // trip the "score" key.
    for key in JSON_KEYS {
        if let Some(pos) = lower.find(key) {
            let rest = lower[pos + key.len()..].trim_start();
            if rest.starts_with(':') {
                return Some("json-key");
            }
        }
    }
    // Phrase form: ground-truth / answer-key material named outright.
    if lower.contains("ground truth") || lower.contains("answer key") {
        return Some("phrase");
    }
    None
}

/// Trim and char-cap a line for a hit's excerpt.
fn excerpt(text: &str) -> String {
    text.trim().chars().take(EXCERPT_CHARS_MAX).collect()
}

#[cfg(test)]
mod tests {
    use super::{ARTIFACT_BYTES_MAX, BundleArtifact, HITS_REPORTED_MAX, scan_bundle, scan_text};

    fn artifact<'a>(name: &'a str, content: &'a str) -> BundleArtifact<'a> {
        BundleArtifact { name, content }
    }

    /// Validation: ordinary eval-lane text passes — trajectory
    /// summaries, rollout counts, and prose that merely mentions
    /// scoring mid-sentence.
    #[test]
    fn clean_bundle_passes() {
        let bundle = [
            artifact(
                "trajectory-notes.txt",
                "case 7: expected ORDER[3] fetch parse, got ORDER[2] fetch emit\n\
                 SUCCESSFUL ROLLOUTS: 5\n\
                 the scoring function rewards brevity in this skill\n\
                 scoreline stayed flat across the epoch",
            ),
            artifact("diff.patch", "- old line\n+ new line\n"),
        ];
        assert!(scan_bundle(&bundle).is_ok());
    }

    /// Validation: empty inputs pass.
    #[test]
    fn empty_inputs_pass() {
        assert!(scan_bundle(&[]).is_ok());
        assert!(scan_text("").is_ok());
        assert!(scan_bundle(&[artifact("notes.txt", "")]).is_ok());
    }

    /// Validation: the model reflection prompt's fixed template
    /// lines (optimizer::reflection_prompt) carry no grader
    /// material and must never trip the guard wired into propose.
    #[test]
    fn reflection_prompt_shape_passes() {
        let prompt = "PROTECTED GUIDANCE (read-only; never edit):\n\
                      (none)\n\
                      SUCCESSFUL ROLLOUTS: 3\n\
                      DO NOT PROPOSE THESE AGAIN (rejected earlier):\n\
                      - direction model:bind-quote\n\
                      EDIT FORMAT (one edit per line, EXACTLY one of these forms):\n\
                      Propose up to 6 edits. Reply with ONLY the edit lines, nothing else.";
        assert!(scan_text(prompt).is_ok());
    }

    /// Adversarial: a verdict file is refused by name alone,
    /// whatever its content.
    #[test]
    fn verdict_filename_refused() {
        let err = scan_bundle(&[artifact("run-7-verdict.txt", "all quiet here")]).unwrap_err();
        assert_eq!(err.hits.len(), 1);
        assert_eq!(err.hits[0].marker, "name");
        assert_eq!(err.hits[0].line, None);
    }

    /// Adversarial: an answer-key file is refused by name.
    #[test]
    fn answer_key_filename_refused() {
        let err = scan_bundle(&[artifact("answer-key.txt", "nothing to see")]).unwrap_err();
        assert_eq!(err.hits[0].marker, "name");
    }

    /// Adversarial: label forms are refused, with line numbers.
    #[test]
    fn label_forms_refused() {
        let err =
            scan_text("trajectory line\nVERDICT: PASS\n  score = 95\nFinal Grade: A").unwrap_err();
        assert_eq!(err.hits.len(), 3);
        assert_eq!(err.hits[0].line, Some(2));
        assert_eq!(err.hits[0].marker, "verdict");
        assert_eq!(err.hits[1].marker, "score");
        assert_eq!(err.hits[2].marker, "final grade");
    }

    /// Adversarial: mixed-case labels still fire.
    #[test]
    fn mixed_case_label_refused() {
        let err = scan_text("ScOrE: 12").unwrap_err();
        assert_eq!(err.hits[0].marker, "score");
    }

    /// Adversarial: a JSON grader record is refused.
    #[test]
    fn json_key_refused() {
        let err = scan_text("{\"verdict\": \"fail\", \"score\": 0.31}").unwrap_err();
        assert_eq!(err.hits[0].marker, "json-key");
    }

    /// Adversarial: a rubric section heading is refused.
    #[test]
    fn heading_refused() {
        let err = scan_text("## Scoring Rubric\n- completeness: weighted").unwrap_err();
        assert_eq!(err.hits[0].marker, "heading");
    }

    /// Adversarial: ground-truth material named in prose is
    /// refused (phrase form).
    #[test]
    fn phrase_refused() {
        let err =
            scan_text("compare the draft against the ground truth before proposing").unwrap_err();
        assert_eq!(err.hits[0].marker, "phrase");
    }

    /// Adversarial: the report is bounded — twenty violating lines
    /// yield exactly the cap, with the suppression flag set.
    #[test]
    fn hits_are_bounded() {
        let text = "verdict: x\n".repeat(20);
        let err = scan_text(&text).unwrap_err();
        assert_eq!(err.hits.len(), HITS_REPORTED_MAX);
        assert!(err.more_hits);
    }

    /// Adversarial: an artifact too large to scan in full is
    /// refused, never partially certified.
    #[test]
    fn oversize_refused() {
        let big = "x".repeat(ARTIFACT_BYTES_MAX + 1);
        let err = scan_bundle(&[artifact("trace.log", &big)]).unwrap_err();
        assert_eq!(err.hits[0].marker, "content-over-scan-cap");
    }
}
