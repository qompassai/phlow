//! Manifest parsing and validation for the experiment's TOML contracts.
//!
//! Four manifests ship with this crate (`manifests/*.toml`): the suite
//! list, the language tiers, the budget defaults, and the promotion
//! thresholds. Task manifests (`evals/*/*.toml`) follow the plan's
//! test-case contract. All parsing goes through [`toml::Value`] with
//! explicit validation control flow — no derive macros for validation —
//! mirroring `phlow-config`. Every error names the file and the key.
//!
//! Manifests are read-only contracts: the candidate under evaluation must
//! never be able to write them (see the `evals/*/README.md` split
//! contracts).

use crate::error::{ExperimentError, manifest_invalid, manifest_unreadable};
use std::path::Path;

// ---------------------------------------------------------------------------
// Bounds (all with units)
// ---------------------------------------------------------------------------

/// Maximum suites in `suites.toml`.
pub const SUITES_MAX: usize = 16;
/// Maximum tiers in `languages.toml`.
pub const TIERS_MAX: usize = 8;
/// Maximum languages in one tier.
pub const LANGUAGES_PER_TIER_MAX: usize = 16;
/// Maximum checks in one task manifest.
pub const TASK_CHECKS_MAX: usize = 32;
/// Maximum required files in one acceptance section.
pub const REQUIRED_FILES_MAX: usize = 64;
/// Maximum forbidden paths in one acceptance section.
pub const FORBIDDEN_PATHS_MAX: usize = 64;
/// Maximum characters in a manifest name field.
pub const MANIFEST_NAME_CHARS_MAX: usize = 128;
/// Maximum characters in a manifest description field.
pub const MANIFEST_DESC_CHARS_MAX: usize = 1_024;
/// The only schema version this crate accepts.
pub const MANIFEST_SCHEMA_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Risk class (shared by task manifests and improvement proposals)
// ---------------------------------------------------------------------------

/// The risk class of a task or proposal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskClass {
    /// Ordinary development risk.
    Normal,
    /// Elevated: touches trust-adjacent surfaces.
    Elevated,
    /// Critical: safety cases, secrets-adjacent, or promotion-blocking.
    Critical,
}

impl RiskClass {
    /// Parses `normal`, `elevated`, or `critical`.
    pub fn parse(value: &str) -> Result<Self, ExperimentError> {
        match value {
            "normal" => Ok(Self::Normal),
            "elevated" => Ok(Self::Elevated),
            "critical" => Ok(Self::Critical),
            _ => Err(ExperimentError::ManifestInvalid {
                file: "task manifest",
                key: "risk".to_string(),
                reason: format!("unknown risk class {value:?}; want normal|elevated|critical"),
            }),
        }
    }

    /// The stable machine-readable name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Elevated => "elevated",
            Self::Critical => "critical",
        }
    }
}

// ---------------------------------------------------------------------------
// Suite manifest
// ---------------------------------------------------------------------------

/// One test suite entry in `suites.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuiteDef {
    /// Suite id, e.g. `"adversarial"`.
    pub id: String,
    /// Human-readable description.
    pub description: String,
    /// Whether the suite is required for every experiment.
    pub required: bool,
}

/// The parsed `suites.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuiteManifest {
    /// Must be [`MANIFEST_SCHEMA_VERSION`].
    pub schema_version: u32,
    /// The suites, in file order.
    pub suites: Vec<SuiteDef>,
}

// ---------------------------------------------------------------------------
// Language manifest
// ---------------------------------------------------------------------------

/// One language entry in a tier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageDef {
    /// Language name, e.g. `"rust"`.
    pub name: String,
    /// The minimum app task class for this language.
    pub min_app: String,
    /// Required checks, e.g. `["fmt", "clippy", "tests"]`.
    pub required_checks: Vec<String>,
    /// What adversarial cases focus on for this language.
    pub adversarial_focus: String,
}

/// One tier (`A`–`E`) in `languages.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TierDef {
    /// Tier letter, `A` through `E`.
    pub tier: char,
    /// Human-readable description.
    pub description: String,
    /// The languages in this tier.
    pub languages: Vec<LanguageDef>,
}

/// The parsed `languages.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageManifest {
    /// Must be [`MANIFEST_SCHEMA_VERSION`].
    pub schema_version: u32,
    /// The tiers, in file order.
    pub tiers: Vec<TierDef>,
}

// ---------------------------------------------------------------------------
// Budget manifest
// ---------------------------------------------------------------------------

/// The budget defaults from `budgets.toml`.
///
/// These are the plan's *initial experimental limits*, not universal
/// tuning: calibrate from measured runs, one budget at a time, recording
/// the reason for each change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetDefaults {
    /// Maximum concurrent workers.
    pub workers_max: u64,
    /// Scheduler queue capacity, in nodes.
    pub queue_capacity: u64,
    /// Maximum children admitted per task.
    pub children_per_task_max: u64,
    /// Maximum delegation depth.
    pub depth_max: u64,
    /// Per-task deadline, in milliseconds.
    pub task_deadline_ms: u64,
    /// Maximum tool calls per task.
    pub tool_calls_max: u64,
    /// Maximum output per task, in bytes.
    pub output_bytes_max: u64,
    /// Maximum resident memory per task, in bytes.
    pub memory_bytes_max: u64,
    /// Maximum model turns per task.
    pub model_turns_max: u64,
    /// Maximum files a task may change.
    pub changed_files_max: u64,
    /// Maximum bytes a task may change.
    pub changed_bytes_max: u64,
}

/// The parsed `budgets.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetManifest {
    /// Must be [`MANIFEST_SCHEMA_VERSION`].
    pub schema_version: u32,
    /// The defaults.
    pub defaults: BudgetDefaults,
}

// ---------------------------------------------------------------------------
// Promotion manifest
// ---------------------------------------------------------------------------

/// The promotion thresholds from `promotion.toml`.
///
/// Proposed starting gates, not externally established standards: adjust
/// only through a reviewed policy change separate from any candidate being
/// judged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromotionThresholds {
    /// Required pass percentage on critical safety cases (100).
    pub critical_safety_pass_pct: u64,
    /// Whether a newly failing previously-solved critical task is allowed
    /// (never).
    pub allow_new_failures_on_prior_successes: bool,
    /// Whether a measurable gain on hidden holdouts is required.
    pub hidden_holdout_gain_required: bool,
    /// Maximum allowed p95 latency increase, in percent (10).
    pub p95_latency_increase_pct_max: u64,
    /// Maximum allowed resource-cost increase, in percent (20).
    pub cost_increase_pct_max: u64,
    /// Whether a larger cost increase needs explicit approval.
    pub cost_increase_requires_approval: bool,
    /// Whether a rollback rehearsal to the exact prior revision is required.
    pub rollback_rehearsal_required: bool,
}

/// The parsed `promotion.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromotionManifest {
    /// Must be [`MANIFEST_SCHEMA_VERSION`].
    pub schema_version: u32,
    /// The thresholds.
    pub thresholds: PromotionThresholds,
}

// ---------------------------------------------------------------------------
// Task manifest (the plan's test-case contract)
// ---------------------------------------------------------------------------

/// Budget section of a task manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskBudget {
    /// Wall-clock budget, in milliseconds.
    pub wall_ms: u64,
    /// Maximum model turns.
    pub model_turns: u64,
    /// Maximum tool calls.
    pub tool_calls: u64,
    /// Maximum files the task may change.
    pub changed_files: u64,
    /// Maximum bytes the task may change.
    pub changed_bytes: u64,
    /// Maximum workers for the task.
    pub workers: u64,
    /// Scheduler queue depth for the task.
    pub queue_depth: u64,
}

/// One `[[checks]]` entry: name, exact argv, required flag, kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskCheck {
    /// Check name, e.g. `"clippy"`.
    pub name: String,
    /// Exact argv the host must execute.
    pub argv: Vec<String>,
    /// Whether the check is required (required checks cannot be replaced).
    pub required: bool,
    /// Check kind, e.g. `"format"`, `"lint"`, `"test"`.
    pub kind: String,
}

/// Acceptance section of a task manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Acceptance {
    /// Files that must exist after the task.
    pub required_files: Vec<String>,
    /// Paths the task must never touch.
    pub forbidden_paths: Vec<String>,
    /// Maximum new dependencies the task may add.
    pub max_new_dependencies: u64,
}

/// The parsed task manifest: the plan's machine-readable test-case
/// contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskManifest {
    /// Must be [`MANIFEST_SCHEMA_VERSION`].
    pub schema_version: u32,
    /// Task id, e.g. `"rust-cli-parse-001"`.
    pub id: String,
    /// Language, e.g. `"rust"`.
    pub language: String,
    /// Task kind, e.g. `"cli"`, `"adversarial"`.
    pub kind: String,
    /// Risk class.
    pub risk: RiskClass,
    /// Fixture the task runs against.
    pub workspace_fixture: String,
    /// The task statement.
    pub task: String,
    /// The task's budget.
    pub budget: TaskBudget,
    /// The checks the host must run.
    pub checks: Vec<TaskCheck>,
    /// The acceptance criteria.
    pub acceptance: Acceptance,
}

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

/// Reads a manifest file into a string, mapping I/O failure to
/// [`ExperimentError::ManifestUnreadable`].
pub fn read_manifest_file(path: &Path, file: &'static str) -> Result<String, ExperimentError> {
    std::fs::read_to_string(path)
        .map_err(|error| manifest_unreadable(file, &bounded_io_reason(&error)))
}

/// Bounds an I/O error's message for inclusion in an error value.
fn bounded_io_reason(error: &std::io::Error) -> String {
    error.to_string().chars().take(256).collect()
}

/// Parses and validates `suites.toml`.
pub fn parse_suite_manifest(
    toml_text: &str,
    file: &'static str,
) -> Result<SuiteManifest, ExperimentError> {
    let table = parse_root(toml_text, file)?;
    check_schema_version(&table, file)?;
    reject_unknown(&table, file, "manifest", &["schema_version", "suite"])?;
    let suites = parse_array(&table, file, "suite", SUITES_MAX, |item, index| {
        let at = format!("suite[{index}]");
        let item = as_table(item, file, &at)?;
        reject_unknown(item, file, &at, &["id", "description", "required"])?;
        Ok(SuiteDef {
            id: get_name(item, file, &at, "id")?,
            description: get_desc(item, file, &at, "description")?,
            required: get_bool(item, file, &at, "required")?,
        })
    })?;
    if suites.is_empty() {
        return Err(manifest_invalid(file, "suite", "at least one suite is required"));
    }
    Ok(SuiteManifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        suites,
    })
}

/// Parses and validates `languages.toml`.
pub fn parse_language_manifest(
    toml_text: &str,
    file: &'static str,
) -> Result<LanguageManifest, ExperimentError> {
    let table = parse_root(toml_text, file)?;
    check_schema_version(&table, file)?;
    reject_unknown(&table, file, "manifest", &["schema_version", "tier"])?;
    let tiers = parse_array(&table, file, "tier", TIERS_MAX, |item, index| {
        let at = format!("tier[{index}]");
        let item = as_table(item, file, &at)?;
        reject_unknown(item, file, &at, &["tier", "description", "language"])?;
        let tier_text = get_name(item, file, &at, "tier")?;
        let tier = tier_text.chars().next().ok_or_else(|| {
            manifest_invalid(file, &format!("{at}.tier"), "tier must be one letter A-E")
        })?;
        if tier_text.len() != 1 || !('A'..='E').contains(&tier) {
            return Err(manifest_invalid(
                file,
                &format!("{at}.tier"),
                "tier must be one letter A-E",
            ));
        }
        let languages = parse_array(item, file, "language", LANGUAGES_PER_TIER_MAX, |lang, li| {
            let lat = format!("{at}.language[{li}]");
            let lang = as_table(lang, file, &lat)?;
            reject_unknown(
                lang,
                file,
                &lat,
                &[
                    "name",
                    "min_app",
                    "required_checks",
                    "adversarial_focus",
                ],
            )?;
            Ok(LanguageDef {
                name: get_name(lang, file, &lat, "name")?,
                min_app: get_desc(lang, file, &lat, "min_app")?,
                required_checks: get_string_array(lang, file, &lat, "required_checks", 32)?,
                adversarial_focus: get_desc(lang, file, &lat, "adversarial_focus")?,
            })
        })?;
        if languages.is_empty() {
            return Err(manifest_invalid(file, &at, "each tier needs at least one language"));
        }
        Ok(TierDef {
            tier,
            description: get_desc(item, file, &at, "description")?,
            languages,
        })
    })?;
    if tiers.is_empty() {
        return Err(manifest_invalid(file, "tier", "at least one tier is required"));
    }
    Ok(LanguageManifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        tiers,
    })
}

/// Parses and validates `budgets.toml`.
pub fn parse_budget_manifest(
    toml_text: &str,
    file: &'static str,
) -> Result<BudgetManifest, ExperimentError> {
    let table = parse_root(toml_text, file)?;
    check_schema_version(&table, file)?;
    reject_unknown(&table, file, "manifest", &["schema_version", "defaults"])?;
    let defaults = as_table_value(
        table.get("defaults"),
        file,
        "defaults",
        "the [defaults] table is required",
    )?;
    reject_unknown(
        defaults,
        file,
        "defaults",
        &[
            "workers_max",
            "queue_capacity",
            "children_per_task_max",
            "depth_max",
            "task_deadline_ms",
            "tool_calls_max",
            "output_bytes_max",
            "memory_bytes_max",
            "model_turns_max",
            "changed_files_max",
            "changed_bytes_max",
        ],
    )?;
    let get = |key: &str| get_positive_u64(defaults, file, "defaults", key);
    Ok(BudgetManifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        defaults: BudgetDefaults {
            workers_max: get("workers_max")?,
            queue_capacity: get("queue_capacity")?,
            children_per_task_max: get("children_per_task_max")?,
            depth_max: get("depth_max")?,
            task_deadline_ms: get("task_deadline_ms")?,
            tool_calls_max: get("tool_calls_max")?,
            output_bytes_max: get("output_bytes_max")?,
            memory_bytes_max: get("memory_bytes_max")?,
            model_turns_max: get("model_turns_max")?,
            changed_files_max: get("changed_files_max")?,
            changed_bytes_max: get("changed_bytes_max")?,
        },
    })
}

/// Parses and validates `promotion.toml`.
pub fn parse_promotion_manifest(
    toml_text: &str,
    file: &'static str,
) -> Result<PromotionManifest, ExperimentError> {
    let table = parse_root(toml_text, file)?;
    check_schema_version(&table, file)?;
    reject_unknown(&table, file, "manifest", &["schema_version", "thresholds"])?;
    let thresholds = as_table_value(
        table.get("thresholds"),
        file,
        "thresholds",
        "the [thresholds] table is required",
    )?;
    reject_unknown(
        thresholds,
        file,
        "thresholds",
        &[
            "critical_safety_pass_pct",
            "allow_new_failures_on_prior_successes",
            "hidden_holdout_gain_required",
            "p95_latency_increase_pct_max",
            "cost_increase_pct_max",
            "cost_increase_requires_approval",
            "rollback_rehearsal_required",
        ],
    )?;
    let critical_safety_pass_pct =
        get_u64(thresholds, file, "thresholds", "critical_safety_pass_pct")?;
    let p95_latency_increase_pct_max = get_u64(
        thresholds,
        file,
        "thresholds",
        "p95_latency_increase_pct_max",
    )?;
    let cost_increase_pct_max =
        get_u64(thresholds, file, "thresholds", "cost_increase_pct_max")?;
    Ok(PromotionManifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        thresholds: PromotionThresholds {
            critical_safety_pass_pct,
            allow_new_failures_on_prior_successes: get_bool(
                thresholds,
                file,
                "thresholds",
                "allow_new_failures_on_prior_successes",
            )?,
            hidden_holdout_gain_required: get_bool(
                thresholds,
                file,
                "thresholds",
                "hidden_holdout_gain_required",
            )?,
            p95_latency_increase_pct_max,
            cost_increase_pct_max,
            cost_increase_requires_approval: get_bool(
                thresholds,
                file,
                "thresholds",
                "cost_increase_requires_approval",
            )?,
            rollback_rehearsal_required: get_bool(
                thresholds,
                file,
                "thresholds",
                "rollback_rehearsal_required",
            )?,
        },
    })
}

/// Parses and validates one task manifest (`evals/*/*.toml`).
pub fn parse_task_manifest(
    toml_text: &str,
    file: &'static str,
) -> Result<TaskManifest, ExperimentError> {
    let table = parse_root(toml_text, file)?;
    check_schema_version(&table, file)?;
    reject_unknown(
        &table,
        file,
        "manifest",
        &[
            "schema_version",
            "id",
            "language",
            "kind",
            "risk",
            "workspace_fixture",
            "task",
            "budget",
            "checks",
            "acceptance",
        ],
    )?;
    let budget = as_table_value(
        table.get("budget"),
        file,
        "budget",
        "the [budget] table is required",
    )?;
    reject_unknown(
        budget,
        file,
        "budget",
        &[
            "wall_ms",
            "model_turns",
            "tool_calls",
            "changed_files",
            "changed_bytes",
            "workers",
            "queue_depth",
        ],
    )?;
    let bget = |key: &str| get_positive_u64(budget, file, "budget", key);
    // Zero-change budgets are legal (analysis-only tasks); the other
    // budget fields must be positive.
    let zget = |key: &str| get_u64(budget, file, "budget", key);
    let checks = parse_array(&table, file, "checks", TASK_CHECKS_MAX, |item, index| {
        let at = format!("checks[{index}]");
        let item = as_table(item, file, &at)?;
        reject_unknown(item, file, &at, &["name", "argv", "required", "kind"])?;
        Ok(TaskCheck {
            name: get_name(item, file, &at, "name")?,
            argv: get_string_array(item, file, &at, "argv", 64)?,
            required: get_bool(item, file, &at, "required")?,
            kind: get_name(item, file, &at, "kind")?,
        })
    })?;
    if checks.is_empty() {
        return Err(manifest_invalid(file, "checks", "at least one check is required"));
    }
    let acceptance = as_table_value(
        table.get("acceptance"),
        file,
        "acceptance",
        "the [acceptance] table is required",
    )?;
    reject_unknown(
        acceptance,
        file,
        "acceptance",
        &["required_files", "forbidden_paths", "max_new_dependencies"],
    )?;
    let risk_text = get_name(&table, file, "manifest", "risk")?;
    let risk = RiskClass::parse(&risk_text).map_err(|_| {
        manifest_invalid(
            file,
            "risk",
            "unknown risk class; want normal|elevated|critical",
        )
    })?;
    Ok(TaskManifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        id: get_name(&table, file, "manifest", "id")?,
        language: get_name(&table, file, "manifest", "language")?,
        kind: get_name(&table, file, "manifest", "kind")?,
        risk,
        workspace_fixture: get_desc(&table, file, "manifest", "workspace_fixture")?,
        task: get_desc(&table, file, "manifest", "task")?,
        budget: TaskBudget {
            wall_ms: bget("wall_ms")?,
            model_turns: bget("model_turns")?,
            tool_calls: bget("tool_calls")?,
            changed_files: zget("changed_files")?,
            changed_bytes: zget("changed_bytes")?,
            workers: bget("workers")?,
            queue_depth: bget("queue_depth")?,
        },
        checks,
        acceptance: Acceptance {
            required_files: get_string_array(
                acceptance,
                file,
                "acceptance",
                "required_files",
                REQUIRED_FILES_MAX,
            )?,
            forbidden_paths: get_string_array(
                acceptance,
                file,
                "acceptance",
                "forbidden_paths",
                FORBIDDEN_PATHS_MAX,
            )?,
            max_new_dependencies: get_u64(
                acceptance,
                file,
                "acceptance",
                "max_new_dependencies",
            )?,
        },
    })
}

// ---------------------------------------------------------------------------
// Validation primitives (explicit control flow, no derive)
// ---------------------------------------------------------------------------

/// The TOML table type used throughout validation.
type Table = toml::map::Map<String, toml::Value>;

/// Parses the document root; malformed TOML fails with the file named.
fn parse_root(toml_text: &str, file: &'static str) -> Result<Table, ExperimentError> {
    toml_text
        .parse::<toml::Value>()
        .map_err(|error| {
            let reason: String = error.to_string().chars().take(256).collect();
            manifest_invalid(file, "document", &reason)
        })
        .and_then(|value| match value {
            toml::Value::Table(table) => Ok(table),
            _ => Err(manifest_invalid(
                file,
                "document",
                "the root must be a TOML table",
            )),
        })
}

/// Requires `schema_version = 1`.
fn check_schema_version(table: &Table, file: &'static str) -> Result<(), ExperimentError> {
    let version = get_u64(table, file, "manifest", "schema_version")?;
    if version != u64::from(MANIFEST_SCHEMA_VERSION) {
        return Err(manifest_invalid(
            file,
            "schema_version",
            "only schema_version = 1 is supported",
        ));
    }
    Ok(())
}

/// Rejects unknown keys in `table`; the error names the file and section.
fn reject_unknown(
    table: &Table,
    file: &'static str,
    at: &str,
    allowed: &[&str],
) -> Result<(), ExperimentError> {
    for key in table.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(manifest_invalid(
                file,
                &format!("{at}.{key}"),
                "unknown key",
            ));
        }
    }
    Ok(())
}

/// Requires `key` to hold a table.
fn as_table_value(
    value: Option<&toml::Value>,
    file: &'static str,
    key: &str,
    missing: &str,
) -> Result<Table, ExperimentError> {
    match value {
        Some(toml::Value::Table(table)) => Ok(table.clone()),
        Some(_) => Err(manifest_invalid(file, key, "must be a table")),
        None => Err(manifest_invalid(file, key, missing)),
    }
}

/// Requires `array` to hold a table at `index`.
fn as_table(
    value: &toml::Value,
    file: &'static str,
    at: &str,
) -> Result<Table, ExperimentError> {
    match value {
        toml::Value::Table(table) => Ok(table.clone()),
        _ => Err(manifest_invalid(file, at, "must be a table")),
    }
}

/// Parses a `[[array]]` with an item bound, applying `parse_item` to each.
fn parse_array<T>(
    table: &Table,
    file: &'static str,
    key: &str,
    max: usize,
    parse_item: impl Fn(&toml::Value, usize) -> Result<T, ExperimentError>,
) -> Result<Vec<T>, ExperimentError> {
    let values = match table.get(key) {
        Some(toml::Value::Array(values)) => values,
        Some(_) => return Err(manifest_invalid(file, key, "must be an array")),
        None => return Err(manifest_invalid(file, key, "is required")),
    };
    if values.len() > max {
        return Err(manifest_invalid(
            file,
            key,
            &format!("at most {max} items allowed"),
        ));
    }
    let mut out = Vec::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        out.push(parse_item(value, index)?);
    }
    Ok(out)
}

/// Requires a non-empty string within [`MANIFEST_NAME_CHARS_MAX`].
fn get_name(
    table: &Table,
    file: &'static str,
    at: &str,
    key: &str,
) -> Result<String, ExperimentError> {
    get_bounded_string(table, file, at, key, MANIFEST_NAME_CHARS_MAX)
}

/// Requires a non-empty string within [`MANIFEST_DESC_CHARS_MAX`].
fn get_desc(
    table: &Table,
    file: &'static str,
    at: &str,
    key: &str,
) -> Result<String, ExperimentError> {
    get_bounded_string(table, file, at, key, MANIFEST_DESC_CHARS_MAX)
}

fn get_bounded_string(
    table: &Table,
    file: &'static str,
    at: &str,
    key: &str,
    max: usize,
) -> Result<String, ExperimentError> {
    match table.get(key) {
        Some(toml::Value::String(value)) => {
            if value.is_empty() {
                return Err(manifest_invalid(file, &format!("{at}.{key}"), "must not be empty"));
            }
            if value.len() > max {
                return Err(manifest_invalid(
                    file,
                    &format!("{at}.{key}"),
                    &format!("at most {max} characters"),
                ));
            }
            Ok(value.clone())
        }
        Some(_) => Err(manifest_invalid(file, &format!("{at}.{key}"), "must be a string")),
        None => Err(manifest_invalid(file, &format!("{at}.{key}"), "is required")),
    }
}

/// Requires a boolean.
fn get_bool(
    table: &Table,
    file: &'static str,
    at: &str,
    key: &str,
) -> Result<bool, ExperimentError> {
    match table.get(key) {
        Some(toml::Value::Boolean(value)) => Ok(*value),
        Some(_) => Err(manifest_invalid(file, &format!("{at}.{key}"), "must be a boolean")),
        None => Err(manifest_invalid(file, &format!("{at}.{key}"), "is required")),
    }
}

/// Requires an integer >= 0.
fn get_u64(
    table: &Table,
    file: &'static str,
    at: &str,
    key: &str,
) -> Result<u64, ExperimentError> {
    match table.get(key) {
        Some(toml::Value::Integer(value)) => {
            u64::try_from(*value).map_err(|_| {
                manifest_invalid(file, &format!("{at}.{key}"), "must not be negative")
            })
        }
        Some(_) => Err(manifest_invalid(file, &format!("{at}.{key}"), "must be an integer")),
        None => Err(manifest_invalid(file, &format!("{at}.{key}"), "is required")),
    }
}

/// Requires a positive integer.
fn get_positive_u64(
    table: &Table,
    file: &'static str,
    at: &str,
    key: &str,
) -> Result<u64, ExperimentError> {
    let value = get_u64(table, file, at, key)?;
    if value == 0 {
        return Err(manifest_invalid(
            file,
            &format!("{at}.{key}"),
            "must be positive",
        ));
    }
    Ok(value)
}

/// Requires an array of non-empty strings within bounds.
fn get_string_array(
    table: &Table,
    file: &'static str,
    at: &str,
    key: &str,
    max: usize,
) -> Result<Vec<String>, ExperimentError> {
    let values = match table.get(key) {
        Some(toml::Value::Array(values)) => values,
        Some(_) => {
            return Err(manifest_invalid(
                file,
                &format!("{at}.{key}"),
                "must be an array of strings",
            ));
        }
        None => return Err(manifest_invalid(file, &format!("{at}.{key}"), "is required")),
    };
    if values.len() > max {
        return Err(manifest_invalid(
            file,
            &format!("{at}.{key}"),
            &format!("at most {max} items allowed"),
        ));
    }
    let mut out = Vec::with_capacity(values.len());
    for value in values {
        match value {
            toml::Value::String(text) => {
                if text.is_empty() {
                    return Err(manifest_invalid(
                        file,
                        &format!("{at}.{key}"),
                        "entries must not be empty",
                    ));
                }
                if text.len() > MANIFEST_DESC_CHARS_MAX {
                    return Err(manifest_invalid(
                        file,
                        &format!("{at}.{key}"),
                        "entry too long",
                    ));
                }
                out.push(text.clone());
            }
            _ => {
                return Err(manifest_invalid(
                    file,
                    &format!("{at}.{key}"),
                    "must be an array of strings",
                ));
            }
        }
    }
    Ok(out)
}
