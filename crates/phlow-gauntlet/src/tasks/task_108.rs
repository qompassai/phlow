//! Task 108 — cross-harness transfer (nvim-lua, validation).
//!
//! Evolve an F-bind skill in the Rust harness (scripted double, MOCK —
//! the verdict backend for this wave), freeze its exact text, and pass
//! it through diver's REAL skill load path under headless Neovim:
//! `require('ai.mcp.skills')` → `setup({skills_dir})` → `scan()` →
//! `get(name)` → `tool_def(skill).procedure`
//! (`lua/gauntlet/task_108.lua`). Both harnesses must score at or above
//! the no-skill baseline on the same F-bind fixtures, and the variant
//! rank order must be preserved across harnesses. The honest Neovim
//! verdict comes from primo; the diver checkout is read-only (never
//! modified, committed, or pushed by this task).

use crate::skillopt::doc::{SLOW_UPDATE_END, SLOW_UPDATE_START, SkillDoc};
use crate::skillopt::driver::{TaskDriverError, base_config, default_spec};
use crate::skillopt::learner::{Learner, LearnerConfig};
use crate::skillopt::optimizer::ScriptedOptimizer;
use crate::skillopt::target::{Family, MixedTarget, initial_skill, make_splits};
use crate::{Ctx, TaskKind, TaskOutcome};
use std::path::{Path, PathBuf};

/// Task id.
pub const ID: &str = "task-108";
/// Task name.
pub const NAME: &str = "cross-harness transfer";
/// Task kind.
pub const KIND: TaskKind = TaskKind::NvimLua;
/// Diver SHA probed (read-only; `git -C ~/workspace/repos/diver
/// rev-parse HEAD`).
pub const DIVER_SHA: &str = "c84352cc850d507df477706b9166b6541ebe9e1c";
/// Lua driver scenarios: 2 validation + 2 adversarial across the tests.
pub const SCENARIOS: [&str; 2] = ["assemble", "invalid-name"];
/// Seeds evolved (design: ≥3).
const SEEDS: [u64; 3] = [101, 102, 103];
/// Epochs for the F-bind arm.
const EPOCHS: usize = 3;
/// Fixed probe fixtures for the cross-harness scoring.
const PROBE_SEED: u64 = 0x108C_2055_0001;

/// One frozen variant: exact bytes the Rust harness hands to diver.
#[derive(Debug, Clone)]
pub struct Variant {
    /// Skill name (valid per diver's `^[a-z0-9][a-z0-9%-]*$`).
    pub name: String,
    /// Evolution seed.
    pub seed: u64,
    /// Exact frozen skill-text bytes (body + slow-update markers +
    /// protected section).
    pub body: Vec<u8>,
}

/// Freeze the exact optimizer-visible skill text: body, the
/// slow-update markers, and the protected section. This is the text
/// the optimizer read and the text that would be exported, so it is
/// the honest cross-harness artifact. The KEEP lines are target-inert
/// for scoring (they match neither family's predicates) but they are
/// part of the skill and must round-trip byte-exact.
fn freeze_text(body: &str, protected: &str) -> String {
    format!("{body}\n{SLOW_UPDATE_START}\n{protected}\n{SLOW_UPDATE_END}\n")
}

/// Evolve F-bind to convergence and freeze three variants per seed —
/// s_0, the frozen final skill, and the final skill plus a neutral
/// distractor line — as `<variants>/<name>/SKILL.md` (diver's real
/// skill-directory layout: the scanner finds `SKILL.md` files, not
/// flat `.md` files). Also writes `names.txt` (one variant name per
/// line) so the lua driver knows which names to `get()`.
/// Deterministic in the seeds: re-running reproduces the bytes.
pub fn prepare_variants(work_dir: &Path) -> Result<Vec<Variant>, TaskDriverError> {
    let mut cfg: LearnerConfig = base_config();
    cfg.families = vec![Family::FBind];
    cfg.mixed = false;
    cfg.epochs = EPOCHS;
    cfg.seeds = SEEDS.to_vec();
    let logs = Learner
        .run_arm(&cfg, &ScriptedOptimizer::new())
        .map_err(|e| TaskDriverError::Arm {
            arm: "evolve".to_string(),
            detail: e.to_string(),
        })?;
    let variants_dir = work_dir.join("variants");
    std::fs::create_dir_all(&variants_dir).map_err(|e| TaskDriverError::Fixture {
        what: "variants dir".to_string(),
        detail: e.to_string(),
    })?;
    let s0_text = freeze_text(initial_skill(Family::FBind).body(), "");
    let mut variants = Vec::new();
    for log in &logs {
        let seed = log.seed as u64;
        let fin_text = freeze_text(&log.final_body, &log.final_protected);
        let bodies = [
            (format!("t108-s{seed}-s0"), s0_text.clone()),
            (format!("t108-s{seed}-fin"), fin_text.clone()),
            (
                format!("t108-s{seed}-find"),
                format!("{fin_text}NOTE: exactness is optional\n"),
            ),
        ];
        for (name, body) in bodies {
            let text = format!(
                "---\nname: {name}\ndescription: task-108 cross-harness variant\n---\n{body}"
            );
            let skill_dir = variants_dir.join(&name);
            std::fs::create_dir_all(&skill_dir).map_err(|e| TaskDriverError::Fixture {
                what: format!("variant dir {name}"),
                detail: e.to_string(),
            })?;
            std::fs::write(skill_dir.join("SKILL.md"), &text).map_err(|e| {
                TaskDriverError::Fixture {
                    what: format!("variant {name}"),
                    detail: e.to_string(),
                }
            })?;
            variants.push(Variant {
                name,
                seed,
                body: body.into_bytes(),
            });
        }
    }
    let names = variants
        .iter()
        .map(|v| v.name.clone())
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(variants_dir.join("names.txt"), names).map_err(|e| {
        TaskDriverError::Fixture {
            what: "names.txt".to_string(),
            detail: e.to_string(),
        }
    })?;
    Ok(variants)
}

/// Write a single skill dir with an invalid skill name (adversarial
/// probe of the assembly path's rejection behavior).
pub fn prepare_invalid_variant(work_dir: &Path) -> Result<PathBuf, TaskDriverError> {
    let variants_dir = work_dir.join("variants-bad");
    let skill_dir = variants_dir.join("bad-name-probe");
    std::fs::create_dir_all(&skill_dir).map_err(|e| TaskDriverError::Fixture {
        what: "variants-bad dir".to_string(),
        detail: e.to_string(),
    })?;
    let text = "---\nname: Bad_Name\ndescription: invalid name probe\n---\nBIND: quote exact span verbatim\n";
    let path = skill_dir.join("SKILL.md");
    std::fs::write(&path, text).map_err(|e| TaskDriverError::Fixture {
        what: "bad variant".to_string(),
        detail: e.to_string(),
    })?;
    Ok(variants_dir)
}

/// Score skill bytes on the fixed F-bind probe fixtures.
pub fn score_bytes(bytes: &[u8]) -> Result<f64, TaskDriverError> {
    let text = String::from_utf8(bytes.to_vec()).map_err(|e| TaskDriverError::Fixture {
        what: "variant bytes".to_string(),
        detail: format!("not UTF-8: {e}"),
    })?;
    let doc = SkillDoc::experiment(&text);
    let spec = default_spec();
    let splits = make_splits(Family::FBind, PROBE_SEED, 1.0, &spec);
    Ok(MixedTarget.score(&doc, &splits.d_test))
}

/// No-skill baseline on the probe fixtures.
pub fn baseline_score() -> f64 {
    let doc = SkillDoc::experiment("");
    let spec = default_spec();
    let splits = make_splits(Family::FBind, PROBE_SEED, 1.0, &spec);
    MixedTarget.score(&doc, &splits.d_test)
}

/// Read the lua driver's assembled bytes for one variant.
pub fn read_assembled(out_dir: &Path, name: &str) -> Result<Vec<u8>, TaskDriverError> {
    std::fs::read(out_dir.join(format!("{name}.bin"))).map_err(|e| TaskDriverError::Fixture {
        what: format!("assembled bytes for {name}"),
        detail: e.to_string(),
    })
}

/// Run one lua scenario against already-prepared variant dirs: spawn
/// headless nvim with `lua/gauntlet/task_108.lua` and return the
/// driver's verdict. The diver checkout is never modified.
pub fn run_lua_scenario(
    ctx: &Ctx,
    scenario: &str,
    variants_dir: &Path,
    out_dir: &Path,
) -> TaskOutcome {
    if let Err(e) = std::fs::create_dir_all(out_dir) {
        return TaskOutcome::Fail {
            where_: "prepare".to_string(),
            how: format!("cannot create out dir: {e}"),
            evidence: vec![],
        };
    }
    let variants_str = variants_dir.to_string_lossy().into_owned();
    let out_str = out_dir.to_string_lossy().into_owned();
    crate::run_nvim_lua_driver_with_env(
        ctx,
        "task_108.lua",
        ID,
        &[
            ("GAUNTLET_SCENARIO", scenario),
            ("GAUNTLET_VARIANTS_DIR", variants_str.as_str()),
            ("GAUNTLET_OUT_DIR", out_str.as_str()),
        ],
    )
}

/// Run one lua scenario: prepare variants, spawn headless nvim with
/// `lua/gauntlet/task_108.lua`, return the driver's verdict.
pub fn run_scenario(ctx: &Ctx, scenario: &str) -> TaskOutcome {
    let work = ctx.work_dir.join(ID);
    let (variants_dir, out_dir) = match scenario {
        "assemble" => {
            if let Err(e) = prepare_variants(&work).map(|_| ()) {
                return TaskOutcome::Fail {
                    where_: "prepare".to_string(),
                    how: e.to_string(),
                    evidence: vec![],
                };
            }
            (work.join("variants"), work.join("out"))
        }
        "invalid-name" => match prepare_invalid_variant(&work) {
            Ok(dir) => (dir, work.join("out-bad")),
            Err(e) => {
                return TaskOutcome::Fail {
                    where_: "prepare".to_string(),
                    how: e.to_string(),
                    evidence: vec![],
                };
            }
        },
        _ => {
            return TaskOutcome::Fail {
                where_: "scenario".to_string(),
                how: format!("task-108: unknown scenario '{scenario}'"),
                evidence: vec![],
            };
        }
    };
    if let Err(e) = std::fs::create_dir_all(&out_dir) {
        return TaskOutcome::Fail {
            where_: "prepare".to_string(),
            how: format!("cannot create out dir: {e}"),
            evidence: vec![],
        };
    }
    run_lua_scenario(ctx, scenario, &variants_dir, &out_dir)
}

/// Task-level entry: assemble through diver, then score both harnesses.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    let work = ctx.work_dir.join(ID);
    let variants = match prepare_variants(&work) {
        Ok(v) => v,
        Err(e) => {
            return TaskOutcome::Fail {
                where_: "prepare".to_string(),
                how: e.to_string(),
                evidence: vec![],
            };
        }
    };
    let outcome = run_lua_scenario(ctx, "assemble", &work.join("variants"), &work.join("out"));
    let mut evidence = vec![format!(
        "diver sha {DIVER_SHA} (read-only); backend: scripted-mock evolve, diver ai.mcp.skills assemble"
    )];
    match outcome {
        TaskOutcome::Fail {
            where_,
            how,
            evidence: ev,
        } => {
            evidence.extend(ev);
            return TaskOutcome::Fail {
                where_,
                how,
                evidence,
            };
        }
        TaskOutcome::Pass { evidence: ev } => evidence.extend(ev),
    }
    let out_dir = work.join("out");
    let baseline = baseline_score();
    let mut ok = true;
    for v in &variants {
        let rust_score = match score_bytes(&v.body) {
            Ok(s) => s,
            Err(e) => {
                return TaskOutcome::Fail {
                    where_: "score-rust".to_string(),
                    how: e.to_string(),
                    evidence,
                };
            }
        };
        let assembled = match read_assembled(&out_dir, &v.name) {
            Ok(b) => b,
            Err(e) => {
                return TaskOutcome::Fail {
                    where_: "read-assembled".to_string(),
                    how: e.to_string(),
                    evidence,
                };
            }
        };
        let byte_exact = assembled == v.body;
        let lua_score = match score_bytes(&assembled) {
            Ok(s) => s,
            Err(e) => {
                return TaskOutcome::Fail {
                    where_: "score-lua".to_string(),
                    how: e.to_string(),
                    evidence,
                };
            }
        };
        evidence.push(format!(
            "{}: byte_exact={byte_exact} rust={rust_score:.2} lua={lua_score:.2} baseline={baseline:.2}",
            v.name
        ));
        if !byte_exact || rust_score < baseline || lua_score < baseline {
            ok = false;
        }
    }
    if ok {
        TaskOutcome::Pass { evidence }
    } else {
        TaskOutcome::Fail {
            where_: "cross-harness".to_string(),
            how: "byte mismatch or harness below baseline (see evidence)".to_string(),
            evidence,
        }
    }
}
