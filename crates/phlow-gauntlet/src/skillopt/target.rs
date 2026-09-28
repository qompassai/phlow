//! Targets: the frozen agents the SkillOpt loop optimizes skills for.
//!
//! [`Target::rollout`] runs one task case under a skill and returns a
//! [`Trajectory`] with an automatic 0/1 reward from an executable
//! verifier (the paper's method "depends heavily on automatically
//! verifiable reward", §IV.1; our fixture verifiers are exactly that).
//!
//! [`ScriptedTarget`] is a MOCK, labeled as such everywhere it appears:
//! a deterministic policy that is a *published* function of skill-doc
//! content (declared below and in each task doc). It includes distractor
//! directives (plausible text that changes nothing, or changes it
//! wrongly) so the optimizer's choice is genuinely under-determined by
//! any single trajectory.
//!
//! Synthetic families (each mirrors one of the paper's learned
//! procedural rules, cited per task):
//!
//! - **F-order** (tool sequencing; cf. the paper's SpreadsheetBench rule —
//!   procedural order discipline). Cases carry a `profile` (0..N); each
//!   profile demands a distinct tool order. The target reads
//!   `ORDER[<profile>]: t1 t2 t3 t4` lines (profiled), falling back to an
//!   unprofiled `ORDER: ...` line, else a default wrong order. Reward 1
//!   iff the emitted order exactly matches the profile's required order.
//! - **F-bind** (evidence binding; cf. the paper's DocVQA rule — "bind
//!   the question to the exact visual row/header/field, then copy only
//!   the aligned answer span"). Reward 1 iff the emitted answer is the
//!   exact source span. Without the exact line
//!   `BIND: quote exact span verbatim` the target paraphrases (marked so
//!   it can never equal the span) → 0.
//! - **F-ledger** (horizon/budget ledger; cf. the paper's ALFWorld rule —
//!   "keep a horizon-aware visited/frontier ledger"). Three canonical
//!   rules (`LEDGER: ...`); reward 1 iff all three slots are active for
//!   the case horizon and the case twist is handled. *Narrow* variants
//!   (`LEDGER: ... for short horizons`) are active only for horizons ≤ 6
//!   and handle twists on short horizons — strictly better than nothing
//!   on short-horizon selection sets, worse on long-horizon test sets.
//!   Twist rules (`LEDGER: on empty frontier, ...`,
//!   `LEDGER: on revisit, ...`) handle the two twist kinds; the scripted
//!   optimizer only proposes them once slow-update guidance names them.

use super::doc::SkillDoc;
use super::rng::XorShift;

/// A synthetic task family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// Tool sequencing.
    FOrder,
    /// Evidence binding (exact-span copy).
    FBind,
    /// Horizon-aware ledger.
    FLedger,
}

/// Twist kinds for F-ledger cases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Twist {
    /// Plain walk.
    None,
    /// Frontier empties mid-walk.
    EmptyFrontier,
    /// A node is revisited.
    Revisit,
}

/// One task case: the unit of rollout.
#[derive(Debug, Clone)]
pub struct TaskCase {
    /// Stable id, unique across the three splits for a seed (mixed
    /// splits: unique across both families too).
    pub id: usize,
    /// Which family (a split may mix families).
    pub family: Family,
    /// F-order: which tool order is required.
    pub profile: u8,
    /// F-ledger: walk length.
    pub horizon: u8,
    /// F-ledger: twist kind.
    pub twist: Twist,
    /// F-bind: index into the source table.
    pub source_idx: usize,
}

/// One rollout: the trajectory plus its automatic reward.
#[derive(Debug, Clone)]
pub struct Trajectory {
    /// The case that ran.
    pub case_id: usize,
    /// Automatic reward, 0 or 1.
    pub reward: u8,
    /// What the verifier expected (for reflection).
    pub expected: String,
    /// What the target emitted (for reflection).
    pub got: String,
}

/// The frozen agent. Implementations must be deterministic given
/// (skill, case): the loop re-evaluates under fixed seeds.
pub trait Target {
    /// Run one case under `skill`.
    fn rollout(&self, skill: &SkillDoc, case: &TaskCase) -> Trajectory;
}

/// How profiles are distributed in generated F-order cases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileSkew {
    /// Uniform over profiles.
    Uniform,
    /// Zipf-ish: profile p has weight 1/(p+1). Rare procedures need
    /// more evidence — the mechanism task-105 measures.
    Zipf,
}

/// Knobs for split construction. One struct so tasks declare their full
/// evidence design in one place.
#[derive(Debug, Clone)]
pub struct SplitSpec {
    /// F-order profile count (8 or 10 in our tasks).
    pub order_profiles: u8,
    /// Profile distribution for F-order D_tr.
    pub skew: ProfileSkew,
    /// D_tr case count at 100%.
    pub n_tr: usize,
    /// D_sel case count.
    pub n_sel: usize,
    /// D_test case count.
    pub n_test: usize,
    /// F-ledger horizon range for D_sel/D_test, inclusive.
    pub horizon_lo: u8,
    /// F-ledger horizon range for D_sel/D_test, inclusive.
    pub horizon_hi: u8,
}

/// Train/selection/test splits. `D_test` never participates in training
/// (paper §II.2); all three are disjoint; construction is seeded.
#[derive(Debug, Clone)]
pub struct Splits {
    /// Training cases.
    pub d_tr: Vec<TaskCase>,
    /// Selection (gate) cases.
    pub d_sel: Vec<TaskCase>,
    /// Held-out test cases.
    pub d_test: Vec<TaskCase>,
}

// ---------------------------------------------------------------------------
// Family content tables (published doubles)
// ---------------------------------------------------------------------------

/// Required tool orders per F-order profile. Distinct permutations of
/// the four tools; index = profile.
pub const ORDER_REQUIRED: [[&str; 4]; 10] = [
    ["fetch", "parse", "validate", "emit"],
    ["fetch", "validate", "parse", "emit"],
    ["parse", "fetch", "validate", "emit"],
    ["fetch", "parse", "emit", "validate"],
    ["validate", "fetch", "parse", "emit"],
    ["parse", "validate", "fetch", "emit"],
    ["fetch", "emit", "parse", "validate"],
    ["validate", "parse", "fetch", "emit"],
    ["emit", "fetch", "validate", "parse"],
    ["parse", "emit", "validate", "fetch"],
];

/// The default wrong order emitted when the skill says nothing usable.
/// Starts with `emit`; no required order does, so it never matches.
const ORDER_DEFAULT: [&str; 4] = ["emit", "fetch", "parse", "validate"];

/// F-ledger canonical rules (exact lines; all three needed).
pub const LEDGER_CANONICAL: [&str; 3] = [
    "LEDGER: maintain visited set across steps",
    "LEDGER: maintain frontier queue across steps",
    "LEDGER: emit visited and frontier at each step",
];

/// F-ledger narrow variants (exact lines; short-horizon only).
pub const LEDGER_NARROW: [&str; 3] = [
    "LEDGER: maintain visited set for short horizons",
    "LEDGER: maintain frontier queue for short horizons",
    "LEDGER: emit ledgers only when horizon is short",
];

/// F-ledger twist rules (exact lines; proposed only under slow guidance).
pub const LEDGER_TWIST_RULES: [&str; 2] = [
    "LEDGER: on empty frontier, re-emit previous frontier",
    "LEDGER: on revisit, keep first-visit order",
];

/// The exact F-bind line.
pub const BIND_EXACT: &str = "BIND: quote exact span verbatim";
/// The flawed F-bind line in s_0.
pub const BIND_FLAWED: &str = "BIND: paraphrase the answer briefly";

/// F-bind source table: (country, city). Deterministic fixture.
pub const BIND_SOURCES: [(&str, &str); 12] = [
    ("Peru", "Lima"),
    ("Japan", "Tokyo"),
    ("Egypt", "Cairo"),
    ("Brazil", "Brasilia"),
    ("Canada", "Ottawa"),
    ("Kenya", "Nairobi"),
    ("Norway", "Oslo"),
    ("Chile", "Santiago"),
    ("India", "New Delhi"),
    ("Morocco", "Rabat"),
    ("Vietnam", "Hanoi"),
    ("Poland", "Warsaw"),
];

/// Horizon at or below which narrow ledger rules stay active.
pub const NARROW_HORIZON_MAX: u8 = 6;

/// The initial skill per family: plausible but flawed.
pub fn initial_skill(family: Family) -> SkillDoc {
    match family {
        Family::FOrder => {
            // Handles profile 0 only (unprofiled line); the flaw the
            // loop must fix is the missing ORDER[p] lines.
            SkillDoc::experiment("ORDER: fetch parse validate emit\n")
        }
        Family::FBind => {
            // The flaw is the paraphrase directive itself.
            SkillDoc::experiment("BIND: paraphrase the answer briefly\n")
        }
        Family::FLedger => {
            // No ledger rules at all; the fix needs ≥3 directives.
            SkillDoc::experiment("# ledger skill: no rules yet\n")
        }
    }
}

/// Whether a skill line is a canonical (fully general) rule for a family.
/// Used by the slow update (KEEP attribution) and retention measurement.
pub fn is_canonical_line(family: Family, line: &str) -> bool {
    match family {
        Family::FOrder => {
            if let Some(rest) = line.strip_prefix("ORDER[")
                && let Some((num, tools)) = rest.split_once("]: ")
                && let Ok(p) = num.parse::<usize>()
            {
                return p < ORDER_REQUIRED.len()
                    && tools.split_whitespace().collect::<Vec<_>>() == ORDER_REQUIRED[p];
            }
            false
        }
        Family::FBind => line == BIND_EXACT,
        Family::FLedger => LEDGER_CANONICAL.contains(&line) || LEDGER_TWIST_RULES.contains(&line),
    }
}

// ---------------------------------------------------------------------------
// Case generation
// ---------------------------------------------------------------------------

/// Generate `count` cases for `family`, deterministic in `seed`.
/// `sel_horizons` selects the horizon pool: true → short (D_sel-like),
/// false → full range (D_test-like).
pub fn gen_cases(
    family: Family,
    count: usize,
    seed: u64,
    spec: &SplitSpec,
    sel_horizons: bool,
) -> Vec<TaskCase> {
    let mut rng = XorShift::new(seed ^ 0x5EED_0001);
    let mut cases = Vec::with_capacity(count);
    for i in 0..count {
        let case = match family {
            Family::FOrder => {
                let profile = match spec.skew {
                    ProfileSkew::Uniform => rng.below(spec.order_profiles as usize) as u8,
                    ProfileSkew::Zipf => zipf_profile(&mut rng, spec.order_profiles),
                };
                TaskCase {
                    id: i,
                    family,
                    profile,
                    horizon: 0,
                    twist: Twist::None,
                    source_idx: 0,
                }
            }
            Family::FBind => TaskCase {
                id: i,
                family,
                profile: 0,
                horizon: 0,
                twist: Twist::None,
                source_idx: rng.below(BIND_SOURCES.len()),
            },
            Family::FLedger => {
                let (lo, hi) = if sel_horizons {
                    (2u8, 4u8)
                } else {
                    (spec.horizon_lo, spec.horizon_hi)
                };
                let horizon = lo + rng.below((hi - lo + 1) as usize) as u8;
                let twist = match rng.below(4) {
                    0 => Twist::EmptyFrontier,
                    1 => Twist::Revisit,
                    _ => Twist::None,
                };
                TaskCase {
                    id: i,
                    family,
                    profile: 0,
                    horizon,
                    twist,
                    source_idx: 0,
                }
            }
        };
        cases.push(case);
    }
    cases
}

/// Zipf-ish profile draw: weight(p) ∝ 1/(p+1).
fn zipf_profile(rng: &mut XorShift, profiles: u8) -> u8 {
    let total: f64 = (1..=profiles).map(|p| 1.0 / f64::from(p)).sum();
    let mut roll = rng.next_f64() * total;
    for p in 0..profiles {
        roll -= 1.0 / f64::from(p + 1);
        if roll <= 0.0 {
            return p;
        }
    }
    profiles - 1
}

/// Build the three disjoint splits. `d_tr_frac` subsamples the shuffled
/// D_tr pool: at 1.0 the whole pool is kept; below 1.0 the subsample is
/// profile-stratified (round-robin across profiles in profile order, so
/// every fraction preserves the profile mix instead of taking a raw
/// prefix). D_sel/D_test are generated independently of the fraction, so
/// they are identical across fractions for a seed.
pub fn make_splits(family: Family, seed: u64, d_tr_frac: f64, spec: &SplitSpec) -> Splits {
    assert!(
        (0.0..=1.0).contains(&d_tr_frac) && d_tr_frac > 0.0,
        "d_tr_frac must be in (0, 1]"
    );
    let mut rng = XorShift::new(seed ^ 0x5EED_0002);
    let mut d_tr = gen_cases(family, spec.n_tr, seed ^ 0x5EED_0010, spec, false);
    rng.shuffle(&mut d_tr);
    let keep = ((d_tr.len() as f64) * d_tr_frac).ceil() as usize;
    let mut d_tr: Vec<TaskCase> = if d_tr_frac < 1.0 {
        stratified_take(d_tr, keep.max(1))
    } else {
        d_tr
    };
    let mut d_sel = gen_cases(family, spec.n_sel, seed ^ 0x5EED_0020, spec, true);
    let mut d_test = gen_cases(family, spec.n_test, seed ^ 0x5EED_0030, spec, false);
    // Globally unique ids across the three splits (the learner matches
    // trajectories to cases by id; collisions would misattribute).
    reid(&mut d_tr, 0);
    reid(&mut d_sel, spec.n_tr);
    reid(&mut d_test, spec.n_tr + spec.n_sel);
    Splits {
        d_tr,
        d_sel,
        d_test,
    }
}

/// Assign `base + index` ids, deterministically.
fn reid(cases: &mut [TaskCase], base: usize) {
    for (i, case) in cases.iter_mut().enumerate() {
        case.id = base + i;
    }
}

/// Deterministic profile-stratified subsample: round-robin across
/// profiles (profile order), preserving within-profile shuffled order.
/// Returns exactly `keep` cases (or the whole pool if smaller).
fn stratified_take(pool: Vec<TaskCase>, keep: usize) -> Vec<TaskCase> {
    let mut by_profile: Vec<(u8, Vec<TaskCase>)> = Vec::new();
    for case in pool {
        match by_profile.iter_mut().find(|(p, _)| *p == case.profile) {
            Some((_, v)) => v.push(case),
            None => by_profile.push((case.profile, vec![case])),
        }
    }
    by_profile.sort_by_key(|(p, _)| *p);
    let mut out = Vec::with_capacity(keep.min(by_profile.iter().map(|(_, v)| v.len()).sum()));
    let mut idx = 0usize;
    while out.len() < keep {
        let mut advanced = false;
        for (_, group) in by_profile.iter() {
            if let Some(case) = group.get(idx) {
                out.push(case.clone());
                advanced = true;
                if out.len() == keep {
                    break;
                }
            }
        }
        if !advanced {
            break;
        }
        idx += 1;
    }
    out
}

/// Build mixed-family splits (task-102): each split draws from both
/// families in proportion.
pub fn make_mixed_splits(seed: u64, d_tr_frac: f64, spec: &SplitSpec) -> Splits {
    let mut a = make_splits(Family::FOrder, seed, d_tr_frac, spec);
    let mut b = make_splits(Family::FBind, seed ^ 0xB1AB, d_tr_frac, spec);
    let mut rng = XorShift::new(seed ^ 0x5EED_0040);
    let mut d_tr = std::mem::take(&mut a.d_tr);
    d_tr.append(&mut b.d_tr);
    rng.shuffle(&mut d_tr);
    let mut d_sel = std::mem::take(&mut a.d_sel);
    d_sel.append(&mut b.d_sel);
    rng.shuffle(&mut d_sel);
    let mut d_test = std::mem::take(&mut a.d_test);
    d_test.append(&mut b.d_test);
    rng.shuffle(&mut d_test);
    // Re-unique the ids across the two families (both halves were
    // numbered from their own bases).
    reid(&mut d_tr, 0);
    reid(&mut d_sel, d_tr.len());
    reid(&mut d_test, d_tr.len() + d_sel.len());
    Splits {
        d_tr,
        d_sel,
        d_test,
    }
}

// ---------------------------------------------------------------------------
// ScriptedTarget (MOCK)
// ---------------------------------------------------------------------------

/// Deterministic scripted target (MOCK): behavior is a published
/// function of skill-doc content, declared in this module's docs.
#[derive(Debug, Clone, Copy)]
pub struct ScriptedTarget {
    /// Which family this target serves.
    pub family: Family,
}

impl ScriptedTarget {
    /// Build for one family.
    pub fn new(family: Family) -> Self {
        ScriptedTarget { family }
    }

    /// The family this target was built for (informational; rollout
    /// dispatches on the case's family so mixed splits work).
    pub fn family(&self) -> Family {
        self.family
    }

    /// Fraction of `cases` with reward 1 under `skill`.
    pub fn score(&self, skill: &SkillDoc, cases: &[TaskCase]) -> f64 {
        if cases.is_empty() {
            return 0.0;
        }
        let hits: usize = cases
            .iter()
            .map(|c| self.rollout(skill, c).reward as usize)
            .sum();
        hits as f64 / cases.len() as f64
    }
}

impl Target for ScriptedTarget {
    fn rollout(&self, skill: &SkillDoc, case: &TaskCase) -> Trajectory {
        match case.family {
            Family::FOrder => rollout_order(skill, case),
            Family::FBind => rollout_bind(skill, case),
            Family::FLedger => rollout_ledger(skill, case),
        }
    }
}

/// Dispatch target for mixed-family splits (task-102).
#[derive(Debug, Clone, Copy)]
pub struct MixedTarget;

impl Target for MixedTarget {
    fn rollout(&self, skill: &SkillDoc, case: &TaskCase) -> Trajectory {
        ScriptedTarget::new(case.family).rollout(skill, case)
    }
}

impl MixedTarget {
    /// Fraction correct over mixed cases.
    pub fn score(&self, skill: &SkillDoc, cases: &[TaskCase]) -> f64 {
        ScriptedTarget::new(Family::FOrder).score(skill, cases)
    }
}

fn rollout_order(skill: &SkillDoc, case: &TaskCase) -> Trajectory {
    let text = skill.render_for_target();
    let profile_key = format!("ORDER[{}]:", case.profile);
    // First profiled match wins (document order = priority).
    let mut emitted: Option<Vec<String>> = None;
    for line in text.lines() {
        if line.starts_with(&profile_key) {
            emitted = Some(parse_tools(&line[profile_key.len()..]));
            break;
        }
    }
    let emitted = emitted.unwrap_or_else(|| {
        // Unprofiled fallback: the bare "ORDER:" line only (a profiled
        // "ORDER[3]:" line does not match this prefix).
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("ORDER:") {
                return parse_tools(rest);
            }
        }
        ORDER_DEFAULT.iter().map(|s| s.to_string()).collect()
    });
    let required: Vec<String> = ORDER_REQUIRED[case.profile as usize]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let reward = u8::from(emitted == required);
    Trajectory {
        case_id: case.id,
        reward,
        expected: required.join(" "),
        got: emitted.join(" "),
    }
}

/// Parse a tool list from a skill line: known tools, in order.
fn parse_tools(rest: &str) -> Vec<String> {
    const TOOLS: [&str; 4] = ["fetch", "parse", "validate", "emit"];
    rest.split_whitespace()
        .filter(|t| TOOLS.contains(t))
        .map(str::to_string)
        .collect()
}

fn rollout_bind(skill: &SkillDoc, case: &TaskCase) -> Trajectory {
    let (_, city) = BIND_SOURCES[case.source_idx % BIND_SOURCES.len()];
    let exact = skill.render_for_target().lines().any(|l| l == BIND_EXACT);
    let got = if exact {
        city.to_string()
    } else {
        format!("~{city}~")
    };
    Trajectory {
        case_id: case.id,
        reward: u8::from(got == city),
        expected: city.to_string(),
        got,
    }
}

fn rollout_ledger(skill: &SkillDoc, case: &TaskCase) -> Trajectory {
    let text = skill.render_for_target();
    let has = |line: &str| text.lines().any(|l| l == line);
    let full_slots = LEDGER_CANONICAL.iter().all(|c| has(c));
    let narrow_slots = LEDGER_NARROW
        .iter()
        .enumerate()
        .all(|(i, n)| has(n) || has(LEDGER_CANONICAL[i]));
    // Narrow rules are short-horizon approximations: they do not handle
    // twists. Canonical rules are fully general.
    let short = case.horizon <= NARROW_HORIZON_MAX;
    let capable = full_slots || (narrow_slots && short);
    let twist_ok = match case.twist {
        Twist::None => true,
        Twist::EmptyFrontier => has(LEDGER_TWIST_RULES[0]) || full_slots,
        Twist::Revisit => has(LEDGER_TWIST_RULES[1]) || full_slots,
    };
    let reward = u8::from(capable && twist_ok);
    Trajectory {
        case_id: case.id,
        reward,
        expected: format!("ledger(h={}, twist={:?})", case.horizon, case.twist),
        got: if reward == 1 {
            "correct ledger".to_string()
        } else {
            "broken ledger".to_string()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Family, MixedTarget, ProfileSkew, ScriptedTarget, SplitSpec, Target, initial_skill,
        make_splits,
    };

    fn spec() -> SplitSpec {
        SplitSpec {
            order_profiles: 10,
            skew: ProfileSkew::Uniform,
            n_tr: 40,
            n_sel: 20,
            n_test: 40,
            horizon_lo: 3,
            horizon_hi: 10,
        }
    }

    /// Validation: same seed -> identical splits; splits are disjoint with
    /// the configured sizes. (Profiles are drawn per the configured skew;
    /// task-105 stratifies explicitly at the driver level.)
    #[test]
    fn splits_deterministic_and_disjoint() {
        let (a, b) = (
            make_splits(Family::FOrder, 11, 1.0, &spec()),
            make_splits(Family::FOrder, 11, 1.0, &spec()),
        );
        assert_eq!(a.d_tr.len(), 40);
        assert_eq!(a.d_tr[0].id, b.d_tr[0].id);
        assert_eq!(a.d_tr[a.d_tr.len() - 1].id, b.d_tr[b.d_tr.len() - 1].id);
        let sel_ids: std::collections::HashSet<usize> = a.d_sel.iter().map(|c| c.id).collect();
        let test_ids: std::collections::HashSet<usize> = a.d_test.iter().map(|c| c.id).collect();
        assert_eq!(a.d_sel.len(), 20);
        assert_eq!(a.d_test.len(), 40);
        assert!(a.d_tr.iter().all(|c| !sel_ids.contains(&c.id)), "disjoint");
        assert!(
            a.d_sel.iter().all(|c| !test_ids.contains(&c.id)),
            "disjoint"
        );
    }

    /// Validation: a skill with all canonical rules scores 1.0.
    #[test]
    fn canonical_skill_perfect() {
        let target = MixedTarget;
        for family in [Family::FOrder, Family::FBind, Family::FLedger] {
            let mut skill = initial_skill(family);
            for line in canonical_rules(family) {
                if !skill.has_line(&line) {
                    skill
                        .apply(&super::super::doc::Edit {
                            op: super::super::doc::EditOp::Append { line: line.clone() },
                            rationale: "test".into(),
                            direction: "test".into(),
                        })
                        .unwrap();
                }
            }
            let splits = make_splits(family, 5, 1.0, &spec());
            assert_eq!(
                target.score(&skill, &splits.d_test),
                1.0,
                "canonical skill must be perfect for {family:?}"
            );
        }
    }

    /// Validation: s_0 never starts perfect (headroom to learn). F-order
    /// starts partial; F-bind and F-ledger start at 0 (their initial
    /// lines help nothing) — all headroom, no ceiling hit.
    #[test]
    fn initial_skill_partial() {
        let target = MixedTarget;
        for family in [Family::FOrder, Family::FBind, Family::FLedger] {
            let skill = initial_skill(family);
            let splits = make_splits(family, 5, 1.0, &spec());
            let s = target.score(&skill, &splits.d_test);
            assert!(s < 1.0, "{family:?} s_0 = {s}");
            if family == Family::FOrder {
                assert!(s > 0.0, "{family:?} s_0 = {s}");
            }
        }
    }

    /// Adversarial: wrong-order and twist-breaking skills fail their cases.
    #[test]
    fn broken_skills_fail() {
        let target = ScriptedTarget::new(Family::FOrder);
        let splits = make_splits(Family::FOrder, 5, 1.0, &spec());
        let mut skill = initial_skill(Family::FOrder);
        skill
            .apply(&super::super::doc::Edit {
                op: super::super::doc::EditOp::Append {
                    line: "ORDER[0]: cut mix bake".into(),
                },
                rationale: "wrong".into(),
                direction: "wrong".into(),
            })
            .unwrap();
        let case = splits.d_test.iter().find(|c| c.profile == 0).unwrap();
        assert_eq!(target.rollout(&skill, case).reward, 0);
        // F-ledger twist: narrow rule alone cannot solve twist cases.
        let ledger = ScriptedTarget::new(Family::FLedger);
        let mut narrow = initial_skill(Family::FLedger);
        narrow
            .apply(&super::super::doc::Edit {
                op: super::super::doc::EditOp::Append {
                    line: super::LEDGER_NARROW[0].to_string(),
                },
                rationale: "narrow".into(),
                direction: "narrow".into(),
            })
            .unwrap();
        let lsplits = make_splits(Family::FLedger, 5, 1.0, &spec());
        let twist = lsplits
            .d_test
            .iter()
            .find(|c| c.twist != super::Twist::None)
            .unwrap();
        assert_eq!(ledger.rollout(&narrow, twist).reward, 0);
    }

    /// Adversarial: D_tr fraction 0.1 keeps ceil(40 * 0.1) = 4 cases and
    /// stays disjoint from D_sel.
    #[test]
    fn small_fraction_prefix() {
        let splits = make_splits(Family::FOrder, 5, 0.1, &spec());
        assert_eq!(splits.d_tr.len(), 4);
        let sel_ids: std::collections::HashSet<usize> = splits.d_sel.iter().map(|c| c.id).collect();
        assert!(splits.d_tr.iter().all(|c| !sel_ids.contains(&c.id)));
    }

    fn canonical_rules(family: Family) -> Vec<String> {
        match family {
            Family::FOrder => super::ORDER_REQUIRED
                .iter()
                .enumerate()
                .map(|(p, tools)| format!("ORDER[{p}]: {}", tools.join(" ")))
                .collect(),
            Family::FBind => vec![super::BIND_EXACT.to_string()],
            Family::FLedger => super::LEDGER_CANONICAL
                .iter()
                .map(|s| s.to_string())
                .chain(super::LEDGER_TWIST_RULES.iter().map(|s| s.to_string()))
                .collect(),
        }
    }
}
