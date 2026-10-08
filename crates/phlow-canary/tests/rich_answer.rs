//! Live-path regression tests: the battery consumes phlow-system1's
//! real `RichAnswer` type end-to-end (the canary-local stand-in is
//! gone), distributions are validated before statistics trust them,
//! and a backend that cannot produce a distribution fails closed.

use phlow_canary::probe::{ProbeResult, check_rich_answer};
use phlow_system1::{Answer, Question, System1Error};

fn choice_question() -> Question {
    Question::Choice {
        instructions: "Classify the sentiment.".to_owned(),
        options: vec![
            "negative".to_owned(),
            "neutral".to_owned(),
            "positive".to_owned(),
        ],
    }
}

fn rich(distribution: Vec<f64>) -> phlow_system1::RichAnswer {
    phlow_system1::RichAnswer {
        answer: Answer::Choice {
            selected: 2,
            probability: 0.9,
        },
        distribution,
    }
}

// --- Validation: the type identity and the honest path ---

#[test]
fn canary_rich_answer_is_the_system1_type() {
    // Compile-time identity: a value built as phlow_system1::RichAnswer
    // is accepted where phlow_canary names RichAnswer. If the local
    // stand-in ever returns, this test stops compiling.
    let value = rich(vec![0.05, 0.05, 0.9]);
    let _: phlow_canary::RichAnswer = value.clone();
    let _: phlow_canary::RichAnswerBatch = phlow_system1::RichAnswerBatch {
        answers: [("q-0".to_owned(), value)].into_iter().collect(),
    };
}

#[test]
fn valid_distribution_passes_check() {
    assert!(check_rich_answer(&choice_question(), &rich(vec![0.05, 0.05, 0.9])).is_ok());
}

// --- Adversarial: malformed distributions fail closed ---

#[test]
fn distribution_not_summing_to_one_fails() {
    assert!(check_rich_answer(&choice_question(), &rich(vec![0.2, 0.2, 0.1])).is_err());
}

#[test]
fn non_finite_or_out_of_range_distribution_fails() {
    for distribution in [
        vec![0.05, f64::NAN, 0.9],
        vec![0.05, f64::INFINITY, 0.9],
        vec![-0.1, 0.2, 0.9],
        vec![0.05, 0.05, 1.2],
    ] {
        assert!(check_rich_answer(&choice_question(), &rich(distribution)).is_err());
    }
}

#[test]
fn wrong_length_distribution_fails() {
    assert!(check_rich_answer(&choice_question(), &rich(vec![0.1, 0.9])).is_err());
    assert!(check_rich_answer(&choice_question(), &rich(vec![])).is_err());
}

#[test]
fn distribution_unavailable_backend_error_fails_closed() {
    let result = ProbeResult::backend_error(
        "calibration.bimodal-001",
        &System1Error::DistributionUnavailable {
            reason: "backend exposes scalar answers only",
        },
    );
    assert!(!result.passed);
    assert_eq!(
        result.evidence.error.as_deref(),
        Some("distribution_unavailable")
    );
}
