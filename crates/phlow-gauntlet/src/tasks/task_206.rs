//! task-206: credential leakage through prompt attachments.
//!
//! Drives the real code-generation prompt builder with repository file content.
//!
//! Honest scope: no canary registration/scanner exists. The desired scanner
//! must prevent an API-key canary in attached .env or source text from reaching
//! the prompt, while retaining harmless source. This probes model-visible
//! prompt text, not an invented scanner and not the repository reader itself.
//! Only synthetic in-memory file content is supplied; no local secrets are read.

use super::task_201::{CANARY, case_index, report, run_cases};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{Ctx, TaskKind, TaskOutcome};
use phlow_llm::build_codegen_prompt;

/// Stable task identifier.
pub const ID: &str = "task-206";
/// Human-readable invariant.
pub const NAME: &str = "credential leakage through prompt attachments";
/// Offline Rust seams; no network, subprocess, or Neovim required.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation controls, then two adversarial credential probes.
pub const CASES: [&str; 4] = [
    "ordinary_source_attachment_preserved",
    "no_attachments_request_preserved",
    "dotenv_credential_blocked",
    "credential_in_source_comment_blocked",
];

/// Run one bounded in-memory probe; unknown names return a driver error.
/// Fixtures contain at most two calls/messages and less than 1 KiB of input.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    let index = case_index(case, &CASES)?;
    let content = match index {
        0 => "fn main() {}".to_owned(),
        1 => String::new(),
        2 => format!("OPENAI_API_KEY={CANARY}"),
        _ => format!("// recovered Authorization: Bearer {CANARY}\nfn main() {{}}"),
    };
    let path = if index == 2 { ".env" } else { "main.rs" };
    let files = [(path, content.as_str())];
    let attachments = if index == 1 { &files[..0] } else { &files[..] };
    let output = build_codegen_prompt("inspect code", "Rust", "none", "fixture", attachments);
    let preserved = output.contains("inspect code") && (index != 0 || output.contains(&content));
    Ok(report(
        CASES[index],
        !output.contains(CANARY) && preserved,
        output.len(),
        "build_codegen_prompt attachments",
    ))
}

/// Run all four cases, retaining only boolean/count evidence, never payloads.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    run_cases(ID, &CASES, run_case)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(index: usize) {
        let result = run_case(CASES[index]).expect("bounded fixture must execute");
        assert!(result.passed, "{}", result.failures.join("; "));
    }

    #[test]
    fn ordinary_source_attachment_preserved() {
        check(0);
    }

    #[test]
    fn no_attachments_request_preserved() {
        check(1);
    }

    #[test]
    fn dotenv_credential_blocked() {
        check(2);
    }

    #[test]
    fn credential_in_source_comment_blocked() {
        check(3);
    }
}
