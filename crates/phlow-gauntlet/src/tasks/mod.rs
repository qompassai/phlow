//! The 100 gauntlet tasks. One module per task; each file has a single
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
pub mod task_100;
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
pub mod task_36;
pub mod task_37;
pub mod task_38;
pub mod task_39;
pub mod task_40;
pub mod task_41;
pub mod task_42;
pub mod task_43;
pub mod task_44;
pub mod task_45;
pub mod task_46;
pub mod task_47;
pub mod task_48;
pub mod task_49;
pub mod task_50;
pub mod task_51;
pub mod task_52;
pub mod task_53;
pub mod task_54;
pub mod task_55;
pub mod task_56;
pub mod task_57;
pub mod task_58;
pub mod task_59;
pub mod task_60;
pub mod task_61;
pub mod task_62;
pub mod task_63;
pub mod task_64;
pub mod task_65;
pub mod task_66;
pub mod task_67;
pub mod task_68;
pub mod task_69;
pub mod task_70;
pub mod task_71;
pub mod task_72;
pub mod task_73;
pub mod task_74;
pub mod task_75;
pub mod task_76;
pub mod task_77;
pub mod task_78;
pub mod task_79;
pub mod task_80;
pub mod task_81;
pub mod task_82;
pub mod task_83;
pub mod task_84;
pub mod task_85;
pub mod task_86;
pub mod task_87;
pub mod task_88;
pub mod task_89;
pub mod task_90;
pub mod task_91;
pub mod task_92;
pub mod task_93;
pub mod task_94;
pub mod task_95;
pub mod task_96;
pub mod task_97;
pub mod task_98;
pub mod task_99;

use crate::{Ctx, GauntletError, TaskKind, TaskReport};
use std::time::Instant;

/// Closed list of task ids, in run order. The list is fixed at
/// `TASK_COUNT_MAX`: extending the gauntlet is a design change.
pub const TASK_IDS: [&str; crate::TASK_COUNT_MAX] = [
    "task-01", "task-02", "task-03", "task-04", "task-05", "task-06", "task-07", "task-08",
    "task-09", "task-10", "task-11", "task-12", "task-13", "task-14", "task-15", "task-16",
    "task-17", "task-18", "task-19", "task-20", "task-21", "task-22", "task-23", "task-24",
    "task-25", "task-26", "task-27", "task-28", "task-29", "task-30", "task-31", "task-32",
    "task-33", "task-34", "task-35", "task-36", "task-37", "task-38", "task-39", "task-40",
    "task-41", "task-42", "task-43", "task-44", "task-45", "task-46", "task-47", "task-48",
    "task-49", "task-50", "task-51", "task-52", "task-53", "task-54", "task-55", "task-56",
    "task-57", "task-58", "task-59", "task-60", "task-61", "task-62", "task-63", "task-64",
    "task-65", "task-66", "task-67", "task-68", "task-69", "task-70", "task-71", "task-72",
    "task-73", "task-74", "task-75", "task-76", "task-77", "task-78", "task-79", "task-80",
    "task-81", "task-82", "task-83", "task-84", "task-85", "task-86", "task-87", "task-88",
    "task-89", "task-90", "task-91", "task-92", "task-93", "task-94", "task-95", "task-96",
    "task-97", "task-98", "task-99", "task-100",
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
        task_36::ID => (task_36::ID, task_36::NAME, task_36::KIND, task_36::run(ctx)),
        task_37::ID => (task_37::ID, task_37::NAME, task_37::KIND, task_37::run(ctx)),
        task_38::ID => (task_38::ID, task_38::NAME, task_38::KIND, task_38::run(ctx)),
        task_39::ID => (task_39::ID, task_39::NAME, task_39::KIND, task_39::run(ctx)),
        task_40::ID => (task_40::ID, task_40::NAME, task_40::KIND, task_40::run(ctx)),
        task_41::ID => (task_41::ID, task_41::NAME, task_41::KIND, task_41::run(ctx)),
        task_42::ID => (task_42::ID, task_42::NAME, task_42::KIND, task_42::run(ctx)),
        task_43::ID => (task_43::ID, task_43::NAME, task_43::KIND, task_43::run(ctx)),
        task_44::ID => (task_44::ID, task_44::NAME, task_44::KIND, task_44::run(ctx)),
        task_45::ID => (task_45::ID, task_45::NAME, task_45::KIND, task_45::run(ctx)),
        task_46::ID => (task_46::ID, task_46::NAME, task_46::KIND, task_46::run(ctx)),
        task_47::ID => (task_47::ID, task_47::NAME, task_47::KIND, task_47::run(ctx)),
        task_48::ID => (task_48::ID, task_48::NAME, task_48::KIND, task_48::run(ctx)),
        task_49::ID => (task_49::ID, task_49::NAME, task_49::KIND, task_49::run(ctx)),
        task_50::ID => (task_50::ID, task_50::NAME, task_50::KIND, task_50::run(ctx)),
        task_51::ID => (task_51::ID, task_51::NAME, task_51::KIND, task_51::run(ctx)),
        task_52::ID => (task_52::ID, task_52::NAME, task_52::KIND, task_52::run(ctx)),
        task_53::ID => (task_53::ID, task_53::NAME, task_53::KIND, task_53::run(ctx)),
        task_54::ID => (task_54::ID, task_54::NAME, task_54::KIND, task_54::run(ctx)),
        task_55::ID => (task_55::ID, task_55::NAME, task_55::KIND, task_55::run(ctx)),
        task_56::ID => (task_56::ID, task_56::NAME, task_56::KIND, task_56::run(ctx)),
        task_57::ID => (task_57::ID, task_57::NAME, task_57::KIND, task_57::run(ctx)),
        task_58::ID => (task_58::ID, task_58::NAME, task_58::KIND, task_58::run(ctx)),
        task_59::ID => (task_59::ID, task_59::NAME, task_59::KIND, task_59::run(ctx)),
        task_60::ID => (task_60::ID, task_60::NAME, task_60::KIND, task_60::run(ctx)),
        task_61::ID => (task_61::ID, task_61::NAME, task_61::KIND, task_61::run(ctx)),
        task_62::ID => (task_62::ID, task_62::NAME, task_62::KIND, task_62::run(ctx)),
        task_63::ID => (task_63::ID, task_63::NAME, task_63::KIND, task_63::run(ctx)),
        task_64::ID => (task_64::ID, task_64::NAME, task_64::KIND, task_64::run(ctx)),
        task_65::ID => (task_65::ID, task_65::NAME, task_65::KIND, task_65::run(ctx)),
        task_66::ID => (task_66::ID, task_66::NAME, task_66::KIND, task_66::run(ctx)),
        task_67::ID => (task_67::ID, task_67::NAME, task_67::KIND, task_67::run(ctx)),
        task_68::ID => (task_68::ID, task_68::NAME, task_68::KIND, task_68::run(ctx)),
        task_69::ID => (task_69::ID, task_69::NAME, task_69::KIND, task_69::run(ctx)),
        task_70::ID => (task_70::ID, task_70::NAME, task_70::KIND, task_70::run(ctx)),
        task_71::ID => (task_71::ID, task_71::NAME, task_71::KIND, task_71::run(ctx)),
        task_72::ID => (task_72::ID, task_72::NAME, task_72::KIND, task_72::run(ctx)),
        task_73::ID => (task_73::ID, task_73::NAME, task_73::KIND, task_73::run(ctx)),
        task_74::ID => (task_74::ID, task_74::NAME, task_74::KIND, task_74::run(ctx)),
        task_75::ID => (task_75::ID, task_75::NAME, task_75::KIND, task_75::run(ctx)),
        task_76::ID => (task_76::ID, task_76::NAME, task_76::KIND, task_76::run(ctx)),
        task_77::ID => (task_77::ID, task_77::NAME, task_77::KIND, task_77::run(ctx)),
        task_78::ID => (task_78::ID, task_78::NAME, task_78::KIND, task_78::run(ctx)),
        task_79::ID => (task_79::ID, task_79::NAME, task_79::KIND, task_79::run(ctx)),
        task_80::ID => (task_80::ID, task_80::NAME, task_80::KIND, task_80::run(ctx)),
        task_81::ID => (task_81::ID, task_81::NAME, task_81::KIND, task_81::run(ctx)),
        task_82::ID => (task_82::ID, task_82::NAME, task_82::KIND, task_82::run(ctx)),
        task_83::ID => (task_83::ID, task_83::NAME, task_83::KIND, task_83::run(ctx)),
        task_84::ID => (task_84::ID, task_84::NAME, task_84::KIND, task_84::run(ctx)),
        task_85::ID => (task_85::ID, task_85::NAME, task_85::KIND, task_85::run(ctx)),
        task_86::ID => (task_86::ID, task_86::NAME, task_86::KIND, task_86::run(ctx)),
        task_87::ID => (task_87::ID, task_87::NAME, task_87::KIND, task_87::run(ctx)),
        task_88::ID => (task_88::ID, task_88::NAME, task_88::KIND, task_88::run(ctx)),
        task_89::ID => (task_89::ID, task_89::NAME, task_89::KIND, task_89::run(ctx)),
        task_90::ID => (task_90::ID, task_90::NAME, task_90::KIND, task_90::run(ctx)),
        task_91::ID => (task_91::ID, task_91::NAME, task_91::KIND, task_91::run(ctx)),
        task_92::ID => (task_92::ID, task_92::NAME, task_92::KIND, task_92::run(ctx)),
        task_93::ID => (task_93::ID, task_93::NAME, task_93::KIND, task_93::run(ctx)),
        task_94::ID => (task_94::ID, task_94::NAME, task_94::KIND, task_94::run(ctx)),
        task_95::ID => (task_95::ID, task_95::NAME, task_95::KIND, task_95::run(ctx)),
        task_96::ID => (task_96::ID, task_96::NAME, task_96::KIND, task_96::run(ctx)),
        task_97::ID => (task_97::ID, task_97::NAME, task_97::KIND, task_97::run(ctx)),
        task_98::ID => (task_98::ID, task_98::NAME, task_98::KIND, task_98::run(ctx)),
        task_99::ID => (task_99::ID, task_99::NAME, task_99::KIND, task_99::run(ctx)),
        task_100::ID => (
            task_100::ID,
            task_100::NAME,
            task_100::KIND,
            task_100::run(ctx),
        ),
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
        task_36::ID => Some((task_36::NAME, task_36::KIND)),
        task_37::ID => Some((task_37::NAME, task_37::KIND)),
        task_38::ID => Some((task_38::NAME, task_38::KIND)),
        task_39::ID => Some((task_39::NAME, task_39::KIND)),
        task_40::ID => Some((task_40::NAME, task_40::KIND)),
        task_41::ID => Some((task_41::NAME, task_41::KIND)),
        task_42::ID => Some((task_42::NAME, task_42::KIND)),
        task_43::ID => Some((task_43::NAME, task_43::KIND)),
        task_44::ID => Some((task_44::NAME, task_44::KIND)),
        task_45::ID => Some((task_45::NAME, task_45::KIND)),
        task_46::ID => Some((task_46::NAME, task_46::KIND)),
        task_47::ID => Some((task_47::NAME, task_47::KIND)),
        task_48::ID => Some((task_48::NAME, task_48::KIND)),
        task_49::ID => Some((task_49::NAME, task_49::KIND)),
        task_50::ID => Some((task_50::NAME, task_50::KIND)),
        task_51::ID => Some((task_51::NAME, task_51::KIND)),
        task_52::ID => Some((task_52::NAME, task_52::KIND)),
        task_53::ID => Some((task_53::NAME, task_53::KIND)),
        task_54::ID => Some((task_54::NAME, task_54::KIND)),
        task_55::ID => Some((task_55::NAME, task_55::KIND)),
        task_56::ID => Some((task_56::NAME, task_56::KIND)),
        task_57::ID => Some((task_57::NAME, task_57::KIND)),
        task_58::ID => Some((task_58::NAME, task_58::KIND)),
        task_59::ID => Some((task_59::NAME, task_59::KIND)),
        task_60::ID => Some((task_60::NAME, task_60::KIND)),
        task_61::ID => Some((task_61::NAME, task_61::KIND)),
        task_62::ID => Some((task_62::NAME, task_62::KIND)),
        task_63::ID => Some((task_63::NAME, task_63::KIND)),
        task_64::ID => Some((task_64::NAME, task_64::KIND)),
        task_65::ID => Some((task_65::NAME, task_65::KIND)),
        task_66::ID => Some((task_66::NAME, task_66::KIND)),
        task_67::ID => Some((task_67::NAME, task_67::KIND)),
        task_68::ID => Some((task_68::NAME, task_68::KIND)),
        task_69::ID => Some((task_69::NAME, task_69::KIND)),
        task_70::ID => Some((task_70::NAME, task_70::KIND)),
        task_71::ID => Some((task_71::NAME, task_71::KIND)),
        task_72::ID => Some((task_72::NAME, task_72::KIND)),
        task_73::ID => Some((task_73::NAME, task_73::KIND)),
        task_74::ID => Some((task_74::NAME, task_74::KIND)),
        task_75::ID => Some((task_75::NAME, task_75::KIND)),
        task_76::ID => Some((task_76::NAME, task_76::KIND)),
        task_77::ID => Some((task_77::NAME, task_77::KIND)),
        task_78::ID => Some((task_78::NAME, task_78::KIND)),
        task_79::ID => Some((task_79::NAME, task_79::KIND)),
        task_80::ID => Some((task_80::NAME, task_80::KIND)),
        task_81::ID => Some((task_81::NAME, task_81::KIND)),
        task_82::ID => Some((task_82::NAME, task_82::KIND)),
        task_83::ID => Some((task_83::NAME, task_83::KIND)),
        task_84::ID => Some((task_84::NAME, task_84::KIND)),
        task_85::ID => Some((task_85::NAME, task_85::KIND)),
        task_86::ID => Some((task_86::NAME, task_86::KIND)),
        task_87::ID => Some((task_87::NAME, task_87::KIND)),
        task_88::ID => Some((task_88::NAME, task_88::KIND)),
        task_89::ID => Some((task_89::NAME, task_89::KIND)),
        task_90::ID => Some((task_90::NAME, task_90::KIND)),
        task_91::ID => Some((task_91::NAME, task_91::KIND)),
        task_92::ID => Some((task_92::NAME, task_92::KIND)),
        task_93::ID => Some((task_93::NAME, task_93::KIND)),
        task_94::ID => Some((task_94::NAME, task_94::KIND)),
        task_95::ID => Some((task_95::NAME, task_95::KIND)),
        task_96::ID => Some((task_96::NAME, task_96::KIND)),
        task_97::ID => Some((task_97::NAME, task_97::KIND)),
        task_98::ID => Some((task_98::NAME, task_98::KIND)),
        task_99::ID => Some((task_99::NAME, task_99::KIND)),
        task_100::ID => Some((task_100::NAME, task_100::KIND)),
        _ => None,
    }
}
