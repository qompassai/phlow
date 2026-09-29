//! task-276: two hundred options round trip (validation).
//!
//! Script the final valid index in 200 distinct options; retain all options and correct ID.

use super::task_254::support::{
    BTreeMap, Question, QuestionBatch, check_choice, decide_result, scripted_choice,
};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-276";
/// Concern exercised against the scripted System 1 backend.
pub const NAME: &str = "two hundred options round trip";
/// Offline Rust driver; no live model or HTTP service.
pub const KIND: TaskKind = TaskKind::Rust;

/// Run the one validation scenario, retaining errors as failed evidence.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    super::task_233::collect_outcome(
        ID,
        std::iter::once(("two_hundred_options_round_trip", case())),
    )
}

fn case() -> Result<bool, String> {
    const OPTION_COUNT: usize = 200;
    let options: Vec<String> = (0..OPTION_COUNT)
        .map(|index| format!("option-{index:03}"))
        .collect();
    let batch = QuestionBatch {
        state: "Select option-199.".to_owned(),
        questions: BTreeMap::from([(
            "q".to_owned(),
            Question::Choice {
                instructions: "Select the requested option.".to_owned(),
                options,
            },
        )]),
    };
    let backend = scripted_choice(OPTION_COUNT - 1, 0.99);
    let Ok(answer) = decide_result(&backend, &batch)? else {
        return Ok(false);
    };
    check_choice(&backend, &batch, &answer, OPTION_COUNT - 1, 0.99)
}

#[cfg(test)]
mod tests {
    #[ignore = "resume contract: 200-option protocol round trip (protocol not implemented)"]
    #[test]
    fn two_hundred_options_round_trip() {
        let result = super::case();
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
