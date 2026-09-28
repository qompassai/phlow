//! task-56: approval timeout defaults deny (nvimlua).
//!
//! Drives `lua/gauntlet/task_56.lua` in headless Neovim through the real
//! harness approval-expiry seam: `ai.harness.approval` (request / decide /
//! get / sweep_expired) plus `supervisor.tick` driving the expiry — the
//! task-03 pattern, but aimed at the *timeout* path (approver assigned,
//! never responds) rather than the no-approver path.
//!
//! Four scenarios via `GAUNTLET_SCENARIO` (2 validation, 2 adversarial):
//! - `responds-in-time` (V): approver approves before the deadline →
//!   approved, the gated tool proceeds.
//! - `terminal-within-deadline` (V): nobody responds; the request is
//!   terminal by deadline + epsilon, pinned at the boundary (pending at
//!   deadline − 1ns, terminal at deadline).
//! - `timeout-never-grants` (A): deadline passes with no response → not
//!   granted, not left pending; the gate blocks the tool. Hunts the
//!   classic default-allow-on-timeout bug.
//! - `late-approval-cannot-resurrect` (A): decide('approved') after expiry
//!   is rejected with an explicit already-decided error; the record
//!   stays expired.
//!
//! Honest naming note: the design says "denied" and the module header
//! says "Requests expire to denied", but the code's terminal state is
//! named `expired` — the deny-equivalent (terminal, never approved,
//! decide() rejects it). The driver asserts the security property, not
//! the label.

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-56";
/// Human-readable name.
pub const NAME: &str = "approval timeout defaults deny";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// Driver scenarios, in run order: two validation, two adversarial.
pub const SCENARIOS: [&str; 4] = [
    "responds-in-time",
    "terminal-within-deadline",
    "timeout-never-grants",
    "late-approval-cannot-resurrect",
];

/// Attempt the task (the `responds-in-time` control scenario).
pub fn run(ctx: &Ctx) -> TaskOutcome {
    run_scenario(ctx, "responds-in-time")
}

/// Attempt one named driver scenario via `GAUNTLET_SCENARIO`.
///
/// Known scenarios: `"responds-in-time"`, `"terminal-within-deadline"`,
/// `"timeout-never-grants"`, `"late-approval-cannot-resurrect"`.
/// Unknown names make the driver report failure.
pub fn run_scenario(ctx: &Ctx, scenario: &str) -> TaskOutcome {
    crate::run_nvim_lua_driver_with_env(
        ctx,
        "task_56.lua",
        "task-56",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
