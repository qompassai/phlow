//! The 130 gauntlet tasks. One module per task; each file has a single
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
pub mod task_101;
pub mod task_102;
pub mod task_103;
pub mod task_104;
pub mod task_105;
pub mod task_106;
pub mod task_107;
pub mod task_108;
pub mod task_109;
pub mod task_11;
pub mod task_110;
pub mod task_111;
pub mod task_112;
pub mod task_113;
pub mod task_114;
pub mod task_115;
pub mod task_116;
pub mod task_117;
pub mod task_118;
pub mod task_119;
pub mod task_12;
pub mod task_120;
pub mod task_121;
pub mod task_122;
pub mod task_123;
pub mod task_124;
pub mod task_125;
pub mod task_126;
pub mod task_127;
pub mod task_128;
pub mod task_129;
pub mod task_13;
pub mod task_130;
pub mod task_131;
pub mod task_132;
pub mod task_133;
pub mod task_134;
pub mod task_135;
pub mod task_136;
pub mod task_137;
pub mod task_138;
pub mod task_139;
pub mod task_14;
pub mod task_140;
pub mod task_141;
pub mod task_142;
pub mod task_143;
pub mod task_144;
pub mod task_145;
pub mod task_146;
pub mod task_147;
pub mod task_148;
pub mod task_149;
pub mod task_15;
pub mod task_150;
pub mod task_151;
pub mod task_152;
pub mod task_153;
pub mod task_154;
pub mod task_155;
pub mod task_156;
pub mod task_157;
pub mod task_158;
pub mod task_159;
pub mod task_16;
pub mod task_160;
pub mod task_161;
pub mod task_162;
pub mod task_163;
pub mod task_164;
pub mod task_165;
pub mod task_166;
pub mod task_167;
pub mod task_168;
pub mod task_169;
pub mod task_17;
pub mod task_170;
pub mod task_171;
pub mod task_172;
pub mod task_173;
pub mod task_174;
pub mod task_175;
pub mod task_176;
pub mod task_177;
pub mod task_178;
pub mod task_179;
pub mod task_18;
pub mod task_180;
pub mod task_181;
pub mod task_182;
pub mod task_183;
pub mod task_184;
pub mod task_185;
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
    "task-97", "task-98", "task-99", "task-100", "task-101", "task-102", "task-103", "task-104",
    "task-105", "task-106", "task-107", "task-108", "task-109", "task-110", "task-111", "task-112",
    "task-113", "task-114", "task-115", "task-116", "task-117", "task-118", "task-119", "task-120",
    "task-121", "task-122", "task-123", "task-124", "task-125", "task-126", "task-127", "task-128",
    "task-129", "task-130", "task-131", "task-132", "task-133", "task-134", "task-135", "task-136",
    "task-137", "task-138", "task-139", "task-140", "task-141", "task-142", "task-143", "task-144",
    "task-145", "task-146", "task-147", "task-148", "task-149", "task-150", "task-151", "task-152",
    "task-153", "task-154", "task-155", "task-156", "task-157", "task-158",
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
        task_178::ID => (
            task_178::ID,
            task_178::NAME,
            task_178::KIND,
            task_178::run(ctx),
        ),
        task_179::ID => (
            task_179::ID,
            task_179::NAME,
            task_179::KIND,
            task_179::run(ctx),
        ),
        task_180::ID => (
            task_180::ID,
            task_180::NAME,
            task_180::KIND,
            task_180::run(ctx),
        ),
        task_181::ID => (
            task_181::ID,
            task_181::NAME,
            task_181::KIND,
            task_181::run(ctx),
        ),
        task_182::ID => (
            task_182::ID,
            task_182::NAME,
            task_182::KIND,
            task_182::run(ctx),
        ),
        task_183::ID => (
            task_183::ID,
            task_183::NAME,
            task_183::KIND,
            task_183::run(ctx),
        ),
        task_184::ID => (
            task_184::ID,
            task_184::NAME,
            task_184::KIND,
            task_184::run(ctx),
        ),
        task_185::ID => (
            task_185::ID,
            task_185::NAME,
            task_185::KIND,
            task_185::run(ctx),
        ),
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
        task_101::ID => (
            task_101::ID,
            task_101::NAME,
            task_101::KIND,
            task_101::run(ctx),
        ),
        task_102::ID => (
            task_102::ID,
            task_102::NAME,
            task_102::KIND,
            task_102::run(ctx),
        ),
        task_103::ID => (
            task_103::ID,
            task_103::NAME,
            task_103::KIND,
            task_103::run(ctx),
        ),
        task_104::ID => (
            task_104::ID,
            task_104::NAME,
            task_104::KIND,
            task_104::run(ctx),
        ),
        task_105::ID => (
            task_105::ID,
            task_105::NAME,
            task_105::KIND,
            task_105::run(ctx),
        ),
        task_106::ID => (
            task_106::ID,
            task_106::NAME,
            task_106::KIND,
            task_106::run(ctx),
        ),
        task_107::ID => (
            task_107::ID,
            task_107::NAME,
            task_107::KIND,
            task_107::run(ctx),
        ),
        task_108::ID => (
            task_108::ID,
            task_108::NAME,
            task_108::KIND,
            task_108::run(ctx),
        ),
        task_109::ID => (
            task_109::ID,
            task_109::NAME,
            task_109::KIND,
            task_109::run(ctx),
        ),
        task_110::ID => (
            task_110::ID,
            task_110::NAME,
            task_110::KIND,
            task_110::run(ctx),
        ),
        task_111::ID => (
            task_111::ID,
            task_111::NAME,
            task_111::KIND,
            task_111::run(ctx),
        ),
        task_112::ID => (
            task_112::ID,
            task_112::NAME,
            task_112::KIND,
            task_112::run(ctx),
        ),
        task_113::ID => (
            task_113::ID,
            task_113::NAME,
            task_113::KIND,
            task_113::run(ctx),
        ),
        task_114::ID => (
            task_114::ID,
            task_114::NAME,
            task_114::KIND,
            task_114::run(ctx),
        ),
        task_115::ID => (
            task_115::ID,
            task_115::NAME,
            task_115::KIND,
            task_115::run(ctx),
        ),
        task_116::ID => (
            task_116::ID,
            task_116::NAME,
            task_116::KIND,
            task_116::run(ctx),
        ),
        task_117::ID => (
            task_117::ID,
            task_117::NAME,
            task_117::KIND,
            task_117::run(ctx),
        ),
        task_118::ID => (
            task_118::ID,
            task_118::NAME,
            task_118::KIND,
            task_118::run(ctx),
        ),
        task_119::ID => (
            task_119::ID,
            task_119::NAME,
            task_119::KIND,
            task_119::run(ctx),
        ),
        task_120::ID => (
            task_120::ID,
            task_120::NAME,
            task_120::KIND,
            task_120::run(ctx),
        ),
        task_121::ID => (
            task_121::ID,
            task_121::NAME,
            task_121::KIND,
            task_121::run(ctx),
        ),
        task_122::ID => (
            task_122::ID,
            task_122::NAME,
            task_122::KIND,
            task_122::run(ctx),
        ),
        task_123::ID => (
            task_123::ID,
            task_123::NAME,
            task_123::KIND,
            task_123::run(ctx),
        ),
        task_124::ID => (
            task_124::ID,
            task_124::NAME,
            task_124::KIND,
            task_124::run(ctx),
        ),
        task_125::ID => (
            task_125::ID,
            task_125::NAME,
            task_125::KIND,
            task_125::run(ctx),
        ),
        task_126::ID => (
            task_126::ID,
            task_126::NAME,
            task_126::KIND,
            task_126::run(ctx),
        ),
        task_127::ID => (
            task_127::ID,
            task_127::NAME,
            task_127::KIND,
            task_127::run(ctx),
        ),
        task_128::ID => (
            task_128::ID,
            task_128::NAME,
            task_128::KIND,
            task_128::run(ctx),
        ),
        task_129::ID => (
            task_129::ID,
            task_129::NAME,
            task_129::KIND,
            task_129::run(ctx),
        ),
        task_130::ID => (
            task_130::ID,
            task_130::NAME,
            task_130::KIND,
            task_130::run(ctx),
        ),
        task_131::ID => (
            task_131::ID,
            task_131::NAME,
            task_131::KIND,
            task_131::run(ctx),
        ),
        task_132::ID => (
            task_132::ID,
            task_132::NAME,
            task_132::KIND,
            task_132::run(ctx),
        ),
        task_133::ID => (
            task_133::ID,
            task_133::NAME,
            task_133::KIND,
            task_133::run(ctx),
        ),
        task_134::ID => (
            task_134::ID,
            task_134::NAME,
            task_134::KIND,
            task_134::run(ctx),
        ),
        task_135::ID => (
            task_135::ID,
            task_135::NAME,
            task_135::KIND,
            task_135::run(ctx),
        ),
        task_141::ID => (
            task_141::ID,
            task_141::NAME,
            task_141::KIND,
            task_141::run(ctx),
        ),
        task_142::ID => (
            task_142::ID,
            task_142::NAME,
            task_142::KIND,
            task_142::run(ctx),
        ),
        task_143::ID => (
            task_143::ID,
            task_143::NAME,
            task_143::KIND,
            task_143::run(ctx),
        ),
        task_144::ID => (
            task_144::ID,
            task_144::NAME,
            task_144::KIND,
            task_144::run(ctx),
        ),
        task_145::ID => (
            task_145::ID,
            task_145::NAME,
            task_145::KIND,
            task_145::run(ctx),
        ),
        task_146::ID => (
            task_146::ID,
            task_146::NAME,
            task_146::KIND,
            task_146::run(ctx),
        ),
        task_147::ID => (
            task_147::ID,
            task_147::NAME,
            task_147::KIND,
            task_147::run(ctx),
        ),
        task_148::ID => (
            task_148::ID,
            task_148::NAME,
            task_148::KIND,
            task_148::run(ctx),
        ),
        task_149::ID => (
            task_149::ID,
            task_149::NAME,
            task_149::KIND,
            task_149::run(ctx),
        ),
        task_150::ID => (
            task_150::ID,
            task_150::NAME,
            task_150::KIND,
            task_150::run(ctx),
        ),
        task_151::ID => (
            task_151::ID,
            task_151::NAME,
            task_151::KIND,
            task_151::run(ctx),
        ),
        task_152::ID => (
            task_152::ID,
            task_152::NAME,
            task_152::KIND,
            task_152::run(ctx),
        ),
        task_153::ID => (
            task_153::ID,
            task_153::NAME,
            task_153::KIND,
            task_153::run(ctx),
        ),
        task_154::ID => (
            task_154::ID,
            task_154::NAME,
            task_154::KIND,
            task_154::run(ctx),
        ),
        task_155::ID => (
            task_155::ID,
            task_155::NAME,
            task_155::KIND,
            task_155::run(ctx),
        ),
        task_156::ID => (
            task_156::ID,
            task_156::NAME,
            task_156::KIND,
            task_156::run(ctx),
        ),
        task_157::ID => (
            task_157::ID,
            task_157::NAME,
            task_157::KIND,
            task_157::run(ctx),
        ),
        task_158::ID => (
            task_158::ID,
            task_158::NAME,
            task_158::KIND,
            task_158::run(ctx),
        ),
        task_136::ID => (
            task_136::ID,
            task_136::NAME,
            task_136::KIND,
            task_136::run(ctx),
        ),
        task_137::ID => (
            task_137::ID,
            task_137::NAME,
            task_137::KIND,
            task_137::run(ctx),
        ),
        task_138::ID => (
            task_138::ID,
            task_138::NAME,
            task_138::KIND,
            task_138::run(ctx),
        ),
        task_139::ID => (
            task_139::ID,
            task_139::NAME,
            task_139::KIND,
            task_139::run(ctx),
        ),
        task_140::ID => (
            task_140::ID,
            task_140::NAME,
            task_140::KIND,
            task_140::run(ctx),
        ),
        task_159::ID => (
            task_159::ID,
            task_159::NAME,
            task_159::KIND,
            task_159::run(ctx),
        ),
        task_160::ID => (
            task_160::ID,
            task_160::NAME,
            task_160::KIND,
            task_160::run(ctx),
        ),
        task_161::ID => (
            task_161::ID,
            task_161::NAME,
            task_161::KIND,
            task_161::run(ctx),
        ),
        task_162::ID => (
            task_162::ID,
            task_162::NAME,
            task_162::KIND,
            task_162::run(ctx),
        ),
        task_163::ID => (
            task_163::ID,
            task_163::NAME,
            task_163::KIND,
            task_163::run(ctx),
        ),
        task_164::ID => (
            task_164::ID,
            task_164::NAME,
            task_164::KIND,
            task_164::run(ctx),
        ),
        task_165::ID => (
            task_165::ID,
            task_165::NAME,
            task_165::KIND,
            task_165::run(ctx),
        ),
        task_166::ID => (
            task_166::ID,
            task_166::NAME,
            task_166::KIND,
            task_166::run(ctx),
        ),
        task_167::ID => (
            task_167::ID,
            task_167::NAME,
            task_167::KIND,
            task_167::run(ctx),
        ),
        task_168::ID => (
            task_168::ID,
            task_168::NAME,
            task_168::KIND,
            task_168::run(ctx),
        ),
        task_169::ID => (
            task_169::ID,
            task_169::NAME,
            task_169::KIND,
            task_169::run(ctx),
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
        task_178::ID => Some((task_178::NAME, task_178::KIND)),
        task_179::ID => Some((task_179::NAME, task_179::KIND)),
        task_180::ID => Some((task_180::NAME, task_180::KIND)),
        task_181::ID => Some((task_181::NAME, task_181::KIND)),
        task_182::ID => Some((task_182::NAME, task_182::KIND)),
        task_183::ID => Some((task_183::NAME, task_183::KIND)),
        task_184::ID => Some((task_184::NAME, task_184::KIND)),
        task_185::ID => Some((task_185::NAME, task_185::KIND)),
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
        task_101::ID => Some((task_101::NAME, task_101::KIND)),
        task_102::ID => Some((task_102::NAME, task_102::KIND)),
        task_103::ID => Some((task_103::NAME, task_103::KIND)),
        task_104::ID => Some((task_104::NAME, task_104::KIND)),
        task_105::ID => Some((task_105::NAME, task_105::KIND)),
        task_106::ID => Some((task_106::NAME, task_106::KIND)),
        task_107::ID => Some((task_107::NAME, task_107::KIND)),
        task_108::ID => Some((task_108::NAME, task_108::KIND)),
        task_109::ID => Some((task_109::NAME, task_109::KIND)),
        task_110::ID => Some((task_110::NAME, task_110::KIND)),
        task_111::ID => Some((task_111::NAME, task_111::KIND)),
        task_112::ID => Some((task_112::NAME, task_112::KIND)),
        task_113::ID => Some((task_113::NAME, task_113::KIND)),
        task_114::ID => Some((task_114::NAME, task_114::KIND)),
        task_115::ID => Some((task_115::NAME, task_115::KIND)),
        task_116::ID => Some((task_116::NAME, task_116::KIND)),
        task_117::ID => Some((task_117::NAME, task_117::KIND)),
        task_118::ID => Some((task_118::NAME, task_118::KIND)),
        task_119::ID => Some((task_119::NAME, task_119::KIND)),
        task_120::ID => Some((task_120::NAME, task_120::KIND)),
        task_121::ID => Some((task_121::NAME, task_121::KIND)),
        task_122::ID => Some((task_122::NAME, task_122::KIND)),
        task_123::ID => Some((task_123::NAME, task_123::KIND)),
        task_124::ID => Some((task_124::NAME, task_124::KIND)),
        task_125::ID => Some((task_125::NAME, task_125::KIND)),
        task_126::ID => Some((task_126::NAME, task_126::KIND)),
        task_127::ID => Some((task_127::NAME, task_127::KIND)),
        task_128::ID => Some((task_128::NAME, task_128::KIND)),
        task_129::ID => Some((task_129::NAME, task_129::KIND)),
        task_130::ID => Some((task_130::NAME, task_130::KIND)),
        task_131::ID => Some((task_131::NAME, task_131::KIND)),
        task_132::ID => Some((task_132::NAME, task_132::KIND)),
        task_133::ID => Some((task_133::NAME, task_133::KIND)),
        task_134::ID => Some((task_134::NAME, task_134::KIND)),
        task_135::ID => Some((task_135::NAME, task_135::KIND)),
        task_141::ID => Some((task_141::NAME, task_141::KIND)),
        task_142::ID => Some((task_142::NAME, task_142::KIND)),
        task_143::ID => Some((task_143::NAME, task_143::KIND)),
        task_144::ID => Some((task_144::NAME, task_144::KIND)),
        task_145::ID => Some((task_145::NAME, task_145::KIND)),
        task_146::ID => Some((task_146::NAME, task_146::KIND)),
        task_147::ID => Some((task_147::NAME, task_147::KIND)),
        task_148::ID => Some((task_148::NAME, task_148::KIND)),
        task_149::ID => Some((task_149::NAME, task_149::KIND)),
        task_150::ID => Some((task_150::NAME, task_150::KIND)),
        task_151::ID => Some((task_151::NAME, task_151::KIND)),
        task_152::ID => Some((task_152::NAME, task_152::KIND)),
        task_153::ID => Some((task_153::NAME, task_153::KIND)),
        task_154::ID => Some((task_154::NAME, task_154::KIND)),
        task_155::ID => Some((task_155::NAME, task_155::KIND)),
        task_156::ID => Some((task_156::NAME, task_156::KIND)),
        task_157::ID => Some((task_157::NAME, task_157::KIND)),
        task_158::ID => Some((task_158::NAME, task_158::KIND)),
        task_136::ID => Some((task_136::NAME, task_136::KIND)),
        task_137::ID => Some((task_137::NAME, task_137::KIND)),
        task_138::ID => Some((task_138::NAME, task_138::KIND)),
        task_139::ID => Some((task_139::NAME, task_139::KIND)),
        task_140::ID => Some((task_140::NAME, task_140::KIND)),
        task_159::ID => Some((task_159::NAME, task_159::KIND)),
        task_160::ID => Some((task_160::NAME, task_160::KIND)),
        task_161::ID => Some((task_161::NAME, task_161::KIND)),
        task_162::ID => Some((task_162::NAME, task_162::KIND)),
        task_163::ID => Some((task_163::NAME, task_163::KIND)),
        task_164::ID => Some((task_164::NAME, task_164::KIND)),
        task_165::ID => Some((task_165::NAME, task_165::KIND)),
        task_166::ID => Some((task_166::NAME, task_166::KIND)),
        task_167::ID => Some((task_167::NAME, task_167::KIND)),
        task_168::ID => Some((task_168::NAME, task_168::KIND)),
        task_169::ID => Some((task_169::NAME, task_169::KIND)),
        _ => None,
    }
}
