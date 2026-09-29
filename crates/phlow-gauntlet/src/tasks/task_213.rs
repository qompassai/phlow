//! task-213: approval scope immutability.
//!
//! Honest scope: Rescoped from Diver Lua to phlow-approval. The queue owns the admitted scope;
//! editing the caller's input document afterwards must not broaden a pending or approved record,
//! and a wider scope needs a new request and a new human decision. No executor is involved.

use phlow_approval::{DEFAULT_TTL, HumanVerdict, Request, State};
use serde_json::json;

use super::task_209::{HUMAN, RUN, pending, pending_from, record, req_json, run_cases};
use super::task_233::err;
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-213";
/// Desired permission invariant.
pub const NAME: &str = "approval scope immutability";
/// Drives the phlow-approval queue directly.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation cases followed by two adversarial cases.
pub const CASES: [&str; 4] = [
    "approved_path_preserved",
    "approved_tool_preserved",
    "pending_scope_cannot_expand",
    "approved_scope_cannot_expand",
];

/// Run the four cases; retain all case outcomes, including failures.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    run_cases(ID, &CASES, case)
}

fn case(index: usize) -> Result<bool, String> {
    match index {
        0 | 1 => {
            let (mut queue, id) = pending()?;
            queue
                .decide(&id, HumanVerdict::Approve, Some(HUMAN))
                .map_err(err)?;
            let stored = record(&queue, &id)?;
            Ok(match index {
                0 => stored.scope.paths() == ["/work/a"],
                _ => stored.scope.tool() == "fs.write",
            })
        }
        2 => {
            let mut input = req_json();
            let (queue, id) = pending_from(&input)?;
            let paths = input["paths"]
                .as_array_mut()
                .ok_or("fixture paths not an array")?;
            paths.push(json!("/work/b"));
            Ok(record(&queue, &id)?.scope.paths() == ["/work/a"])
        }
        3 => approved_scope_cannot_expand(),
        _ => Err("case index outside fixed array".to_owned()),
    }
}

fn approved_scope_cannot_expand() -> Result<bool, String> {
    let mut input = req_json();
    let (mut queue, id) = pending_from(&input)?;
    queue
        .decide(&id, HumanVerdict::Approve, Some(HUMAN))
        .map_err(err)?;
    input["paths"][0] = json!("/work");
    let wider = Request::from_json(&input).map_err(err)?;
    let wider_id = queue.request(RUN, wider, DEFAULT_TTL).map_err(err)?;
    let original = record(&queue, &id)?;
    Ok(original.scope.paths() == ["/work/a"]
        && original.state == State::Approved
        && wider_id != id
        && record(&queue, &wider_id)?.state == State::Pending)
}

#[cfg(test)]
mod tests {
    #[test]
    fn approved_path_preserved() {
        let result = super::case(0);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn approved_tool_preserved() {
        let result = super::case(1);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn pending_scope_cannot_expand() {
        let result = super::case(2);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn approved_scope_cannot_expand() {
        let result = super::case(3);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
