//! task-52: straggler mitigation (nvimlua).
//!
//! Behavioral probe: the design asks for straggler mitigation on the
//! parallel-run supervisor (task-01's fan-out machinery) — a slow-but-
//! alive worker should trigger a speculative duplicate after a timeout,
//! whichever finishes first wins, and exactly one result commits. The
//! driver (`lua/gauntlet/task_52.lua`) exercises the REAL
//! `ai.harness.supervisor` with a mock adapter whose workers have
//! driver-controlled completion, stepping the simulated clock explicitly
//! via `supervisor.tick`.
//!
//! Honest result: the seam is ABSENT. The supervisor carries no
//! speculate/duplicate/backup-request API (the probe scans the real
//! export table: zero hits), no timeout launches a second attempt for a
//! slow run, and the straggler scenario shows the logical run completing
//! at the straggler's pace — after 10x the fast path the slow run still
//! shows exactly 1 `run.started`, no speculative duplicate. The design's
//! p99 bound (speculation timeout + fast-path time) has no seam. The
//! one guarantee that holds is single-commit, and it holds vacuously:
//! with only one attempt ever launched, at most one result commits (the
//! probe asserts exactly 1 `run.finished` and that a second finish is
//! refused as an invalid transition).
//!
//! Distinct from task-18: a *dead* worker is detected and its work
//! *reassigned* (retry with bounded backoff); a straggler is alive, and
//! killing/replacing it would be a latency optimization, not recovery.
//! The supervisor implements the former (diver-owned, flagged in wave
//! 41-45); the latter does not exist.
//!
//! Fail-closed: if speculation APIs ever appear, the
//! `no-speculation-api` scenario reports `where = "recon"` (premise
//! changed). Diver-owned finding: flagged, never fixed on gauntlet
//! authority.

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};

/// Task id.
pub const ID: &str = "task-52";
/// Human-readable name.
pub const NAME: &str = "straggler mitigation";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// The probe scenarios the Lua driver runs, in order:
/// two validation, two adversarial.
pub const SCENARIOS: [&str; 4] = [
    "all-fast",
    "straggler",
    "single-commit",
    "no-speculation-api",
];

/// Attempt the task: run every probe scenario against the real
/// supervisor, then report the honest task-level verdict.
///
/// Each scenario's probe assertions are expected to pass — they
/// characterize the real no-speculation behavior. The task-level
/// verdict is `fail` at `"seam"` regardless: the design's pass criteria
/// (speculative duplicate after a timeout; p99 bounded by timeout +
/// fast-path) describe machinery that does not exist.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    let mut evidence = vec![
        "recon: ai.harness.supervisor manages run lifecycles (create/start/finish/retry_tick) with bounded retry backoff — it has no speculative execution: no second attempt is ever launched for a slow-but-alive run"
            .to_string(),
    ];
    for scenario in SCENARIOS {
        match run_scenario(ctx, scenario) {
            TaskOutcome::Pass { evidence: probe } => {
                evidence.push(format!("scenario {scenario}: probe passed"));
                for line in &probe {
                    evidence.push(format!("scenario {scenario}: {line}"));
                }
            }
            TaskOutcome::Fail {
                where_,
                how,
                evidence: probe,
            } => {
                for line in &probe {
                    evidence.push(format!("scenario {scenario}: {line}"));
                }
                return TaskOutcome::Fail {
                    where_: format!("probe-{scenario}-{where_}"),
                    how,
                    evidence: bound_evidence(evidence),
                };
            }
        }
    }
    evidence.push(
        "finding: the straggler completes at its own pace — after 10x the fast path the slow run still shows exactly 1 run.started, no speculative duplicate; p99 is bounded by the slow worker, not by speculation-timeout + fast-path"
            .to_string(),
    );
    TaskOutcome::Fail {
        where_: "seam".to_string(),
        how: "seam absent: ai.harness.supervisor has no speculative execution — no speculate/duplicate/backup-request API, no timeout launches a second attempt for a slow-but-alive worker. The straggler probe (real supervisor, driver-controlled worker latency, simulated clock stepped 10x past the fast path) shows the slow run keeps exactly 1 run.started and the logical run completes at the straggler's pace. The design's pass criteria (speculative duplicate after a timeout; p99 bounded by speculation timeout + fast-path time, measured; loser cancelled with no double side effects) describe machinery that does not exist; single-commit holds only vacuously (one attempt ever launched, finish idempotent). Open design gap (diver-owned).".to_string(),
        evidence: bound_evidence(evidence),
    }
}

/// Attempt one named driver scenario via `GAUNTLET_SCENARIO`.
///
/// Known scenarios: `"all-fast"`, `"straggler"`, `"single-commit"`,
/// `"no-speculation-api"`. Unknown names make the driver report failure.
pub fn run_scenario(ctx: &Ctx, scenario: &str) -> TaskOutcome {
    crate::run_nvim_lua_driver_with_env(
        ctx,
        "task_52.lua",
        "task-52",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
