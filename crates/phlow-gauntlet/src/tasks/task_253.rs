//! task-253: a confidently wrong System 1 never auto-approves, and its errors surface.
//!
//! Honest scope: drives phlow-system1's RiskScorer over real phlow-approval decisions and the
//! real ApprovalQueue with scripted answers at 0.99 confidence; no model runs. Escalation is
//! checked to leave the existing human queue path working, and the queue itself refuses System 1
//! as an approver. Out-of-contract answers and backend failures must come back as typed, displayed
//! escalation reasons, not a silent fallback. Confidence thresholds are not validated here.

use phlow_approval::{DEFAULT_TTL, HumanVerdict, Risk, State};
use phlow_system1::{
    Escalation, IRREVERSIBLE_ID, MockBackend, REVERSIBLE_ID, RISK_ID, Route, System1Error,
};

use super::task_209::{HUMAN, RUN, queue, record, run_cases};
use super::task_233::err;
use super::task_251::{approval_decision, honest, noul, routed, score};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-253";
/// Desired invariant.
pub const NAME: &str = "confidently wrong system1 never auto-approves";
/// Drives the phlow-system1 fast path and phlow-approval queue directly.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation cases, then two adversarial cases.
pub const CASES: [&str; 4] = [
    "calibrated_answer_fast_path",
    "escalation_keeps_human_queue_path",
    "confident_wrong_irreversible_never_approves",
    "confident_wrong_errors_surface",
];

/// Run the four cases; retain all case outcomes, including failures.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    run_cases(ID, &CASES, case)
}

fn case(index: usize) -> Result<bool, String> {
    match index {
        0 => {
            let d = approval_decision("fs.write", "local_reversible", &["/work/a"], &[])?;
            let (route, calls) = routed(honest(), &d, "")?;
            Ok(matches!(route, Route::AutoApprove(_)) && calls == 1)
        }
        1 => {
            let d = approval_decision("fs.write", "local_reversible", &["/work/a"], &[])?;
            let risky = honest().with_answer(RISK_ID, score(0.9, 0.99));
            let (route, _) = routed(risky, &d, "")?;
            let escalated = matches!(route, Route::Escalate(Escalation::RiskTooHigh { .. }));
            let mut queue = queue()?;
            let id = queue.request(RUN, d.proposal(), DEFAULT_TTL).map_err(err)?;
            let system1_refused = queue
                .decide(&id, HumanVerdict::Approve, Some("system"))
                .is_err();
            queue
                .decide(&id, HumanVerdict::Approve, Some(HUMAN))
                .map_err(err)?;
            let approved = record(&queue, &id)?;
            Ok(escalated
                && system1_refused
                && approved.state == State::Approved
                && approved.decided_by.as_deref() == Some(HUMAN))
        }
        2 => {
            let d = approval_decision("fs.delete", "irreversible", &["/work/a"], &[])?;
            let wrong = honest()
                .with_answer(RISK_ID, score(0.01, 0.99))
                .with_answer(REVERSIBLE_ID, noul(true, 0.99))
                .with_answer(IRREVERSIBLE_ID, noul(false, 0.99));
            let (route, calls) = routed(wrong, &d, "")?;
            let blocked = Escalation::ClassNotEligible(Risk::Irreversible);
            Ok(route == Route::Escalate(blocked) && calls == 0)
        }
        3 => {
            let d = approval_decision("fs.write", "local_reversible", &["/work/a"], &[])?;
            let out_of_contract = honest().with_answer(RISK_ID, score(1.5, 0.99));
            let (route, _) = routed(out_of_contract, &d, "")?;
            let protocol_surfaced = matches!(
                &route,
                Route::Escalate(e @ Escalation::System1(System1Error::Protocol { .. }))
                    if e.to_string().contains(RISK_ID)
            );
            let down = System1Error::Transport {
                detail: "connection refused".to_owned(),
            };
            let (route, _) = routed(MockBackend::failing(down), &d, "")?;
            let transport_surfaced = matches!(
                &route,
                Route::Escalate(e @ Escalation::System1(System1Error::Transport { .. }))
                    if e.to_string().contains("connection refused")
            );
            Ok(protocol_surfaced && transport_surfaced)
        }
        _ => Err("case index outside fixed array".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn calibrated_answer_fast_path() {
        let result = super::case(0);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn escalation_keeps_human_queue_path() {
        let result = super::case(1);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn confident_wrong_irreversible_never_approves() {
        let result = super::case(2);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn confident_wrong_errors_surface() {
        let result = super::case(3);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
