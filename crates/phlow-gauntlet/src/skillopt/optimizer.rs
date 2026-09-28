//! Optimizers: the reflection step of the SkillOpt loop.
//!
//! [`Optimizer::propose`] takes the current skill, success/failure
//! minibatches, the rejected-edit buffer, and optimizer-private meta,
//! and returns a *ranked* list of candidate [`Edit`]s. The learner
//! truncates the ranking to L_t (the paper's "merge → global rank →
//! truncate", §II.4).
//!
//! Backends:
//!
//! - [`ScriptedOptimizer`] — MOCK, labeled as such. Fixed competence
//!   (p≈0.6 correct / 0.4 distractor, fallible *by design*), re-sampling
//!   candidate generator. The ablation arms share the identical seed
//!   stream, so only the ablated component varies.
//! - [`ModelOptimizer`] — the REAL backend: primo's local Ollama
//!   (`qwen3:8b`) over HTTP, `think: false`, structured SkillOpt-style
//!   reflection prompts. Loopback only; unreachable endpoint is a typed
//!   error, never a fabricated result.

use super::doc::{Edit, EditOp, PER_EDIT_CHARS_MAX, SkillDoc};
use super::rng::XorShift;
use super::target::{
    BIND_EXACT, BIND_FLAWED, Family, LEDGER_CANONICAL, LEDGER_NARROW, LEDGER_TWIST_RULES,
    ORDER_REQUIRED,
};
use std::collections::HashMap;
use std::fmt;
use std::time::Duration;

/// Max proposals returned per `propose` call (the ranked list the
/// learner truncates to L_t).
pub const PROPOSALS_MAX: usize = 6;
/// Scripted rank-1 competence: P(top proposal is the good fix).
pub const SCRIPTED_COMPETENCE: f64 = 0.6;
/// Reflection prompt cap in chars (prompt is bounded text, not bulk).
pub const PROMPT_CHARS_MAX: usize = 24_000;
/// Ollama response cap in bytes (mirrors phlow-runtime's 2 MiB posture).
pub const RESPONSE_BYTES_MAX: usize = 2 * 1024 * 1024;
/// `/api/generate` per-call timeout.
pub const MODEL_TIMEOUT_SECS: u64 = 120;
/// Adequate `num_predict` (proven by smoke test: thinking disabled, the
/// model answers in a handful of tokens; 512 leaves headroom).
pub const MODEL_NUM_PREDICT: u32 = 512;

/// One trajectory summarized for the reflection prompt.
#[derive(Debug, Clone)]
pub struct TrajSummary {
    /// Case id.
    pub case_id: usize,
    /// F-order profile (for good-fix targeting).
    pub profile: u8,
    /// What the verifier expected.
    pub expected: String,
    /// What the target emitted.
    pub got: String,
}

/// Everything an optimizer may consult in one reflection step.
#[derive(Debug, Clone)]
pub struct ReflectCtx {
    /// `render_for_optimizer()`: body + protected + meta.
    pub skill_text: String,
    /// Parsed `KEEP: ...` lines from the protected section.
    pub keep_lines: Vec<String>,
    /// Lines the current epoch already accepted (the double's recency
    /// prior: it does not propose rewrites of these within the epoch;
    /// cross-epoch memory is the slow update's job — the paper's claim).
    pub epoch_accepted_lines: Vec<String>,
    /// Success minibatch (counts only; the paper reflects on failures).
    pub n_succ: usize,
    /// Failure minibatch (what the optimizer diagnoses).
    pub fail: Vec<TrajSummary>,
    /// Rejected directions (buffer contents) — empty unless the buffer
    /// mode is Full (consulting, not just recording).
    pub rejected: Vec<String>,
    /// Optimizer-private meta text — empty unless meta is on.
    pub meta_text: String,
    /// Parsed meta category stats — empty unless meta is on.
    pub meta_cats: HashMap<String, CatStats>,
    /// Textual learning rate for this step (`usize::MAX` = unbounded).
    pub l_t: usize,
    /// Step index (for ledgers).
    pub step: usize,
    /// Families under optimization (drives the reflection prompt).
    pub families: Vec<Family>,
}

/// Per-category outcome stats: the meta update's "momentum for the
/// optimizer itself" (paper §II.6). Deltas are D_sel fractions.
#[derive(Debug, Clone, Default)]
pub struct CatStats {
    /// Samples recorded.
    pub n: u64,
    /// Mean D_sel delta.
    pub mean: f64,
    /// Variance of D_sel delta.
    pub var: f64,
}

impl CatStats {
    /// Fold one observation into the running stats.
    pub fn observe(&mut self, delta: f64) {
        let n = self.n as f64;
        let new_mean = (self.mean * n + delta) / (n + 1.0);
        // Welford-lite: keep it simple and exact for small n.
        self.var = (self.var * n + (delta - new_mean) * (delta - self.mean)) / (n + 1.0);
        self.mean = new_mean;
        self.n += 1;
    }
}

/// Optimizer failures: the harness's fault or the model's, never the
/// task's. Unreachable model → typed error; tests skip, never fake.
#[derive(Debug, Clone)]
pub enum OptimizerError {
    /// The model endpoint is unreachable or unusable.
    Unavailable {
        /// What was tried.
        detail: String,
    },
    /// The model answered but nothing parseable came back.
    NoUsableEdits,
    /// Misconfiguration (e.g. non-loopback URL).
    Config {
        /// What was wrong.
        detail: String,
    },
}

impl fmt::Display for OptimizerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable { detail } => write!(f, "optimizer unavailable: {detail}"),
            Self::NoUsableEdits => write!(f, "model returned no parseable edits"),
            Self::Config { detail } => write!(f, "optimizer misconfigured: {detail}"),
        }
    }
}

impl std::error::Error for OptimizerError {}

/// The optimizer seam: propose ranked edits from reflection context.
pub trait Optimizer {
    /// Propose up to [`PROPOSALS_MAX`] ranked edits. Rank = list order;
    /// the learner truncates to L_t. `skill` is the live document (for
    /// exact line-presence checks); `ctx` carries the reflection view.
    fn propose(
        &self,
        skill: &SkillDoc,
        ctx: &ReflectCtx,
        rng: &mut XorShift,
    ) -> Result<Vec<Edit>, OptimizerError>;
    /// Backend label for ledgers.
    fn name(&self) -> &'static str;
    /// How many candidates the last `propose` call suppressed because
    /// their directions sat in the rejected buffer. Mock-only
    /// diagnostic (default 0); used for the task-103 hit rate.
    fn last_suppressed(&self) -> usize {
        0
    }
}

// ---------------------------------------------------------------------------
// ScriptedOptimizer (MOCK)
// ---------------------------------------------------------------------------

/// A proposed template: the edit plus whether it is the "good" fix or a
/// distractor. Fallible by design: rank-1 is good with probability
/// [`SCRIPTED_COMPETENCE`].
#[derive(Debug, Clone)]
struct Template {
    edit: Edit,
    good: bool,
    /// Base sampling weight among distractors.
    weight: f64,
}

/// Fixed-competence scripted optimizer (MOCK). The candidate generator
/// re-samples from a fixed template pool every step, so without the
/// rejected-edit buffer it re-proposes dead directions — the load-bearing
/// property task-103 measures.
#[derive(Debug, Clone)]
pub struct ScriptedOptimizer {
    /// Suppressions in the last `propose` call (task-103 hit rate).
    last_suppressed: std::cell::Cell<usize>,
}

impl ScriptedOptimizer {
    /// Build the mock.
    pub fn new() -> Self {
        ScriptedOptimizer {
            last_suppressed: std::cell::Cell::new(0),
        }
    }

    /// Applicable templates for this reflection: good fixes for failing
    /// profiles/slots plus distractors, minus buffer-suppressed
    /// directions, minus KEEP/recency-protected rewrites. Returns the
    /// templates and the number suppressed by the rejected buffer.
    fn templates(&self, ctx: &ReflectCtx, skill: &SkillDoc) -> (Vec<Template>, usize) {
        let mut out = Vec::new();
        let mut suppressed_count = 0usize;
        let mut suppressed = |direction: &str| {
            let hit = ctx.rejected.iter().any(|r| r == direction);
            if hit {
                suppressed_count += 1;
            }
            hit
        };
        let kept = |line: &str| {
            ctx.keep_lines
                .iter()
                .any(|k| k.strip_prefix("KEEP: ") == Some(line))
        };
        let recent = |line: &str| ctx.epoch_accepted_lines.iter().any(|l| l == line);

        // F-order templates: good = correct ORDER[p] for failing p.
        let mut failed_profiles: Vec<u8> = ctx.fail.iter().map(|s| s.profile).collect();
        failed_profiles.sort_unstable();
        failed_profiles.dedup();
        for &p in &failed_profiles {
            let pi = p as usize;
            if pi >= ORDER_REQUIRED.len() {
                continue;
            }
            let want = format!("ORDER[{p}]: {}", ORDER_REQUIRED[pi].join(" "));
            if !skill.has_line(&want) {
                let direction = format!("order:add:{p}");
                if !suppressed(&direction) {
                    out.push(Template {
                        edit: Edit {
                            op: EditOp::Append { line: want },
                            rationale: format!("profile {p} fails; add its order rule"),
                            direction,
                        },
                        good: true,
                        weight: 1.0,
                    });
                }
            }
        }
        // F-order distractors.
        for p in 0..ORDER_REQUIRED.len() as u8 {
            let correct = format!("ORDER[{p}]: {}", ORDER_REQUIRED[p as usize].join(" "));
            // Wrong-order append.
            let mut wrong = ORDER_REQUIRED[p as usize].to_vec();
            wrong.rotate_left(1);
            let wrong_line = format!("ORDER[{p}]: {}", wrong.join(" "));
            if !skill.has_line(&wrong_line) {
                let direction = format!("order:wrong:{p}");
                if !suppressed(&direction) {
                    out.push(Template {
                        edit: Edit {
                            op: EditOp::Append { line: wrong_line },
                            rationale: format!("try an alternative order for {p}"),
                            direction,
                        },
                        good: false,
                        weight: 0.30,
                    });
                }
            }
            // Delete / replace of a correct rule: harmful, and the
            // mechanism behind the unbounded arm's churn (task-101) and
            // cross-epoch forgetting (task-104).
            if skill.has_line(&correct) && !kept(&correct) && !recent(&correct) {
                let direction = format!("order:del:{p}");
                if !suppressed(&direction) {
                    out.push(Template {
                        edit: Edit {
                            op: EditOp::Delete {
                                line: correct.clone(),
                            },
                            rationale: format!("order rule {p} looks redundant"),
                            direction,
                        },
                        good: false,
                        weight: 0.10,
                    });
                }
                let direction = format!("order:repl:{p}");
                if !suppressed(&direction) {
                    out.push(Template {
                        edit: Edit {
                            op: EditOp::Replace {
                                old: correct.clone(),
                                new: format!("ORDER[{p}]: {}", wrong.join(" ")),
                            },
                            rationale: format!("simplify the order rule for {p}"),
                            direction,
                        },
                        good: false,
                        weight: 0.25,
                    });
                }
            }
        }

        // F-bind templates.
        let bind_failing = ctx.fail.iter().any(|s| s.got.starts_with('~'));
        if bind_failing && skill.has_line(BIND_FLAWED) && !skill.has_line(BIND_EXACT) {
            let direction = "bind:fix".to_string();
            if !suppressed(&direction) {
                out.push(Template {
                    edit: Edit {
                        op: EditOp::Replace {
                            old: BIND_FLAWED.to_string(),
                            new: BIND_EXACT.to_string(),
                        },
                        rationale: "paraphrase breaks exact-span binding; quote verbatim"
                            .to_string(),
                        direction,
                    },
                    good: true,
                    weight: 1.0,
                });
            }
        }
        for (line, direction, weight) in [
            ("BIND: quote approximate span", "bind:approx", 0.30),
            ("NOTE: exactness is optional", "bind:note", 0.20),
        ] {
            if !skill.has_line(line) && !suppressed(direction) {
                out.push(Template {
                    edit: Edit {
                        op: EditOp::Append {
                            line: line.to_string(),
                        },
                        rationale: "alternative binding directive".to_string(),
                        direction: direction.to_string(),
                    },
                    good: false,
                    weight,
                });
            }
        }
        if skill.has_line(BIND_EXACT) && !kept(BIND_EXACT) && !recent(BIND_EXACT) {
            let direction = "bind:del";
            if !suppressed(direction) {
                out.push(Template {
                    edit: Edit {
                        op: EditOp::Delete {
                            line: BIND_EXACT.to_string(),
                        },
                        rationale: "verbatim quoting looks redundant".to_string(),
                        direction: direction.to_string(),
                    },
                    good: false,
                    weight: 0.10,
                });
            }
        }

        // F-ledger templates: good = canonical adds for empty slots.
        let text_has = |line: &str| skill.has_line(line);
        let slot_filled = |i: usize| text_has(LEDGER_CANONICAL[i]) || text_has(LEDGER_NARROW[i]);
        let ledger_failing = ctx.fail.iter().any(|s| s.got == "broken ledger");
        if ledger_failing {
            for i in 0..3 {
                if !slot_filled(i) {
                    let direction = format!("ledger:add:{i}");
                    if !suppressed(&direction) {
                        out.push(Template {
                            edit: Edit {
                                op: EditOp::Append {
                                    line: LEDGER_CANONICAL[i].to_string(),
                                },
                                rationale: format!("ledger slot {i} missing"),
                                direction,
                            },
                            good: true,
                            weight: 1.0,
                        });
                    }
                    // Narrow variant: distractor, short-horizon only.
                    let direction = format!("ledger:narrow:{i}");
                    if !suppressed(&direction) {
                        out.push(Template {
                            edit: Edit {
                                op: EditOp::Append {
                                    line: LEDGER_NARROW[i].to_string(),
                                },
                                rationale: format!("short-horizon ledger for slot {i}"),
                                direction,
                            },
                            good: false,
                            weight: 0.30,
                        });
                    }
                }
            }
            // Twist rules: proposed ONLY under slow-update guidance
            // (the double cannot invent them from minibatches alone).
            let guided = ctx
                .skill_text
                .lines()
                .any(|l| l.starts_with("GUIDE:") && l.contains("twist"));
            if guided {
                for (j, rule) in LEDGER_TWIST_RULES.iter().enumerate() {
                    if !text_has(rule) {
                        let direction = format!("ledger:twist:{j}");
                        if !suppressed(&direction) {
                            out.push(Template {
                                edit: Edit {
                                    op: EditOp::Append {
                                        line: rule.to_string(),
                                    },
                                    rationale: "guided twist handling".to_string(),
                                    direction,
                                },
                                good: true,
                                weight: 1.0,
                            });
                        }
                    }
                }
            }
            // Replace canonical with narrow: the forgetting mechanism.
            for i in 0..3 {
                if text_has(LEDGER_CANONICAL[i])
                    && !kept(LEDGER_CANONICAL[i])
                    && !recent(LEDGER_CANONICAL[i])
                {
                    let direction = format!("ledger:repl:{i}");
                    if !suppressed(&direction) {
                        out.push(Template {
                            edit: Edit {
                                op: EditOp::Replace {
                                    old: LEDGER_CANONICAL[i].to_string(),
                                    new: LEDGER_NARROW[i].to_string(),
                                },
                                rationale: format!("short-horizon variant suffices for slot {i}"),
                                direction,
                            },
                            good: false,
                            weight: 0.25,
                        });
                    }
                }
            }
        }

        // Noop distractor (zero-gain edits for the tie-accepts arm).
        if !suppressed("noop") {
            out.push(Template {
                edit: Edit {
                    op: EditOp::Append {
                        line: format!("# reviewed step {}", ctx.step),
                    },
                    rationale: "mark reviewed".to_string(),
                    direction: "noop".to_string(),
                },
                good: false,
                weight: 0.20,
            });
        }
        (out, suppressed_count)
    }
}

impl Default for ScriptedOptimizer {
    fn default() -> Self {
        Self::new()
    }
}

impl Optimizer for ScriptedOptimizer {
    fn propose(
        &self,
        skill: &SkillDoc,
        ctx: &ReflectCtx,
        rng: &mut XorShift,
    ) -> Result<Vec<Edit>, OptimizerError> {
        let (templates, suppressed) = self.templates(ctx, skill);
        self.last_suppressed.set(suppressed);
        if templates.is_empty() {
            return Ok(Vec::new());
        }
        let mut ranked: Vec<Edit> = Vec::with_capacity(PROPOSALS_MAX);
        let mut remaining: Vec<Template> = templates;
        // Rank 1: good with probability SCRIPTED_COMPETENCE. The
        // fallback draws from distractors only, so the documented
        // competence is exact (a good template must not leak in through
        // the fallback branch).
        let good_idx: Vec<usize> = remaining
            .iter()
            .enumerate()
            .filter(|(_, t)| t.good)
            .map(|(i, _)| i)
            .collect();
        let first = if !good_idx.is_empty() && rng.next_f64() < SCRIPTED_COMPETENCE {
            good_idx[rng.below(good_idx.len())]
        } else {
            let bad_idx: Vec<usize> = remaining
                .iter()
                .enumerate()
                .filter(|(_, t)| !t.good)
                .map(|(i, _)| i)
                .collect();
            if bad_idx.is_empty() {
                good_idx[rng.below(good_idx.len())]
            } else {
                // Map the draw over the distractor subset back to
                // `remaining` indices.
                let sub: Vec<Template> = bad_idx.iter().map(|&i| remaining[i].clone()).collect();
                bad_idx[weighted_draw(&sub, &ctx.meta_cats, rng)]
            }
        };
        ranked.push(remaining.remove(first).edit);
        // Ranks 2..=: sample remaining without replacement.
        while ranked.len() < PROPOSALS_MAX && !remaining.is_empty() {
            let idx = weighted_draw(&remaining, &ctx.meta_cats, rng);
            ranked.push(remaining.remove(idx).edit);
        }
        Ok(ranked)
    }

    fn name(&self) -> &'static str {
        "scripted"
    }

    fn last_suppressed(&self) -> usize {
        self.last_suppressed.get()
    }
}

/// Weighted distractor draw, modulated by meta category stats
/// ("momentum for the optimizer itself", paper §II.6):
/// weight = base × max(0.05, 1 + mean − var), deltas in D_sel fraction.
fn weighted_draw(
    templates: &[Template],
    meta_cats: &HashMap<String, CatStats>,
    rng: &mut XorShift,
) -> usize {
    let mut total = 0.0;
    let weights: Vec<f64> = templates
        .iter()
        .map(|t| {
            let w = if t.good {
                t.weight
            } else {
                let cat = t.edit.category();
                let adj = meta_cats
                    .get(cat)
                    .map_or(1.0, |s| (1.0 + s.mean - s.var).max(0.05));
                t.weight * adj
            };
            total += w;
            w
        })
        .collect();
    if total <= 0.0 {
        return rng.below(templates.len());
    }
    let mut roll = rng.next_f64() * total;
    for (i, w) in weights.iter().enumerate() {
        roll -= w;
        if roll <= 0.0 {
            return i;
        }
    }
    templates.len() - 1
}

// ---------------------------------------------------------------------------
// ModelOptimizer (REAL)
// ---------------------------------------------------------------------------

/// The real optimizer backend: primo's local Ollama over HTTP
/// (`/api/generate`, `think: false`, `num_predict` adequate).
/// Loopback only — a non-loopback URL is a configuration error.
#[derive(Debug)]
pub struct ModelOptimizer {
    base_url: String,
    model: String,
    client: reqwest::blocking::Client,
}

impl ModelOptimizer {
    /// Build for `base_url` (must be loopback) and `model`.
    pub fn new(base_url: &str, model: &str) -> Result<Self, OptimizerError> {
        let host = base_url
            .trim_start_matches("http://")
            .trim_start_matches("https://")
            .split(['/', ':'])
            .next()
            .unwrap_or("");
        if !matches!(host, "127.0.0.1" | "::1" | "localhost") {
            return Err(OptimizerError::Config {
                detail: format!("model URL must be loopback, got host {host:?}"),
            });
        }
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| OptimizerError::Config {
                detail: format!("HTTP client failed: {e}"),
            })?;
        Ok(ModelOptimizer {
            base_url: base_url.trim_end_matches('/').to_string(),
            model: model.to_string(),
            client,
        })
    }

    /// Read a response body enforcing the 2 MiB cap *while* reading
    /// (chunk-by-chunk, the same posture as phlow-runtime's
    /// ReqwestTransport), then parse JSON.
    fn read_capped_json(
        response: reqwest::blocking::Response,
    ) -> Result<serde_json::Value, OptimizerError> {
        use std::io::Read as _;
        if !response.status().is_success() {
            return Err(OptimizerError::Unavailable {
                detail: format!("HTTP {}", response.status()),
            });
        }
        let mut response = response;
        let mut body = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            let read = response
                .read(&mut chunk)
                .map_err(|e| OptimizerError::Unavailable {
                    detail: format!("read failed: {e}"),
                })?;
            if read == 0 {
                break;
            }
            if body.len() + read > RESPONSE_BYTES_MAX {
                return Err(OptimizerError::Unavailable {
                    detail: format!("response exceeds {RESPONSE_BYTES_MAX} bytes"),
                });
            }
            body.extend_from_slice(&chunk[..read]);
        }
        serde_json::from_slice(&body).map_err(|e| OptimizerError::Unavailable {
            detail: format!("bad JSON: {e}"),
        })
    }

    /// True when the server answers `/api/tags` and serves `model`.
    /// Any failure — refused, timeout, bad JSON — is false, never an
    /// error: tests skip on false.
    pub fn is_available(&self) -> bool {
        let url = format!("{}/api/tags", self.base_url);
        let response = match self
            .client
            .get(&url)
            .timeout(Duration::from_secs(10))
            .send()
        {
            Ok(r) => r,
            Err(_) => return false,
        };
        let body: serde_json::Value = match Self::read_capped_json(response) {
            Ok(v) => v,
            Err(_) => return false,
        };
        body.get("models")
            .and_then(serde_json::Value::as_array)
            .map(|models| {
                models.iter().any(|m| {
                    m.get("name").and_then(serde_json::Value::as_str) == Some(self.model.as_str())
                })
            })
            .unwrap_or(false)
    }

    /// One `/api/generate` call: `think: false`, `stream: false`,
    /// `num_predict` adequate. Response capped at 2 MiB while reading.
    fn generate(&self, prompt: &str) -> Result<String, OptimizerError> {
        let url = format!("{}/api/generate", self.base_url);
        let payload = serde_json::json!({
            "model": self.model,
            "prompt": prompt,
            "think": false,
            "stream": false,
            "temperature": 0.2,
            "options": { "num_predict": MODEL_NUM_PREDICT },
        });
        let response = self
            .client
            .post(&url)
            .json(&payload)
            .timeout(Duration::from_secs(MODEL_TIMEOUT_SECS))
            .send()
            .map_err(|e| OptimizerError::Unavailable {
                detail: format!("generate failed: {e}"),
            })?;
        if !response.status().is_success() {
            return Err(OptimizerError::Unavailable {
                detail: format!("generate HTTP {}", response.status()),
            });
        }
        let body = Self::read_capped_json(response)?;
        body.get("response")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .ok_or(OptimizerError::Unavailable {
                detail: "missing response field".to_string(),
            })
    }

    /// Build the structured SkillOpt-style reflection prompt.
    fn reflection_prompt(&self, ctx: &ReflectCtx) -> String {
        let mut prompt = String::from(
            "You are the optimizer in a skill-learning loop. A frozen tool-calling agent is guided by the CURRENT SKILL below. Propose small text edits that fix the agent's failures.\n\nCURRENT SKILL (editable):\n---\n",
        );
        prompt.push_str(&ctx.skill_text);
        truncate_chars(&mut prompt, PROMPT_CHARS_MAX);
        prompt.push_str("\n---\n\nPROTECTED GUIDANCE (read-only; never edit):\n");
        if ctx.keep_lines.is_empty() {
            prompt.push_str("(none)\n");
        } else {
            for k in &ctx.keep_lines {
                prompt.push_str(k);
                prompt.push('\n');
            }
        }
        prompt.push_str(&format!(
            "\nFAILED ROLLOUTS ({}; agent output vs expected):\n",
            ctx.fail.len()
        ));
        for s in &ctx.fail {
            prompt.push_str(&format!(
                "- case {}: agent did [{}]; expected [{}]\n",
                s.case_id, s.got, s.expected
            ));
        }
        prompt.push_str(&format!("\nSUCCESSFUL ROLLOUTS: {}\n", ctx.n_succ));
        prompt.push_str("\nDO NOT PROPOSE THESE AGAIN (rejected earlier):\n");
        if ctx.rejected.is_empty() {
            prompt.push_str("(none)\n");
        } else {
            for r in &ctx.rejected {
                prompt.push_str("- ");
                prompt.push_str(r);
                prompt.push('\n');
            }
        }
        prompt.push_str(
            "\nEDIT FORMAT (one edit per line, EXACTLY one of these forms):\n  append: <exact line to add>\n  replace: <exact old line> -> <exact new line>\n  delete: <exact line>\nExamples:\n  append: ORDER[3]: fetch parse emit validate\n  replace: BIND: paraphrase the answer briefly -> BIND: quote exact span verbatim\nRules: use the exact line text from the skill; keep every line under 400 characters.\n",
        );
        // Arm-independent: always request the fixed maximum; the learner
        // truncates to L_t, so the bound under test never leaks into the
        // model's proposal budget (task-101 isolation).
        prompt.push_str(&format!(
            "Propose up to {PROPOSALS_MAX} edits. Reply with ONLY the edit lines, nothing else.\n"
        ));
        truncate_chars(&mut prompt, PROMPT_CHARS_MAX);
        prompt
    }
}

/// Truncate to `max` chars, char-boundary-safe (never splits UTF-8).
fn truncate_chars(s: &mut String, max: usize) {
    if s.chars().count() > max {
        let head: String = s.chars().take(max).collect();
        *s = head;
    }
}

impl Optimizer for ModelOptimizer {
    fn propose(
        &self,
        _skill: &SkillDoc,
        ctx: &ReflectCtx,
        _rng: &mut XorShift,
    ) -> Result<Vec<Edit>, OptimizerError> {
        let prompt = self.reflection_prompt(ctx);
        let text = self.generate(&prompt)?;
        let edits = parse_model_edits(&text);
        if edits.is_empty() {
            return Err(OptimizerError::NoUsableEdits);
        }
        Ok(edits.into_iter().take(PROPOSALS_MAX).collect())
    }

    fn name(&self) -> &'static str {
        "model"
    }
}

/// Parse numbered (or bare) edit lines from model text. Lenient on
/// numbering and case; strict on the op vocabulary.
pub fn parse_model_edits(text: &str) -> Vec<Edit> {
    let mut edits = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        let line = strip_numbering(line);
        let Some((op_word, payload)) = line.split_once(':') else {
            continue;
        };
        let op_word = op_word.trim().to_ascii_lowercase();
        let payload = payload.trim();
        if payload.is_empty() || payload.chars().count() > PER_EDIT_CHARS_MAX {
            continue;
        }
        let edit = match op_word.as_str() {
            "append" | "add" => Edit {
                op: EditOp::Append {
                    line: payload.to_string(),
                },
                rationale: "model-proposed".to_string(),
                direction: format!("model:{}", direction_key(payload)),
            },
            "delete" | "remove" => Edit {
                op: EditOp::Delete {
                    line: payload.to_string(),
                },
                rationale: "model-proposed".to_string(),
                direction: format!("model-del:{}", direction_key(payload)),
            },
            "replace" => {
                let Some((old, new)) = payload.split_once("->") else {
                    continue;
                };
                let (old, new) = (old.trim(), new.trim());
                if old.is_empty() || new.is_empty() {
                    continue;
                }
                Edit {
                    op: EditOp::Replace {
                        old: old.to_string(),
                        new: new.to_string(),
                    },
                    rationale: "model-proposed".to_string(),
                    direction: format!("model-repl:{}", direction_key(old)),
                }
            }
            "insert_after" | "insert" => {
                let Some((anchor, newline)) = payload.split_once("->") else {
                    continue;
                };
                let (anchor, newline) = (anchor.trim(), newline.trim());
                if anchor.is_empty() || newline.is_empty() {
                    continue;
                }
                Edit {
                    op: EditOp::InsertAfter {
                        anchor: anchor.to_string(),
                        line: newline.to_string(),
                    },
                    rationale: "model-proposed".to_string(),
                    direction: format!("model-ins:{}", direction_key(newline)),
                }
            }
            _ => continue,
        };
        edits.push(edit);
    }
    edits
}

/// Strip a leading "1." / "2)" numbering marker, if present.
fn strip_numbering(line: &str) -> &str {
    let mut rest = line;
    let digits = rest
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>();
    if !digits.is_empty() {
        rest = &rest[digits.len()..];
        rest = rest.strip_prefix(['.', ')']).unwrap_or(rest).trim_start();
    }
    rest
}

/// Short stable direction key from an edit payload (first 48 chars,
/// whitespace-collapsed).
fn direction_key(payload: &str) -> String {
    payload
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(48)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        ModelOptimizer, Optimizer, OptimizerError, PER_EDIT_CHARS_MAX, PROPOSALS_MAX, ReflectCtx,
        ScriptedOptimizer,
    };
    use crate::skillopt::doc::SkillDoc;
    use crate::skillopt::rng::XorShift;
    use crate::skillopt::target::{Family, initial_skill};
    use std::collections::HashMap;

    fn ctx_for(skill: &SkillDoc) -> ReflectCtx {
        ReflectCtx {
            skill_text: skill.render_for_optimizer(),
            keep_lines: skill.keep_lines(),
            epoch_accepted_lines: Vec::new(),
            n_succ: 0,
            fail: vec![super::TrajSummary {
                case_id: 0,
                profile: 0,
                expected: "fetch parse validate emit".into(),
                got: "emit fetch parse validate".into(),
            }],
            rejected: Vec::new(),
            meta_text: String::new(),
            meta_cats: HashMap::new(),
            l_t: 4,
            step: 0,
            families: vec![Family::FOrder],
        }
    }

    /// Validation: proposals are bounded and rank-ordered; the same seed
    /// stream is deterministic.
    #[test]
    fn scripted_bounded_and_deterministic() {
        let opt = ScriptedOptimizer::new();
        let skill = initial_skill(Family::FOrder);
        let ctx = ctx_for(&skill);
        let (mut r1, mut r2) = (XorShift::new(9), XorShift::new(9));
        let (a, b) = (
            opt.propose(&skill, &ctx, &mut r1).unwrap(),
            opt.propose(&skill, &ctx, &mut r2).unwrap(),
        );
        assert!(a.len() <= PROPOSALS_MAX && !a.is_empty());
        assert_eq!(
            a.iter().map(|e| &e.direction).collect::<Vec<_>>(),
            b.iter().map(|e| &e.direction).collect::<Vec<_>>(),
            "same seed stream must give the same ranked list"
        );
    }

    /// Validation: buffer-suppressed directions vanish and are counted.
    #[test]
    fn buffer_suppresses_and_counts() {
        let opt = ScriptedOptimizer::new();
        let skill = initial_skill(Family::FOrder);
        let mut ctx = ctx_for(&skill);
        let mut rng = XorShift::new(3);
        let plain: Vec<String> = opt
            .propose(&skill, &ctx, &mut rng)
            .unwrap()
            .into_iter()
            .map(|e| e.direction)
            .collect();
        assert!(!plain.is_empty());
        ctx.rejected = plain.clone();
        let mut rng = XorShift::new(3);
        let again = opt.propose(&skill, &ctx, &mut rng).unwrap();
        assert!(
            again.iter().all(|e| !plain.contains(&e.direction)),
            "suppressed directions must not reappear"
        );
        assert!(opt.last_suppressed() > 0, "suppressions must be counted");
    }

    /// Validation: structured model output parses; oversize edits are
    /// dropped, not truncated.
    #[test]
    fn model_output_parses_and_bounds() {
        let big = "x".repeat(PER_EDIT_CHARS_MAX + 10);
        let text = format!(
            "1. append: ORDER[3]: fetch parse emit validate\n2. replace: parse -> validate\n3. append: {big}\n4. delete: whatever"
        );
        let edits = super::parse_model_edits(&text);
        assert_eq!(
            edits.len(),
            3,
            "oversize edit must be dropped, got {edits:?}"
        );
        assert!(edits[0].direction.starts_with("model:"));
    }

    /// Validation: gibberish model output yields zero edits (never
    /// fabricated).
    #[test]
    fn model_gibberish_yields_nothing() {
        assert!(super::parse_model_edits("hmm, interesting... let me think").is_empty());
        assert!(super::parse_model_edits("").is_empty());
    }

    /// Adversarial: non-loopback model URLs are refused at construction.
    #[test]
    fn model_url_must_be_loopback() {
        assert!(ModelOptimizer::new("http://10.0.0.9:11434", "qwen3:8b").is_err());
        assert!(ModelOptimizer::new("https://ollama.example.com", "qwen3:8b").is_err());
        assert!(ModelOptimizer::new("http://127.0.0.1:11434", "qwen3:8b").is_ok());
    }

    /// Adversarial: unavailable model fails the arm loudly (the learner
    /// converts this to a hard error, never a silent zero).
    #[test]
    fn model_unavailable_is_loud() {
        let opt = ModelOptimizer::new("http://127.0.0.1:1", "qwen3:8b").unwrap();
        let skill = initial_skill(Family::FOrder);
        let ctx = ctx_for(&skill);
        let mut rng = XorShift::new(1);
        let r = opt.propose(&skill, &ctx, &mut rng);
        assert!(
            matches!(r, Err(OptimizerError::Unavailable { .. })),
            "refused port must surface Unavailable, got {r:?}"
        );
    }
}
