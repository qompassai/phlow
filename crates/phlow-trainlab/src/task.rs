//! Synthetic coding-task families and frozen split generation.
//!
//! A direct port of the scheme in `glm53_flash/tasks.py` from the
//! companion repo: eight function families with hidden cases, and
//! deterministic per-split task generation whose entry-point names are
//! unseen in every other split. The seed *scheme* mirrors the Python
//! (per-split base seeds, index/family mixing); the PRNG differs
//! (SplitMix64 vs Python's Mersenne Twister), so generated names are
//! reproducible within phlow but are not byte-identical to the
//! Python run — the invariant that matters (frozen, disjoint splits)
//! is preserved.

use serde::Serialize;
use serde_json::{Value, json};

use crate::error::TrainlabError;
use crate::rng::SplitMix64;

/// Maximum tasks generated for one split in a single call.
pub const TASKS_PER_SPLIT_MAX: usize = 512;
/// Maximum tasks per family in one split.
pub const PER_FAMILY_MAX: usize = 64;
/// Entry-point name suffix length, as in the Python scheme.
const NAME_SUFFIX_LEN: usize = 7;

/// A frozen evaluation split. Names and base seeds mirror the Python:
/// dev 1701, rl 3907, final 2909, confirm 8123.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Split {
    /// Development split: checkpoint/hyperparameter selection happens here.
    Dev,
    /// Reinforcement-learning prompt split.
    Rl,
    /// Final reporting split.
    Final,
    /// Confirmation split: opened once, after selection (see
    /// [`crate::gate`]).
    Confirm,
}

impl Split {
    /// Parse a split name; unknown names are an error, never a guess.
    pub fn parse(name: &str) -> Result<Split, TrainlabError> {
        match name {
            "dev" => Ok(Split::Dev),
            "rl" => Ok(Split::Rl),
            "final" => Ok(Split::Final),
            "confirm" => Ok(Split::Confirm),
            other => Err(TrainlabError::UnknownSplit(other.to_string())),
        }
    }

    /// The split's wire name.
    pub fn name(self) -> &'static str {
        match self {
            Split::Dev => "dev",
            Split::Rl => "rl",
            Split::Final => "final",
            Split::Confirm => "confirm",
        }
    }

    /// The split's base seed from the Python scheme.
    pub fn base_seed(self) -> u64 {
        match self {
            Split::Dev => 1701,
            Split::Rl => 3907,
            Split::Final => 2909,
            Split::Confirm => 8123,
        }
    }
}

/// One hidden test case: positional arguments and the expected result,
/// as JSON values (the executor's harness compares in Python, whose
/// JSON mapping is exact for the integer/boolean/string/list values
/// these families use).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Case {
    /// Positional arguments for one call.
    pub args: Vec<Value>,
    /// Expected return value.
    pub expected: Value,
}

/// A task family definition: the invariant part of a task.
#[derive(Debug, Clone)]
pub struct Family {
    /// Family name (also the entry-point prefix).
    pub name: &'static str,
    /// Parameter list text for the `def` line.
    pub arguments: &'static str,
    /// Description variants; generation picks one per task.
    pub descriptions: &'static [&'static str],
    /// Reference completion body (indented, leading newline).
    pub body: &'static str,
    /// Hidden cases, identical across splits as in the Python.
    pub cases: Vec<Case>,
}

/// One concrete, frozen task.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CodingTask {
    /// `"{split}-{family}-{index:03}"`, as in the Python.
    pub task_id: String,
    /// Split name this task belongs to.
    pub split: String,
    /// Family name.
    pub family: String,
    /// Generated entry-point (function) name, unseen in other splits.
    pub entry_point: String,
    /// Prompt text ending with the `def` line; a completion continues it.
    pub prompt: String,
    /// Reference completion body.
    pub reference_completion: String,
    /// Hidden cases.
    pub cases: Vec<Case>,
}

impl CodingTask {
    /// The full reference source (prompt + reference completion).
    pub fn reference_source(&self) -> String {
        format!("{}{}", self.prompt, self.reference_completion)
    }
}

/// Family names in definition order.
pub const FAMILY_NAMES: &[&str] = &[
    "increment",
    "double",
    "square",
    "absolute",
    "nonnegative",
    "even",
    "reverse",
    "list_sum",
];

/// Convenience constructor for one hidden case.
fn case(args: Vec<Value>, expected: Value) -> Case {
    Case { args, expected }
}

/// The eight families from `glm53_flash/tasks.py`, in the same order.
///
/// Built per call: case values are `serde_json::Value`s, which have
/// no const construction. The table is tiny and calls are bounded by
/// the split-generation limits, so rebuilding is immaterial.
pub fn families() -> Vec<Family> {
    vec![
        Family {
            name: "increment",
            arguments: "x",
            descriptions: &["Return x plus one.", "Increase x by one."],
            body: "\n    return x + 1\n",
            cases: vec![
                case(vec![json!(-3)], json!(-2)),
                case(vec![json!(0)], json!(1)),
                case(vec![json!(7)], json!(8)),
            ],
        },
        Family {
            name: "double",
            arguments: "x",
            descriptions: &["Return two times x.", "Double the input."],
            body: "\n    return x * 2\n",
            cases: vec![
                case(vec![json!(-4)], json!(-8)),
                case(vec![json!(0)], json!(0)),
                case(vec![json!(6)], json!(12)),
            ],
        },
        Family {
            name: "square",
            arguments: "x",
            descriptions: &["Return x squared.", "Multiply x by itself."],
            body: "\n    return x * x\n",
            cases: vec![
                case(vec![json!(-4)], json!(16)),
                case(vec![json!(0)], json!(0)),
                case(vec![json!(5)], json!(25)),
            ],
        },
        Family {
            name: "absolute",
            arguments: "x",
            descriptions: &[
                "Return the absolute value of x.",
                "Make a negative x positive.",
            ],
            body: "\n    return -x if x < 0 else x\n",
            cases: vec![
                case(vec![json!(-7)], json!(7)),
                case(vec![json!(0)], json!(0)),
                case(vec![json!(9)], json!(9)),
            ],
        },
        Family {
            name: "nonnegative",
            arguments: "x",
            descriptions: &[
                "Clamp x to at least zero.",
                "Return zero when x is negative.",
            ],
            body: "\n    return x if x > 0 else 0\n",
            cases: vec![
                case(vec![json!(-5)], json!(0)),
                case(vec![json!(0)], json!(0)),
                case(vec![json!(8)], json!(8)),
            ],
        },
        Family {
            name: "even",
            arguments: "x",
            descriptions: &["Return whether x is even.", "Check divisibility by two."],
            body: "\n    return x % 2 == 0\n",
            cases: vec![
                case(vec![json!(-3)], json!(false)),
                case(vec![json!(0)], json!(true)),
                case(vec![json!(8)], json!(true)),
            ],
        },
        Family {
            name: "reverse",
            arguments: "text",
            descriptions: &["Return text in reverse order.", "Reverse the string."],
            body: "\n    return text[::-1]\n",
            cases: vec![
                case(vec![json!("")], json!("")),
                case(vec![json!("abc")], json!("cba")),
                case(vec![json!("level")], json!("level")),
            ],
        },
        Family {
            name: "list_sum",
            arguments: "values",
            descriptions: &["Return the sum of values.", "Add every number in the list."],
            body: "\n    return sum(values)\n",
            cases: vec![
                case(vec![json!([])], json!(0)),
                case(vec![json!([1, 2, 3])], json!(6)),
                case(vec![json!([-2, 5])], json!(3)),
            ],
        },
    ]
}

/// All defined family names, in definition order.
pub fn family_names() -> Vec<&'static str> {
    FAMILY_NAMES.to_vec()
}

/// Look up a family by name; unknown names are an error.
pub fn family_by_name(name: &str) -> Result<Family, TrainlabError> {
    families()
        .into_iter()
        .find(|family| family.name == name)
        .ok_or_else(|| TrainlabError::UnknownFamily(name.to_string()))
}

/// Generate the frozen tasks for `split`, `per_family` per family.
///
/// Deterministic: the same `(split, per_family)` always yields the
/// same tasks. `per_family` must be in `1..=PER_FAMILY_MAX`.
pub fn frozen_tasks(split: Split, per_family: usize) -> Result<Vec<CodingTask>, TrainlabError> {
    if per_family == 0 || per_family > PER_FAMILY_MAX {
        return Err(TrainlabError::LimitExceeded(format!(
            "per_family {per_family} outside 1..={PER_FAMILY_MAX}"
        )));
    }
    let all_families = families();
    let total = per_family * all_families.len();
    if total > TASKS_PER_SPLIT_MAX {
        return Err(TrainlabError::LimitExceeded(format!(
            "split would hold {total} tasks, max {TASKS_PER_SPLIT_MAX}"
        )));
    }
    let mut tasks = Vec::with_capacity(total);
    for (family_index, family) in all_families.iter().enumerate() {
        for index in 0..per_family {
            let seed = split.base_seed() + (family_index as u64) * 53;
            tasks.push(make_task(family, split, index, seed));
        }
    }
    Ok(tasks)
}

/// Generate one task, mirroring the Python seed mixing:
/// `(seed + 1) * 1_000_003 + index * 9176 + sum(ord(family name))`.
fn make_task(family: &Family, split: Split, index: usize, seed: u64) -> CodingTask {
    let name_sum: u64 = family.name.bytes().map(u64::from).sum();
    let mixed = (seed + 1) * 1_000_003 + (index as u64) * 9176 + name_sum;
    let mut rng = SplitMix64::new(mixed);
    let entry_point = generate_entry_point(family.name, &mut rng);
    let description =
        family.descriptions[rng.next_below(family.descriptions.len() as u64) as usize];
    let prompt = format!(
        "# Complete this Python function.\n# {description}\ndef {entry_point}({}):",
        family.arguments
    );
    CodingTask {
        task_id: format!("{}-{}-{index:03}", split.name(), family.name),
        split: split.name().to_string(),
        family: family.name.to_string(),
        entry_point,
        prompt,
        reference_completion: family.body.to_string(),
        cases: family.cases.clone(),
    }
}

/// `"{family}_{7 random lowercase letters}"`, as in the Python.
fn generate_entry_point(family_name: &str, rng: &mut SplitMix64) -> String {
    let mut name = String::with_capacity(family_name.len() + 1 + NAME_SUFFIX_LEN);
    name.push_str(family_name);
    name.push('_');
    for _ in 0..NAME_SUFFIX_LEN {
        let letter = u8::try_from(rng.next_below(26)).expect("letter offset fits u8");
        name.push(char::from(b'a' + letter));
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn frozen_tasks_are_deterministic() {
        let a = frozen_tasks(Split::Dev, 4).expect("dev tasks");
        let b = frozen_tasks(Split::Dev, 4).expect("dev tasks");
        assert_eq!(a, b);
        assert_eq!(a.len(), 32);
    }

    #[test]
    fn splits_have_disjoint_entry_points() {
        // Adversarial: confirmation tasks must not reuse names a model
        // could have memorized from dev/rl — the Python scheme's core
        // invariant, checked across every pair of splits.
        let dev: HashSet<String> = frozen_tasks(Split::Dev, 4)
            .expect("dev")
            .iter()
            .map(|task| task.entry_point.clone())
            .collect();
        for split in [Split::Rl, Split::Final, Split::Confirm] {
            for task in frozen_tasks(split, 4).expect("split") {
                assert!(
                    !dev.contains(&task.entry_point),
                    "entry point {} leaks across splits",
                    task.entry_point
                );
            }
        }
    }

    #[test]
    fn task_ids_carry_split_and_index() {
        let tasks = frozen_tasks(Split::Confirm, 2).expect("confirm");
        assert_eq!(tasks[0].task_id, "confirm-increment-000");
        assert_eq!(tasks[0].split, "confirm");
        assert!(tasks[0].prompt.contains(&tasks[0].entry_point));
        assert!(tasks[0].reference_source().ends_with("return x + 1\n"));
    }

    #[test]
    fn per_family_bounds_are_enforced() {
        assert!(frozen_tasks(Split::Dev, 0).is_err());
        assert!(frozen_tasks(Split::Dev, PER_FAMILY_MAX + 1).is_err());
    }

    #[test]
    fn unknown_split_and_family_are_errors() {
        assert!(matches!(
            Split::parse("test"),
            Err(TrainlabError::UnknownSplit(_))
        ));
        assert!(matches!(
            family_by_name("quicksort"),
            Err(TrainlabError::UnknownFamily(_))
        ));
    }
}
