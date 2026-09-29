//! The 130 gauntlet tasks. One module per task; each file has a single
//! owning worker during implementation (disjoint ownership).

/// Shared subprocess harness for the wave-32 (CLI UX doctrine) task
/// drivers. Crate-visible only: integration tests go through the
/// drivers, never the harness directly.
pub(crate) mod cli_harness;

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
pub mod task_186;
pub mod task_187;
pub mod task_188;
pub mod task_189;
pub mod task_19;
pub mod task_190;
pub mod task_191;
pub mod task_192;
pub mod task_193;
pub mod task_194;
pub mod task_195;
pub mod task_196;
pub mod task_197;
pub mod task_198;
pub mod task_199;
pub mod task_20;
pub mod task_200;
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

pub mod task_201;
pub mod task_202;
pub mod task_203;
pub mod task_204;
pub mod task_205;
pub mod task_206;
pub mod task_207;
pub mod task_208;
pub mod task_209;
pub mod task_210;
pub mod task_211;
pub mod task_212;
pub mod task_213;
pub mod task_214;
pub mod task_215;
pub mod task_216;
pub mod task_217;
pub mod task_218;
pub mod task_219;
pub mod task_220;
pub mod task_221;
pub mod task_222;
pub mod task_223;
pub mod task_224;
pub mod task_225;
pub mod task_226;
pub mod task_227;
pub mod task_228;
pub mod task_229;
pub mod task_230;
pub mod task_231;
pub mod task_232;
pub mod task_233;
pub mod task_234;
pub mod task_235;
pub mod task_236;
pub mod task_237;
pub mod task_238;
pub mod task_239;
pub mod task_240;
pub mod task_241;
pub mod task_242;
pub mod task_243;
pub mod task_244;
pub mod task_245;
pub mod task_246;
pub mod task_247;
pub mod task_248;
pub mod task_249;
pub mod task_250;

use crate::{Ctx, GauntletError, TaskKind, TaskOutcome, TaskReport};
use std::time::Instant;

/// One authoritative registry entry per task: the id, its metadata, and
/// its dispatch arm are a single record. The wave-28 incident happened
/// because `TASK_IDS`, `run_task`'s match, `task_meta`'s match, and the
/// `is_wired` probe were four hand-maintained parallel tables; a task
/// could be listed while its arms were missing elsewhere. With one table,
/// that failure shape is unrepresentable: removing an entry removes the
/// listing, the dispatch arm, and the metadata together.
#[derive(Clone, Copy)]
pub struct TaskEntry {
    /// Task id, e.g. `"task-01"`.
    pub id: &'static str,
    /// Human-readable name.
    pub name: &'static str,
    /// How the task is driven.
    pub kind: TaskKind,
    /// Dispatch arm. Always present: entries are constructed whole, never partial.
    pub run: fn(&Ctx) -> TaskOutcome,
}

/// The single authoritative task registry, in run order. `TASK_IDS`,
/// `run_task`, `task_meta`, `is_wired`, and the ledger verifier all read
/// this table, so the views cannot drift apart.
pub const TASKS: [TaskEntry; crate::TASK_COUNT_MAX] = [
    TaskEntry {
        id: task_01::ID,
        name: task_01::NAME,
        kind: task_01::KIND,
        run: task_01::run,
    },
    TaskEntry {
        id: task_02::ID,
        name: task_02::NAME,
        kind: task_02::KIND,
        run: task_02::run,
    },
    TaskEntry {
        id: task_03::ID,
        name: task_03::NAME,
        kind: task_03::KIND,
        run: task_03::run,
    },
    TaskEntry {
        id: task_04::ID,
        name: task_04::NAME,
        kind: task_04::KIND,
        run: task_04::run,
    },
    TaskEntry {
        id: task_05::ID,
        name: task_05::NAME,
        kind: task_05::KIND,
        run: task_05::run,
    },
    TaskEntry {
        id: task_06::ID,
        name: task_06::NAME,
        kind: task_06::KIND,
        run: task_06::run,
    },
    TaskEntry {
        id: task_07::ID,
        name: task_07::NAME,
        kind: task_07::KIND,
        run: task_07::run,
    },
    TaskEntry {
        id: task_08::ID,
        name: task_08::NAME,
        kind: task_08::KIND,
        run: task_08::run,
    },
    TaskEntry {
        id: task_09::ID,
        name: task_09::NAME,
        kind: task_09::KIND,
        run: task_09::run,
    },
    TaskEntry {
        id: task_10::ID,
        name: task_10::NAME,
        kind: task_10::KIND,
        run: task_10::run,
    },
    TaskEntry {
        id: task_11::ID,
        name: task_11::NAME,
        kind: task_11::KIND,
        run: task_11::run,
    },
    TaskEntry {
        id: task_12::ID,
        name: task_12::NAME,
        kind: task_12::KIND,
        run: task_12::run,
    },
    TaskEntry {
        id: task_13::ID,
        name: task_13::NAME,
        kind: task_13::KIND,
        run: task_13::run,
    },
    TaskEntry {
        id: task_14::ID,
        name: task_14::NAME,
        kind: task_14::KIND,
        run: task_14::run,
    },
    TaskEntry {
        id: task_15::ID,
        name: task_15::NAME,
        kind: task_15::KIND,
        run: task_15::run,
    },
    TaskEntry {
        id: task_16::ID,
        name: task_16::NAME,
        kind: task_16::KIND,
        run: task_16::run,
    },
    TaskEntry {
        id: task_17::ID,
        name: task_17::NAME,
        kind: task_17::KIND,
        run: task_17::run,
    },
    TaskEntry {
        id: task_18::ID,
        name: task_18::NAME,
        kind: task_18::KIND,
        run: task_18::run,
    },
    TaskEntry {
        id: task_19::ID,
        name: task_19::NAME,
        kind: task_19::KIND,
        run: task_19::run,
    },
    TaskEntry {
        id: task_20::ID,
        name: task_20::NAME,
        kind: task_20::KIND,
        run: task_20::run,
    },
    TaskEntry {
        id: task_21::ID,
        name: task_21::NAME,
        kind: task_21::KIND,
        run: task_21::run,
    },
    TaskEntry {
        id: task_22::ID,
        name: task_22::NAME,
        kind: task_22::KIND,
        run: task_22::run,
    },
    TaskEntry {
        id: task_23::ID,
        name: task_23::NAME,
        kind: task_23::KIND,
        run: task_23::run,
    },
    TaskEntry {
        id: task_24::ID,
        name: task_24::NAME,
        kind: task_24::KIND,
        run: task_24::run,
    },
    TaskEntry {
        id: task_25::ID,
        name: task_25::NAME,
        kind: task_25::KIND,
        run: task_25::run,
    },
    TaskEntry {
        id: task_26::ID,
        name: task_26::NAME,
        kind: task_26::KIND,
        run: task_26::run,
    },
    TaskEntry {
        id: task_27::ID,
        name: task_27::NAME,
        kind: task_27::KIND,
        run: task_27::run,
    },
    TaskEntry {
        id: task_28::ID,
        name: task_28::NAME,
        kind: task_28::KIND,
        run: task_28::run,
    },
    TaskEntry {
        id: task_29::ID,
        name: task_29::NAME,
        kind: task_29::KIND,
        run: task_29::run,
    },
    TaskEntry {
        id: task_30::ID,
        name: task_30::NAME,
        kind: task_30::KIND,
        run: task_30::run,
    },
    TaskEntry {
        id: task_31::ID,
        name: task_31::NAME,
        kind: task_31::KIND,
        run: task_31::run,
    },
    TaskEntry {
        id: task_32::ID,
        name: task_32::NAME,
        kind: task_32::KIND,
        run: task_32::run,
    },
    TaskEntry {
        id: task_33::ID,
        name: task_33::NAME,
        kind: task_33::KIND,
        run: task_33::run,
    },
    TaskEntry {
        id: task_34::ID,
        name: task_34::NAME,
        kind: task_34::KIND,
        run: task_34::run,
    },
    TaskEntry {
        id: task_35::ID,
        name: task_35::NAME,
        kind: task_35::KIND,
        run: task_35::run,
    },
    TaskEntry {
        id: task_36::ID,
        name: task_36::NAME,
        kind: task_36::KIND,
        run: task_36::run,
    },
    TaskEntry {
        id: task_37::ID,
        name: task_37::NAME,
        kind: task_37::KIND,
        run: task_37::run,
    },
    TaskEntry {
        id: task_38::ID,
        name: task_38::NAME,
        kind: task_38::KIND,
        run: task_38::run,
    },
    TaskEntry {
        id: task_39::ID,
        name: task_39::NAME,
        kind: task_39::KIND,
        run: task_39::run,
    },
    TaskEntry {
        id: task_40::ID,
        name: task_40::NAME,
        kind: task_40::KIND,
        run: task_40::run,
    },
    TaskEntry {
        id: task_41::ID,
        name: task_41::NAME,
        kind: task_41::KIND,
        run: task_41::run,
    },
    TaskEntry {
        id: task_42::ID,
        name: task_42::NAME,
        kind: task_42::KIND,
        run: task_42::run,
    },
    TaskEntry {
        id: task_43::ID,
        name: task_43::NAME,
        kind: task_43::KIND,
        run: task_43::run,
    },
    TaskEntry {
        id: task_44::ID,
        name: task_44::NAME,
        kind: task_44::KIND,
        run: task_44::run,
    },
    TaskEntry {
        id: task_45::ID,
        name: task_45::NAME,
        kind: task_45::KIND,
        run: task_45::run,
    },
    TaskEntry {
        id: task_46::ID,
        name: task_46::NAME,
        kind: task_46::KIND,
        run: task_46::run,
    },
    TaskEntry {
        id: task_47::ID,
        name: task_47::NAME,
        kind: task_47::KIND,
        run: task_47::run,
    },
    TaskEntry {
        id: task_48::ID,
        name: task_48::NAME,
        kind: task_48::KIND,
        run: task_48::run,
    },
    TaskEntry {
        id: task_49::ID,
        name: task_49::NAME,
        kind: task_49::KIND,
        run: task_49::run,
    },
    TaskEntry {
        id: task_50::ID,
        name: task_50::NAME,
        kind: task_50::KIND,
        run: task_50::run,
    },
    TaskEntry {
        id: task_51::ID,
        name: task_51::NAME,
        kind: task_51::KIND,
        run: task_51::run,
    },
    TaskEntry {
        id: task_52::ID,
        name: task_52::NAME,
        kind: task_52::KIND,
        run: task_52::run,
    },
    TaskEntry {
        id: task_53::ID,
        name: task_53::NAME,
        kind: task_53::KIND,
        run: task_53::run,
    },
    TaskEntry {
        id: task_54::ID,
        name: task_54::NAME,
        kind: task_54::KIND,
        run: task_54::run,
    },
    TaskEntry {
        id: task_55::ID,
        name: task_55::NAME,
        kind: task_55::KIND,
        run: task_55::run,
    },
    TaskEntry {
        id: task_56::ID,
        name: task_56::NAME,
        kind: task_56::KIND,
        run: task_56::run,
    },
    TaskEntry {
        id: task_57::ID,
        name: task_57::NAME,
        kind: task_57::KIND,
        run: task_57::run,
    },
    TaskEntry {
        id: task_58::ID,
        name: task_58::NAME,
        kind: task_58::KIND,
        run: task_58::run,
    },
    TaskEntry {
        id: task_59::ID,
        name: task_59::NAME,
        kind: task_59::KIND,
        run: task_59::run,
    },
    TaskEntry {
        id: task_60::ID,
        name: task_60::NAME,
        kind: task_60::KIND,
        run: task_60::run,
    },
    TaskEntry {
        id: task_61::ID,
        name: task_61::NAME,
        kind: task_61::KIND,
        run: task_61::run,
    },
    TaskEntry {
        id: task_62::ID,
        name: task_62::NAME,
        kind: task_62::KIND,
        run: task_62::run,
    },
    TaskEntry {
        id: task_63::ID,
        name: task_63::NAME,
        kind: task_63::KIND,
        run: task_63::run,
    },
    TaskEntry {
        id: task_64::ID,
        name: task_64::NAME,
        kind: task_64::KIND,
        run: task_64::run,
    },
    TaskEntry {
        id: task_65::ID,
        name: task_65::NAME,
        kind: task_65::KIND,
        run: task_65::run,
    },
    TaskEntry {
        id: task_66::ID,
        name: task_66::NAME,
        kind: task_66::KIND,
        run: task_66::run,
    },
    TaskEntry {
        id: task_67::ID,
        name: task_67::NAME,
        kind: task_67::KIND,
        run: task_67::run,
    },
    TaskEntry {
        id: task_68::ID,
        name: task_68::NAME,
        kind: task_68::KIND,
        run: task_68::run,
    },
    TaskEntry {
        id: task_69::ID,
        name: task_69::NAME,
        kind: task_69::KIND,
        run: task_69::run,
    },
    TaskEntry {
        id: task_70::ID,
        name: task_70::NAME,
        kind: task_70::KIND,
        run: task_70::run,
    },
    TaskEntry {
        id: task_71::ID,
        name: task_71::NAME,
        kind: task_71::KIND,
        run: task_71::run,
    },
    TaskEntry {
        id: task_72::ID,
        name: task_72::NAME,
        kind: task_72::KIND,
        run: task_72::run,
    },
    TaskEntry {
        id: task_73::ID,
        name: task_73::NAME,
        kind: task_73::KIND,
        run: task_73::run,
    },
    TaskEntry {
        id: task_74::ID,
        name: task_74::NAME,
        kind: task_74::KIND,
        run: task_74::run,
    },
    TaskEntry {
        id: task_75::ID,
        name: task_75::NAME,
        kind: task_75::KIND,
        run: task_75::run,
    },
    TaskEntry {
        id: task_76::ID,
        name: task_76::NAME,
        kind: task_76::KIND,
        run: task_76::run,
    },
    TaskEntry {
        id: task_77::ID,
        name: task_77::NAME,
        kind: task_77::KIND,
        run: task_77::run,
    },
    TaskEntry {
        id: task_78::ID,
        name: task_78::NAME,
        kind: task_78::KIND,
        run: task_78::run,
    },
    TaskEntry {
        id: task_79::ID,
        name: task_79::NAME,
        kind: task_79::KIND,
        run: task_79::run,
    },
    TaskEntry {
        id: task_80::ID,
        name: task_80::NAME,
        kind: task_80::KIND,
        run: task_80::run,
    },
    TaskEntry {
        id: task_81::ID,
        name: task_81::NAME,
        kind: task_81::KIND,
        run: task_81::run,
    },
    TaskEntry {
        id: task_82::ID,
        name: task_82::NAME,
        kind: task_82::KIND,
        run: task_82::run,
    },
    TaskEntry {
        id: task_83::ID,
        name: task_83::NAME,
        kind: task_83::KIND,
        run: task_83::run,
    },
    TaskEntry {
        id: task_84::ID,
        name: task_84::NAME,
        kind: task_84::KIND,
        run: task_84::run,
    },
    TaskEntry {
        id: task_85::ID,
        name: task_85::NAME,
        kind: task_85::KIND,
        run: task_85::run,
    },
    TaskEntry {
        id: task_86::ID,
        name: task_86::NAME,
        kind: task_86::KIND,
        run: task_86::run,
    },
    TaskEntry {
        id: task_87::ID,
        name: task_87::NAME,
        kind: task_87::KIND,
        run: task_87::run,
    },
    TaskEntry {
        id: task_88::ID,
        name: task_88::NAME,
        kind: task_88::KIND,
        run: task_88::run,
    },
    TaskEntry {
        id: task_89::ID,
        name: task_89::NAME,
        kind: task_89::KIND,
        run: task_89::run,
    },
    TaskEntry {
        id: task_90::ID,
        name: task_90::NAME,
        kind: task_90::KIND,
        run: task_90::run,
    },
    TaskEntry {
        id: task_91::ID,
        name: task_91::NAME,
        kind: task_91::KIND,
        run: task_91::run,
    },
    TaskEntry {
        id: task_92::ID,
        name: task_92::NAME,
        kind: task_92::KIND,
        run: task_92::run,
    },
    TaskEntry {
        id: task_93::ID,
        name: task_93::NAME,
        kind: task_93::KIND,
        run: task_93::run,
    },
    TaskEntry {
        id: task_94::ID,
        name: task_94::NAME,
        kind: task_94::KIND,
        run: task_94::run,
    },
    TaskEntry {
        id: task_95::ID,
        name: task_95::NAME,
        kind: task_95::KIND,
        run: task_95::run,
    },
    TaskEntry {
        id: task_96::ID,
        name: task_96::NAME,
        kind: task_96::KIND,
        run: task_96::run,
    },
    TaskEntry {
        id: task_97::ID,
        name: task_97::NAME,
        kind: task_97::KIND,
        run: task_97::run,
    },
    TaskEntry {
        id: task_98::ID,
        name: task_98::NAME,
        kind: task_98::KIND,
        run: task_98::run,
    },
    TaskEntry {
        id: task_99::ID,
        name: task_99::NAME,
        kind: task_99::KIND,
        run: task_99::run,
    },
    TaskEntry {
        id: task_100::ID,
        name: task_100::NAME,
        kind: task_100::KIND,
        run: task_100::run,
    },
    TaskEntry {
        id: task_101::ID,
        name: task_101::NAME,
        kind: task_101::KIND,
        run: task_101::run,
    },
    TaskEntry {
        id: task_102::ID,
        name: task_102::NAME,
        kind: task_102::KIND,
        run: task_102::run,
    },
    TaskEntry {
        id: task_103::ID,
        name: task_103::NAME,
        kind: task_103::KIND,
        run: task_103::run,
    },
    TaskEntry {
        id: task_104::ID,
        name: task_104::NAME,
        kind: task_104::KIND,
        run: task_104::run,
    },
    TaskEntry {
        id: task_105::ID,
        name: task_105::NAME,
        kind: task_105::KIND,
        run: task_105::run,
    },
    TaskEntry {
        id: task_106::ID,
        name: task_106::NAME,
        kind: task_106::KIND,
        run: task_106::run,
    },
    TaskEntry {
        id: task_107::ID,
        name: task_107::NAME,
        kind: task_107::KIND,
        run: task_107::run,
    },
    TaskEntry {
        id: task_108::ID,
        name: task_108::NAME,
        kind: task_108::KIND,
        run: task_108::run,
    },
    TaskEntry {
        id: task_109::ID,
        name: task_109::NAME,
        kind: task_109::KIND,
        run: task_109::run,
    },
    TaskEntry {
        id: task_110::ID,
        name: task_110::NAME,
        kind: task_110::KIND,
        run: task_110::run,
    },
    TaskEntry {
        id: task_111::ID,
        name: task_111::NAME,
        kind: task_111::KIND,
        run: task_111::run,
    },
    TaskEntry {
        id: task_112::ID,
        name: task_112::NAME,
        kind: task_112::KIND,
        run: task_112::run,
    },
    TaskEntry {
        id: task_113::ID,
        name: task_113::NAME,
        kind: task_113::KIND,
        run: task_113::run,
    },
    TaskEntry {
        id: task_114::ID,
        name: task_114::NAME,
        kind: task_114::KIND,
        run: task_114::run,
    },
    TaskEntry {
        id: task_115::ID,
        name: task_115::NAME,
        kind: task_115::KIND,
        run: task_115::run,
    },
    TaskEntry {
        id: task_116::ID,
        name: task_116::NAME,
        kind: task_116::KIND,
        run: task_116::run,
    },
    TaskEntry {
        id: task_117::ID,
        name: task_117::NAME,
        kind: task_117::KIND,
        run: task_117::run,
    },
    TaskEntry {
        id: task_118::ID,
        name: task_118::NAME,
        kind: task_118::KIND,
        run: task_118::run,
    },
    TaskEntry {
        id: task_119::ID,
        name: task_119::NAME,
        kind: task_119::KIND,
        run: task_119::run,
    },
    TaskEntry {
        id: task_120::ID,
        name: task_120::NAME,
        kind: task_120::KIND,
        run: task_120::run,
    },
    TaskEntry {
        id: task_121::ID,
        name: task_121::NAME,
        kind: task_121::KIND,
        run: task_121::run,
    },
    TaskEntry {
        id: task_122::ID,
        name: task_122::NAME,
        kind: task_122::KIND,
        run: task_122::run,
    },
    TaskEntry {
        id: task_123::ID,
        name: task_123::NAME,
        kind: task_123::KIND,
        run: task_123::run,
    },
    TaskEntry {
        id: task_124::ID,
        name: task_124::NAME,
        kind: task_124::KIND,
        run: task_124::run,
    },
    TaskEntry {
        id: task_125::ID,
        name: task_125::NAME,
        kind: task_125::KIND,
        run: task_125::run,
    },
    TaskEntry {
        id: task_126::ID,
        name: task_126::NAME,
        kind: task_126::KIND,
        run: task_126::run,
    },
    TaskEntry {
        id: task_127::ID,
        name: task_127::NAME,
        kind: task_127::KIND,
        run: task_127::run,
    },
    TaskEntry {
        id: task_128::ID,
        name: task_128::NAME,
        kind: task_128::KIND,
        run: task_128::run,
    },
    TaskEntry {
        id: task_129::ID,
        name: task_129::NAME,
        kind: task_129::KIND,
        run: task_129::run,
    },
    TaskEntry {
        id: task_130::ID,
        name: task_130::NAME,
        kind: task_130::KIND,
        run: task_130::run,
    },
    TaskEntry {
        id: task_131::ID,
        name: task_131::NAME,
        kind: task_131::KIND,
        run: task_131::run,
    },
    TaskEntry {
        id: task_132::ID,
        name: task_132::NAME,
        kind: task_132::KIND,
        run: task_132::run,
    },
    TaskEntry {
        id: task_133::ID,
        name: task_133::NAME,
        kind: task_133::KIND,
        run: task_133::run,
    },
    TaskEntry {
        id: task_134::ID,
        name: task_134::NAME,
        kind: task_134::KIND,
        run: task_134::run,
    },
    TaskEntry {
        id: task_135::ID,
        name: task_135::NAME,
        kind: task_135::KIND,
        run: task_135::run,
    },
    TaskEntry {
        id: task_136::ID,
        name: task_136::NAME,
        kind: task_136::KIND,
        run: task_136::run,
    },
    TaskEntry {
        id: task_137::ID,
        name: task_137::NAME,
        kind: task_137::KIND,
        run: task_137::run,
    },
    TaskEntry {
        id: task_138::ID,
        name: task_138::NAME,
        kind: task_138::KIND,
        run: task_138::run,
    },
    TaskEntry {
        id: task_139::ID,
        name: task_139::NAME,
        kind: task_139::KIND,
        run: task_139::run,
    },
    TaskEntry {
        id: task_140::ID,
        name: task_140::NAME,
        kind: task_140::KIND,
        run: task_140::run,
    },
    TaskEntry {
        id: task_141::ID,
        name: task_141::NAME,
        kind: task_141::KIND,
        run: task_141::run,
    },
    TaskEntry {
        id: task_142::ID,
        name: task_142::NAME,
        kind: task_142::KIND,
        run: task_142::run,
    },
    TaskEntry {
        id: task_143::ID,
        name: task_143::NAME,
        kind: task_143::KIND,
        run: task_143::run,
    },
    TaskEntry {
        id: task_144::ID,
        name: task_144::NAME,
        kind: task_144::KIND,
        run: task_144::run,
    },
    TaskEntry {
        id: task_145::ID,
        name: task_145::NAME,
        kind: task_145::KIND,
        run: task_145::run,
    },
    TaskEntry {
        id: task_146::ID,
        name: task_146::NAME,
        kind: task_146::KIND,
        run: task_146::run,
    },
    TaskEntry {
        id: task_147::ID,
        name: task_147::NAME,
        kind: task_147::KIND,
        run: task_147::run,
    },
    TaskEntry {
        id: task_148::ID,
        name: task_148::NAME,
        kind: task_148::KIND,
        run: task_148::run,
    },
    TaskEntry {
        id: task_149::ID,
        name: task_149::NAME,
        kind: task_149::KIND,
        run: task_149::run,
    },
    TaskEntry {
        id: task_150::ID,
        name: task_150::NAME,
        kind: task_150::KIND,
        run: task_150::run,
    },
    TaskEntry {
        id: task_151::ID,
        name: task_151::NAME,
        kind: task_151::KIND,
        run: task_151::run,
    },
    TaskEntry {
        id: task_152::ID,
        name: task_152::NAME,
        kind: task_152::KIND,
        run: task_152::run,
    },
    TaskEntry {
        id: task_153::ID,
        name: task_153::NAME,
        kind: task_153::KIND,
        run: task_153::run,
    },
    TaskEntry {
        id: task_154::ID,
        name: task_154::NAME,
        kind: task_154::KIND,
        run: task_154::run,
    },
    TaskEntry {
        id: task_155::ID,
        name: task_155::NAME,
        kind: task_155::KIND,
        run: task_155::run,
    },
    TaskEntry {
        id: task_156::ID,
        name: task_156::NAME,
        kind: task_156::KIND,
        run: task_156::run,
    },
    TaskEntry {
        id: task_157::ID,
        name: task_157::NAME,
        kind: task_157::KIND,
        run: task_157::run,
    },
    TaskEntry {
        id: task_158::ID,
        name: task_158::NAME,
        kind: task_158::KIND,
        run: task_158::run,
    },
    TaskEntry {
        id: task_159::ID,
        name: task_159::NAME,
        kind: task_159::KIND,
        run: task_159::run,
    },
    TaskEntry {
        id: task_160::ID,
        name: task_160::NAME,
        kind: task_160::KIND,
        run: task_160::run,
    },
    TaskEntry {
        id: task_161::ID,
        name: task_161::NAME,
        kind: task_161::KIND,
        run: task_161::run,
    },
    TaskEntry {
        id: task_162::ID,
        name: task_162::NAME,
        kind: task_162::KIND,
        run: task_162::run,
    },
    TaskEntry {
        id: task_163::ID,
        name: task_163::NAME,
        kind: task_163::KIND,
        run: task_163::run,
    },
    TaskEntry {
        id: task_164::ID,
        name: task_164::NAME,
        kind: task_164::KIND,
        run: task_164::run,
    },
    TaskEntry {
        id: task_165::ID,
        name: task_165::NAME,
        kind: task_165::KIND,
        run: task_165::run,
    },
    TaskEntry {
        id: task_166::ID,
        name: task_166::NAME,
        kind: task_166::KIND,
        run: task_166::run,
    },
    TaskEntry {
        id: task_167::ID,
        name: task_167::NAME,
        kind: task_167::KIND,
        run: task_167::run,
    },
    TaskEntry {
        id: task_168::ID,
        name: task_168::NAME,
        kind: task_168::KIND,
        run: task_168::run,
    },
    TaskEntry {
        id: task_169::ID,
        name: task_169::NAME,
        kind: task_169::KIND,
        run: task_169::run,
    },
    TaskEntry {
        id: task_170::ID,
        name: task_170::NAME,
        kind: task_170::KIND,
        run: task_170::run,
    },
    TaskEntry {
        id: task_171::ID,
        name: task_171::NAME,
        kind: task_171::KIND,
        run: task_171::run,
    },
    TaskEntry {
        id: task_172::ID,
        name: task_172::NAME,
        kind: task_172::KIND,
        run: task_172::run,
    },
    TaskEntry {
        id: task_173::ID,
        name: task_173::NAME,
        kind: task_173::KIND,
        run: task_173::run,
    },
    TaskEntry {
        id: task_174::ID,
        name: task_174::NAME,
        kind: task_174::KIND,
        run: task_174::run,
    },
    TaskEntry {
        id: task_175::ID,
        name: task_175::NAME,
        kind: task_175::KIND,
        run: task_175::run,
    },
    TaskEntry {
        id: task_176::ID,
        name: task_176::NAME,
        kind: task_176::KIND,
        run: task_176::run,
    },
    TaskEntry {
        id: task_177::ID,
        name: task_177::NAME,
        kind: task_177::KIND,
        run: task_177::run,
    },
    TaskEntry {
        id: task_178::ID,
        name: task_178::NAME,
        kind: task_178::KIND,
        run: task_178::run,
    },
    TaskEntry {
        id: task_179::ID,
        name: task_179::NAME,
        kind: task_179::KIND,
        run: task_179::run,
    },
    TaskEntry {
        id: task_180::ID,
        name: task_180::NAME,
        kind: task_180::KIND,
        run: task_180::run,
    },
    TaskEntry {
        id: task_181::ID,
        name: task_181::NAME,
        kind: task_181::KIND,
        run: task_181::run,
    },
    TaskEntry {
        id: task_182::ID,
        name: task_182::NAME,
        kind: task_182::KIND,
        run: task_182::run,
    },
    TaskEntry {
        id: task_183::ID,
        name: task_183::NAME,
        kind: task_183::KIND,
        run: task_183::run,
    },
    TaskEntry {
        id: task_184::ID,
        name: task_184::NAME,
        kind: task_184::KIND,
        run: task_184::run,
    },
    TaskEntry {
        id: task_185::ID,
        name: task_185::NAME,
        kind: task_185::KIND,
        run: task_185::run,
    },
    TaskEntry {
        id: task_186::ID,
        name: task_186::NAME,
        kind: task_186::KIND,
        run: task_186::run,
    },
    TaskEntry {
        id: task_187::ID,
        name: task_187::NAME,
        kind: task_187::KIND,
        run: task_187::run,
    },
    TaskEntry {
        id: task_188::ID,
        name: task_188::NAME,
        kind: task_188::KIND,
        run: task_188::run,
    },
    TaskEntry {
        id: task_189::ID,
        name: task_189::NAME,
        kind: task_189::KIND,
        run: task_189::run,
    },
    TaskEntry {
        id: task_190::ID,
        name: task_190::NAME,
        kind: task_190::KIND,
        run: task_190::run,
    },
    TaskEntry {
        id: task_191::ID,
        name: task_191::NAME,
        kind: task_191::KIND,
        run: task_191::run,
    },
    TaskEntry {
        id: task_192::ID,
        name: task_192::NAME,
        kind: task_192::KIND,
        run: task_192::run,
    },
    TaskEntry {
        id: task_193::ID,
        name: task_193::NAME,
        kind: task_193::KIND,
        run: task_193::run,
    },
    TaskEntry {
        id: task_194::ID,
        name: task_194::NAME,
        kind: task_194::KIND,
        run: task_194::run,
    },
    TaskEntry {
        id: task_195::ID,
        name: task_195::NAME,
        kind: task_195::KIND,
        run: task_195::run,
    },
    TaskEntry {
        id: task_196::ID,
        name: task_196::NAME,
        kind: task_196::KIND,
        run: task_196::run,
    },
    TaskEntry {
        id: task_197::ID,
        name: task_197::NAME,
        kind: task_197::KIND,
        run: task_197::run,
    },
    TaskEntry {
        id: task_198::ID,
        name: task_198::NAME,
        kind: task_198::KIND,
        run: task_198::run,
    },
    TaskEntry {
        id: task_199::ID,
        name: task_199::NAME,
        kind: task_199::KIND,
        run: task_199::run,
    },
    TaskEntry {
        id: task_200::ID,
        name: task_200::NAME,
        kind: task_200::KIND,
        run: task_200::run,
    },
    TaskEntry {
        id: task_201::ID,
        name: task_201::NAME,
        kind: task_201::KIND,
        run: task_201::run,
    },
    TaskEntry {
        id: task_202::ID,
        name: task_202::NAME,
        kind: task_202::KIND,
        run: task_202::run,
    },
    TaskEntry {
        id: task_203::ID,
        name: task_203::NAME,
        kind: task_203::KIND,
        run: task_203::run,
    },
    TaskEntry {
        id: task_204::ID,
        name: task_204::NAME,
        kind: task_204::KIND,
        run: task_204::run,
    },
    TaskEntry {
        id: task_205::ID,
        name: task_205::NAME,
        kind: task_205::KIND,
        run: task_205::run,
    },
    TaskEntry {
        id: task_206::ID,
        name: task_206::NAME,
        kind: task_206::KIND,
        run: task_206::run,
    },
    TaskEntry {
        id: task_207::ID,
        name: task_207::NAME,
        kind: task_207::KIND,
        run: task_207::run,
    },
    TaskEntry {
        id: task_208::ID,
        name: task_208::NAME,
        kind: task_208::KIND,
        run: task_208::run,
    },
    TaskEntry {
        id: task_209::ID,
        name: task_209::NAME,
        kind: task_209::KIND,
        run: task_209::run,
    },
    TaskEntry {
        id: task_210::ID,
        name: task_210::NAME,
        kind: task_210::KIND,
        run: task_210::run,
    },
    TaskEntry {
        id: task_211::ID,
        name: task_211::NAME,
        kind: task_211::KIND,
        run: task_211::run,
    },
    TaskEntry {
        id: task_212::ID,
        name: task_212::NAME,
        kind: task_212::KIND,
        run: task_212::run,
    },
    TaskEntry {
        id: task_213::ID,
        name: task_213::NAME,
        kind: task_213::KIND,
        run: task_213::run,
    },
    TaskEntry {
        id: task_214::ID,
        name: task_214::NAME,
        kind: task_214::KIND,
        run: task_214::run,
    },
    TaskEntry {
        id: task_215::ID,
        name: task_215::NAME,
        kind: task_215::KIND,
        run: task_215::run,
    },
    TaskEntry {
        id: task_216::ID,
        name: task_216::NAME,
        kind: task_216::KIND,
        run: task_216::run,
    },
    TaskEntry {
        id: task_217::ID,
        name: task_217::NAME,
        kind: task_217::KIND,
        run: task_217::run,
    },
    TaskEntry {
        id: task_218::ID,
        name: task_218::NAME,
        kind: task_218::KIND,
        run: task_218::run,
    },
    TaskEntry {
        id: task_219::ID,
        name: task_219::NAME,
        kind: task_219::KIND,
        run: task_219::run,
    },
    TaskEntry {
        id: task_220::ID,
        name: task_220::NAME,
        kind: task_220::KIND,
        run: task_220::run,
    },
    TaskEntry {
        id: task_221::ID,
        name: task_221::NAME,
        kind: task_221::KIND,
        run: task_221::run,
    },
    TaskEntry {
        id: task_222::ID,
        name: task_222::NAME,
        kind: task_222::KIND,
        run: task_222::run,
    },
    TaskEntry {
        id: task_223::ID,
        name: task_223::NAME,
        kind: task_223::KIND,
        run: task_223::run,
    },
    TaskEntry {
        id: task_224::ID,
        name: task_224::NAME,
        kind: task_224::KIND,
        run: task_224::run,
    },
    TaskEntry {
        id: task_225::ID,
        name: task_225::NAME,
        kind: task_225::KIND,
        run: task_225::run,
    },
    TaskEntry {
        id: task_226::ID,
        name: task_226::NAME,
        kind: task_226::KIND,
        run: task_226::run,
    },
    TaskEntry {
        id: task_227::ID,
        name: task_227::NAME,
        kind: task_227::KIND,
        run: task_227::run,
    },
    TaskEntry {
        id: task_228::ID,
        name: task_228::NAME,
        kind: task_228::KIND,
        run: task_228::run,
    },
    TaskEntry {
        id: task_229::ID,
        name: task_229::NAME,
        kind: task_229::KIND,
        run: task_229::run,
    },
    TaskEntry {
        id: task_230::ID,
        name: task_230::NAME,
        kind: task_230::KIND,
        run: task_230::run,
    },
    TaskEntry {
        id: task_231::ID,
        name: task_231::NAME,
        kind: task_231::KIND,
        run: task_231::run,
    },
    TaskEntry {
        id: task_232::ID,
        name: task_232::NAME,
        kind: task_232::KIND,
        run: task_232::run,
    },
    TaskEntry {
        id: task_233::ID,
        name: task_233::NAME,
        kind: task_233::KIND,
        run: task_233::run,
    },
    TaskEntry {
        id: task_234::ID,
        name: task_234::NAME,
        kind: task_234::KIND,
        run: task_234::run,
    },
    TaskEntry {
        id: task_235::ID,
        name: task_235::NAME,
        kind: task_235::KIND,
        run: task_235::run,
    },
    TaskEntry {
        id: task_236::ID,
        name: task_236::NAME,
        kind: task_236::KIND,
        run: task_236::run,
    },
    TaskEntry {
        id: task_237::ID,
        name: task_237::NAME,
        kind: task_237::KIND,
        run: task_237::run,
    },
    TaskEntry {
        id: task_238::ID,
        name: task_238::NAME,
        kind: task_238::KIND,
        run: task_238::run,
    },
    TaskEntry {
        id: task_239::ID,
        name: task_239::NAME,
        kind: task_239::KIND,
        run: task_239::run,
    },
    TaskEntry {
        id: task_240::ID,
        name: task_240::NAME,
        kind: task_240::KIND,
        run: task_240::run,
    },
    TaskEntry {
        id: task_241::ID,
        name: task_241::NAME,
        kind: task_241::KIND,
        run: task_241::run,
    },
    TaskEntry {
        id: task_242::ID,
        name: task_242::NAME,
        kind: task_242::KIND,
        run: task_242::run,
    },
    TaskEntry {
        id: task_243::ID,
        name: task_243::NAME,
        kind: task_243::KIND,
        run: task_243::run,
    },
    TaskEntry {
        id: task_244::ID,
        name: task_244::NAME,
        kind: task_244::KIND,
        run: task_244::run,
    },
    TaskEntry {
        id: task_245::ID,
        name: task_245::NAME,
        kind: task_245::KIND,
        run: task_245::run,
    },
    TaskEntry {
        id: task_246::ID,
        name: task_246::NAME,
        kind: task_246::KIND,
        run: task_246::run,
    },
    TaskEntry {
        id: task_247::ID,
        name: task_247::NAME,
        kind: task_247::KIND,
        run: task_247::run,
    },
    TaskEntry {
        id: task_248::ID,
        name: task_248::NAME,
        kind: task_248::KIND,
        run: task_248::run,
    },
    TaskEntry {
        id: task_249::ID,
        name: task_249::NAME,
        kind: task_249::KIND,
        run: task_249::run,
    },
    TaskEntry {
        id: task_250::ID,
        name: task_250::NAME,
        kind: task_250::KIND,
        run: task_250::run,
    },
];

/// Derive the closed id list from [`TASKS`] at compile time.
const fn task_ids_from_registry() -> [&'static str; crate::TASK_COUNT_MAX] {
    let mut ids = [""; crate::TASK_COUNT_MAX];
    let mut i = 0;
    while i < crate::TASK_COUNT_MAX {
        ids[i] = TASKS[i].id;
        i += 1;
    }
    ids
}

/// Closed list of task ids, in run order, derived from [`TASKS`]. The list
/// is fixed at `TASK_COUNT_MAX`: extending the gauntlet is a design change.
pub const TASK_IDS: [&str; crate::TASK_COUNT_MAX] = task_ids_from_registry();

/// Registry lookup shared by dispatch, metadata, and the wiring probe.
/// Returns an owned copy: `TASKS` is a `const`, so its entries are copied
/// out rather than borrowed from a temporary.
fn find_entry(id: &str) -> Option<TaskEntry> {
    TASKS.iter().find(|entry| entry.id == id).copied()
}

/// Run one task by id and record a `TaskReport`.
pub fn run_task(id: &str, ctx: &Ctx) -> Result<TaskReport, GauntletError> {
    let started = Instant::now();
    let entry = find_entry(id).ok_or_else(|| GauntletError::UnknownTask { id: id.to_string() })?;
    let outcome = (entry.run)(ctx);
    let duration_ms = started.elapsed().as_millis().min(u64::MAX as u128) as u64;
    Ok(TaskReport {
        id: entry.id,
        name: entry.name,
        kind: entry.kind,
        outcome,
        duration_ms,
    })
}

/// Static metadata for one task id.
pub fn task_meta(id: &str) -> Option<(&'static str, TaskKind)> {
    find_entry(id).map(|entry| (entry.name, entry.kind))
}

/// Gate-zero wiring probe: true exactly when `id` is carried by the registry.
///
/// Added after the wave-28 incident (2026-09-28): tasks 170-177 were listed
/// in `TASK_IDS` and claimed complete by a wave summary, but had arms in
/// neither match, and no gate observed the artifact. The probe used to
/// compare two separate tables; the unified [`TASKS`] registry made that
/// comparison unnecessary --- listing, dispatch, and metadata are one
/// record now. `gauntlet verify-claims` and `gauntlet list` keep calling
/// this probe.
pub fn is_wired(id: &str) -> bool {
    find_entry(id).is_some()
}

#[cfg(test)]
mod wiring_tests {
    use super::*;
    use crate::GauntletError;
    use std::collections::HashSet;
    use std::path::PathBuf;

    /// Regression test for the wave-28 incident: every id advertised in
    /// TASK_IDS must be wired in both tables. Failed for task-170..177
    /// before the fix commit.
    #[test]
    fn every_listed_task_is_wired() {
        for id in TASK_IDS {
            assert!(
                is_wired(id),
                "task '{id}' is listed in TASK_IDS but not wired"
            );
        }
    }

    /// The probe itself must fail closed on unknown ids.
    #[test]
    fn unknown_ids_are_not_wired() {
        assert!(!is_wired("task-999"));
        assert!(!is_wired(""));
        assert!(!is_wired("task-01 "));
    }

    /// Structural: the registry ids are exactly task-01..task-200, in
    /// order, with no gaps or duplicates. A dropped or duplicated entry
    /// fails here before any gate runs.
    #[test]
    fn registry_ids_are_complete_unique_and_ordered() {
        assert_eq!(TASKS.len(), crate::TASK_COUNT_MAX);
        let mut seen = HashSet::with_capacity(TASKS.len());
        for (index, entry) in TASKS.iter().enumerate() {
            let expected = format!("task-{:02}", index + 1);
            assert_eq!(entry.id, expected, "registry out of order at index {index}");
            assert!(
                seen.insert(entry.id),
                "duplicate registry id '{}'",
                entry.id
            );
        }
    }

    /// Structural: dispatch, metadata, the wiring probe, and the public
    /// id list are all views of the one TASKS table, so they cannot
    /// disagree the way the wave-28 tables did.
    #[test]
    fn registry_views_cannot_disagree() {
        assert_eq!(TASKS.len(), TASK_IDS.len());
        for entry in TASKS {
            assert!(is_wired(entry.id), "'{}' not wired", entry.id);
            assert!(
                task_meta(entry.id).is_some(),
                "'{}' has no metadata",
                entry.id
            );
            assert!(
                TASK_IDS.contains(&entry.id),
                "'{}' missing from TASK_IDS",
                entry.id
            );
        }
        for id in TASK_IDS {
            assert!(is_wired(id), "TASK_IDS id '{id}' not wired");
            assert!(
                task_meta(id).is_some(),
                "TASK_IDS id '{id}' has no metadata"
            );
        }
    }

    /// Validation: dispatch fails closed on unknown ids without running
    /// anything. `Ctx::new` needs no real paths for this: the lookup
    /// rejects before the context is touched.
    #[test]
    fn run_task_rejects_unknown_id() {
        let ctx = Ctx::new(
            PathBuf::from("/nonexistent/nvim"),
            PathBuf::from("/nonexistent/diver"),
            PathBuf::from("/nonexistent/work"),
        )
        .expect("Ctx::new must accept non-empty paths");
        let err = run_task("task-999", &ctx).expect_err("unknown id must not dispatch");
        assert!(
            matches!(err, GauntletError::UnknownTask { .. }),
            "unexpected error: {err:?}"
        );
    }
}
