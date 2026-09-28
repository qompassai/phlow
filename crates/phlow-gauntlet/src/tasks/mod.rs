//! The 35 gauntlet tasks. One module per task; each file has a single
//! owning worker during implementation (disjoint ownership).

pub mod task_01;
pub mod task_02;
pub mod task_03;
pub mod task_04;
pub mod task_05;
pub mod task_06;
pub mod task_07;
pub mod task_08;
pub mod task_09;
pub mod task_10;
pub mod task_11;
pub mod task_12;
pub mod task_13;
pub mod task_14;
pub mod task_15;
pub mod task_16;
pub mod task_17;
pub mod task_18;
pub mod task_19;
pub mod task_20;
pub mod task_21;
pub mod task_22;
pub mod task_23;
pub mod task_24;
pub mod task_25;
pub mod task_26;
pub mod task_27;
pub mod task_28;
pub mod task_29;
pub mod task_30;
pub mod task_31;
pub mod task_32;
pub mod task_33;
pub mod task_34;
pub mod task_35;

use crate::{Ctx, GauntletError, TaskKind, TaskReport};
use std::time::Instant;

/// Closed list of task ids, in run order. The list is fixed at
/// `TASK_COUNT_MAX`: extending the gauntlet is a design change.
pub const TASK_IDS: [&str; crate::TASK_COUNT_MAX] = [
    "task-01", "task-02", "task-03", "task-04", "task-05", "task-06", "task-07", "task-08",
    "task-09", "task-10", "task-11", "task-12", "task-13", "task-14", "task-15", "task-16",
    "task-17", "task-18", "task-19", "task-20", "task-21", "task-22", "task-23", "task-24",
    "task-25", "task-26", "task-27", "task-28", "task-29", "task-30", "task-31", "task-32",
    "task-33", "task-34", "task-35",
];

/// Run one task by id and record a `TaskReport`.
pub fn run_task(id: &str, ctx: &Ctx) -> Result<TaskReport, GauntletError> {
    let started = Instant::now();
    let (task_id, name, kind, outcome) = match id {
        task_01::ID => (task_01::ID, task_01::NAME, task_01::KIND, task_01::run(ctx)),
        task_02::ID => (task_02::ID, task_02::NAME, task_02::KIND, task_02::run(ctx)),
        task_03::ID => (task_03::ID, task_03::NAME, task_03::KIND, task_03::run(ctx)),
        task_04::ID => (task_04::ID, task_04::NAME, task_04::KIND, task_04::run(ctx)),
        task_05::ID => (task_05::ID, task_05::NAME, task_05::KIND, task_05::run(ctx)),
        task_06::ID => (task_06::ID, task_06::NAME, task_06::KIND, task_06::run(ctx)),
        task_07::ID => (task_07::ID, task_07::NAME, task_07::KIND, task_07::run(ctx)),
        task_08::ID => (task_08::ID, task_08::NAME, task_08::KIND, task_08::run(ctx)),
        task_09::ID => (task_09::ID, task_09::NAME, task_09::KIND, task_09::run(ctx)),
        task_10::ID => (task_10::ID, task_10::NAME, task_10::KIND, task_10::run(ctx)),
        task_11::ID => (task_11::ID, task_11::NAME, task_11::KIND, task_11::run(ctx)),
        task_12::ID => (task_12::ID, task_12::NAME, task_12::KIND, task_12::run(ctx)),
        task_13::ID => (task_13::ID, task_13::NAME, task_13::KIND, task_13::run(ctx)),
        task_14::ID => (task_14::ID, task_14::NAME, task_14::KIND, task_14::run(ctx)),
        task_15::ID => (task_15::ID, task_15::NAME, task_15::KIND, task_15::run(ctx)),
        task_16::ID => (task_16::ID, task_16::NAME, task_16::KIND, task_16::run(ctx)),
        task_17::ID => (task_17::ID, task_17::NAME, task_17::KIND, task_17::run(ctx)),
        task_18::ID => (task_18::ID, task_18::NAME, task_18::KIND, task_18::run(ctx)),
        task_19::ID => (task_19::ID, task_19::NAME, task_19::KIND, task_19::run(ctx)),
        task_20::ID => (task_20::ID, task_20::NAME, task_20::KIND, task_20::run(ctx)),
        task_21::ID => (task_21::ID, task_21::NAME, task_21::KIND, task_21::run(ctx)),
        task_22::ID => (task_22::ID, task_22::NAME, task_22::KIND, task_22::run(ctx)),
        task_23::ID => (task_23::ID, task_23::NAME, task_23::KIND, task_23::run(ctx)),
        task_24::ID => (task_24::ID, task_24::NAME, task_24::KIND, task_24::run(ctx)),
        task_25::ID => (task_25::ID, task_25::NAME, task_25::KIND, task_25::run(ctx)),
        task_26::ID => (task_26::ID, task_26::NAME, task_26::KIND, task_26::run(ctx)),
        task_27::ID => (task_27::ID, task_27::NAME, task_27::KIND, task_27::run(ctx)),
        task_28::ID => (task_28::ID, task_28::NAME, task_28::KIND, task_28::run(ctx)),
        task_29::ID => (task_29::ID, task_29::NAME, task_29::KIND, task_29::run(ctx)),
        task_30::ID => (task_30::ID, task_30::NAME, task_30::KIND, task_30::run(ctx)),
        task_31::ID => (task_31::ID, task_31::NAME, task_31::KIND, task_31::run(ctx)),
        task_32::ID => (task_32::ID, task_32::NAME, task_32::KIND, task_32::run(ctx)),
        task_33::ID => (task_33::ID, task_33::NAME, task_33::KIND, task_33::run(ctx)),
        task_34::ID => (task_34::ID, task_34::NAME, task_34::KIND, task_34::run(ctx)),
        task_35::ID => (task_35::ID, task_35::NAME, task_35::KIND, task_35::run(ctx)),
        _ => return Err(GauntletError::UnknownTask { id: id.to_string() }),
    };
    let duration_ms = started.elapsed().as_millis().min(u64::MAX as u128) as u64;
    Ok(TaskReport {
        id: task_id,
        name,
        kind,
        outcome,
        duration_ms,
    })
}

/// Static metadata for one task id.
pub fn task_meta(id: &str) -> Option<(&'static str, TaskKind)> {
    match id {
        task_01::ID => Some((task_01::NAME, task_01::KIND)),
        task_02::ID => Some((task_02::NAME, task_02::KIND)),
        task_03::ID => Some((task_03::NAME, task_03::KIND)),
        task_04::ID => Some((task_04::NAME, task_04::KIND)),
        task_05::ID => Some((task_05::NAME, task_05::KIND)),
        task_06::ID => Some((task_06::NAME, task_06::KIND)),
        task_07::ID => Some((task_07::NAME, task_07::KIND)),
        task_08::ID => Some((task_08::NAME, task_08::KIND)),
        task_09::ID => Some((task_09::NAME, task_09::KIND)),
        task_10::ID => Some((task_10::NAME, task_10::KIND)),
        task_11::ID => Some((task_11::NAME, task_11::KIND)),
        task_12::ID => Some((task_12::NAME, task_12::KIND)),
        task_13::ID => Some((task_13::NAME, task_13::KIND)),
        task_14::ID => Some((task_14::NAME, task_14::KIND)),
        task_15::ID => Some((task_15::NAME, task_15::KIND)),
        task_16::ID => Some((task_16::NAME, task_16::KIND)),
        task_17::ID => Some((task_17::NAME, task_17::KIND)),
        task_18::ID => Some((task_18::NAME, task_18::KIND)),
        task_19::ID => Some((task_19::NAME, task_19::KIND)),
        task_20::ID => Some((task_20::NAME, task_20::KIND)),
        task_21::ID => Some((task_21::NAME, task_21::KIND)),
        task_22::ID => Some((task_22::NAME, task_22::KIND)),
        task_23::ID => Some((task_23::NAME, task_23::KIND)),
        task_24::ID => Some((task_24::NAME, task_24::KIND)),
        task_25::ID => Some((task_25::NAME, task_25::KIND)),
        task_26::ID => Some((task_26::NAME, task_26::KIND)),
        task_27::ID => Some((task_27::NAME, task_27::KIND)),
        task_28::ID => Some((task_28::NAME, task_28::KIND)),
        task_29::ID => Some((task_29::NAME, task_29::KIND)),
        task_30::ID => Some((task_30::NAME, task_30::KIND)),
        task_31::ID => Some((task_31::NAME, task_31::KIND)),
        task_32::ID => Some((task_32::NAME, task_32::KIND)),
        task_33::ID => Some((task_33::NAME, task_33::KIND)),
        task_34::ID => Some((task_34::NAME, task_34::KIND)),
        task_35::ID => Some((task_35::NAME, task_35::KIND)),
        _ => None,
    }
}
