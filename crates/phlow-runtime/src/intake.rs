//! Task intake: the Mandate, the two-layer Eval, and resume revalidation.
//!
//! Every task enters the runtime through three layers, in this order:
//!
//! 1. **Mandate** — a deterministic, host-side gate that runs before any
//!    model sees the task. It validates the task's declared scope, bounds,
//!    and mechanical preconditions against the host's standing limits and
//!    returns a typed pass/fail with reasons. No model judgment exists
//!    inside the Mandate; the same spec against the same limits and tree
//!    state always gets the same verdict.
//! 2. **Eval** — a model assessment (clarity, fit, risk notes) that runs
//!    only after the Mandate passes. Its output is advisory to dispatch.
//!    Structurally it cannot widen what the Mandate allowed: the only
//!    bounds an [`EvalReport`] may suggest are narrower ones, and
//!    [`admit`] refuses a widening suggestion outright.
//! 3. **Revalidation** — when a session resumes prior work, the Mandate
//!    re-runs against the *current* limits and tree state, and the
//!    presented spec must be fingerprint-identical to the approved one.
//!    A stale approval never carries across an interruption.
//!
//! Layering: this gate is task-level and pre-run. It does not replace the
//! per-call permission gates that stay upstream of it — operator-approved
//! checks (`phlow-checks`) still verify work during a run, `phlow-approval`
//! still admits individual tool calls, and flowguard still gates flows.
//! Intake decides whether a task may be dispatched at all, and under what
//! bounds; the existing gates keep deciding everything after that.

use std::collections::BTreeSet;
use std::fmt;
use std::path::{Component, Path, PathBuf};

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

/// Maximum declared paths in one task scope.
pub const PATHS_MAX: usize = 64;
/// Maximum declared tools in one task scope.
pub const TOOLS_MAX: usize = 64;
/// Maximum required checks in one task spec.
pub const CHECKS_MAX: usize = 32;
/// Maximum rejection reasons collected for one Mandate verdict. Evaluation
/// stops at the cap; the verdict is already a fail, more reasons add no
/// information dispatch can act on.
pub const REASONS_MAX: usize = 32;
/// Maximum bytes in one task id.
pub const TASK_ID_BYTES_MAX: usize = 128;
/// Maximum risk notes kept from one Eval report; extras are dropped.
pub const RISK_NOTES_MAX: usize = 16;
/// Maximum bytes kept from one Eval risk note; longer notes are truncated
/// on a char boundary.
pub const RISK_NOTE_BYTES_MAX: usize = 512;
/// Maximum bytes of model text [`parse_eval_report`] will inspect.
pub const EVAL_RESPONSE_BYTES_MAX: usize = 16_384;

/// Work limits a task declares for itself. Every field is a count the
/// runtime already enforces per run; the Mandate checks the declaration
/// against the host ceiling before any model turn spends against it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskBounds {
    /// Model turns per role, per cycle.
    pub max_iterations: u32,
    /// Planner/coder/reviewer cycles.
    pub max_cycles: u32,
    /// Tool calls across the whole run.
    pub max_tool_calls: u32,
    /// Context window cap, in characters.
    pub max_context_chars: u32,
}

impl TaskBounds {
    /// True when every field is less than or equal to `ceiling`'s.
    pub fn is_within(&self, ceiling: &TaskBounds) -> bool {
        self.max_iterations <= ceiling.max_iterations
            && self.max_cycles <= ceiling.max_cycles
            && self.max_tool_calls <= ceiling.max_tool_calls
            && self.max_context_chars <= ceiling.max_context_chars
    }

    /// The first field (in declaration order) by which `self` exceeds
    /// `ceiling`, as `(field, requested, ceiling)`, or `None`.
    fn first_excess(&self, ceiling: &TaskBounds) -> Option<(&'static str, u32, u32)> {
        let fields = [
            (
                "max_iterations",
                self.max_iterations,
                ceiling.max_iterations,
            ),
            ("max_cycles", self.max_cycles, ceiling.max_cycles),
            (
                "max_tool_calls",
                self.max_tool_calls,
                ceiling.max_tool_calls,
            ),
            (
                "max_context_chars",
                self.max_context_chars,
                ceiling.max_context_chars,
            ),
        ];
        fields
            .into_iter()
            .find(|(_, requested, limit)| requested > limit)
    }

    /// The name of the first field that is zero, if any. A zero bound is
    /// a declaration error, not a narrow bound: the run could do nothing.
    fn first_zero(&self) -> Option<&'static str> {
        let fields = [
            ("max_iterations", self.max_iterations),
            ("max_cycles", self.max_cycles),
            ("max_tool_calls", self.max_tool_calls),
            ("max_context_chars", self.max_context_chars),
        ];
        fields
            .into_iter()
            .find(|(_, value)| *value == 0)
            .map(|(name, _)| name)
    }
}

/// The declared reach of a task. `paths` and `tools` are set-valued:
/// duplicates collapse and order carries no meaning. Paths are read as
/// requests, never as authorization — the Mandate proves containment
/// against the workspace root; it never trusts the declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskScope {
    /// Workspace-relative or absolute paths the task may touch.
    pub paths: Vec<PathBuf>,
    /// Tool names the task may call.
    pub tools: Vec<String>,
    /// Whether the task intends to write. Write tools in `tools` without
    /// this flag are a contradiction the Mandate rejects.
    pub write: bool,
}

/// A task as declared at intake: identity, prompt, scope, bounds, and the
/// operator checks the task mechanically requires to be configured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskSpec {
    /// Caller-chosen task identifier.
    pub id: String,
    /// The task text a model would eventually see.
    pub prompt: String,
    /// Declared reach.
    pub scope: TaskScope,
    /// Declared work limits.
    pub bounds: TaskBounds,
    /// Names of operator-configured checks this task requires.
    pub required_checks: Vec<String>,
}

impl TaskSpec {
    /// SHA-256 over the canonical form of this spec (sets sorted, paths
    /// lexically normalized). Two specs share a fingerprint exactly when
    /// they declare the same work; resume compares fingerprints so any
    /// post-approval mutation — however small — is caught.
    pub fn fingerprint(&self) -> String {
        let mut text = String::new();
        text.push_str("id:");
        text.push_str(&self.id);
        text.push_str("\nprompt:");
        text.push_str(&self.prompt);
        text.push_str("\nwrite:");
        text.push_str(if self.scope.write { "true" } else { "false" });
        for path in sorted_paths(&self.scope.paths) {
            text.push_str("\npath:");
            text.push_str(&path);
        }
        for tool in sorted_strings(&self.scope.tools) {
            text.push_str("\ntool:");
            text.push_str(&tool);
        }
        for check in sorted_strings(&self.required_checks) {
            text.push_str("\ncheck:");
            text.push_str(&check);
        }
        text.push_str(&format!(
            "\nbounds:{}:{}:{}:{}",
            self.bounds.max_iterations,
            self.bounds.max_cycles,
            self.bounds.max_tool_calls,
            self.bounds.max_context_chars
        ));
        sha256_hex(text.as_bytes())
    }
}

fn sorted_paths(paths: &[PathBuf]) -> Vec<String> {
    let mut names: Vec<String> = paths
        .iter()
        .map(|path| normalize_lexical(path).to_string_lossy().into_owned())
        .collect();
    names.sort();
    names.dedup();
    names
}

fn sorted_strings(items: &[String]) -> Vec<String> {
    let mut names: Vec<String> = items.to_vec();
    names.sort();
    names.dedup();
    names
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

/// The host's standing limits: what any task may ever declare. Built by
/// the runtime from the operator config, the workspace, and the tool
/// schemas, so the Mandate judges against the same state the run would.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostLimits {
    /// Ceiling for every [`TaskBounds`] field.
    pub bounds_ceiling: TaskBounds,
    /// Task prompt cap, in characters.
    pub max_task_chars: u32,
    /// Canonical workspace root; scope paths must stay inside it.
    pub workspace_root: PathBuf,
    /// Every tool name that exists on this host.
    pub tool_universe: BTreeSet<String>,
    /// The subset of `tool_universe` that mutates state.
    pub write_tools: BTreeSet<String>,
    /// Operator-configured check names.
    pub configured_checks: BTreeSet<String>,
    /// Whether the host runs trusted (write tools may be granted).
    pub trusted: bool,
}

impl HostLimits {
    /// SHA-256 over the canonical form of these limits. A config change
    /// that touches anything the Mandate consults changes the fingerprint,
    /// which is how revalidation reports `limits_changed`.
    pub fn fingerprint(&self) -> String {
        let mut text = String::new();
        text.push_str(&format!(
            "ceiling:{}:{}:{}:{}\nmax_task_chars:{}\nroot:{}\ntrusted:{}",
            self.bounds_ceiling.max_iterations,
            self.bounds_ceiling.max_cycles,
            self.bounds_ceiling.max_tool_calls,
            self.bounds_ceiling.max_context_chars,
            self.max_task_chars,
            normalize_lexical(&self.workspace_root).to_string_lossy(),
            self.trusted
        ));
        for tool in &self.tool_universe {
            text.push_str("\ntool:");
            text.push_str(tool);
        }
        for tool in &self.write_tools {
            text.push_str("\nwrite_tool:");
            text.push_str(tool);
        }
        for check in &self.configured_checks {
            text.push_str("\ncheck:");
            text.push_str(check);
        }
        sha256_hex(text.as_bytes())
    }
}

/// One mechanical reason a task fails the Mandate. Reasons are data, not
/// prose: dispatch and callers match on the variant, and [`fmt::Display`]
/// renders the same facts for reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rejection {
    /// The task id is empty.
    EmptyId,
    /// The task id exceeds [`TASK_ID_BYTES_MAX`].
    IdTooLong {
        /// Actual length, in bytes.
        bytes: usize,
        /// The cap.
        max: usize,
    },
    /// The prompt is empty or all whitespace.
    EmptyPrompt,
    /// The prompt exceeds the host's task-character cap.
    PromptTooLong {
        /// Actual length, in characters.
        chars: u32,
        /// The cap.
        max: u32,
    },
    /// Both scope paths and scope tools are empty: nothing is declared.
    EmptyScope,
    /// More paths than [`PATHS_MAX`].
    TooManyPaths {
        /// Actual count.
        count: usize,
        /// The cap.
        max: usize,
    },
    /// A scope path escapes the workspace root (lexically, or through a
    /// symlink when the path exists).
    PathEscapesWorkspace {
        /// The offending declared path, as written in the spec.
        path: PathBuf,
    },
    /// More tools than [`TOOLS_MAX`].
    TooManyTools {
        /// Actual count.
        count: usize,
        /// The cap.
        max: usize,
    },
    /// A declared tool does not exist on this host.
    UnknownTool {
        /// The offending tool name.
        tool: String,
    },
    /// A write tool is declared but the scope does not declare write
    /// intent — the spec contradicts itself.
    WriteToolNotDeclared {
        /// The offending tool name.
        tool: String,
    },
    /// Write intent is declared but the host is not trusted.
    WriteNotTrusted,
    /// A write tool is declared, write intent is declared, but the host
    /// is not trusted.
    WriteToolNotTrusted {
        /// The offending tool name.
        tool: String,
    },
    /// A bounds field is zero.
    BoundZero {
        /// The field name.
        field: &'static str,
    },
    /// A bounds field exceeds the host ceiling.
    BoundExceedsCeiling {
        /// The field name.
        field: &'static str,
        /// Declared value.
        requested: u32,
        /// Host ceiling.
        ceiling: u32,
    },
    /// More required checks than [`CHECKS_MAX`].
    TooManyChecks {
        /// Actual count.
        count: usize,
        /// The cap.
        max: usize,
    },
    /// A required check is not operator-configured on this host.
    UnknownCheck {
        /// The offending check name.
        name: String,
    },
}

impl Rejection {
    /// The stable machine name of this reason, used in reports.
    pub fn kind(&self) -> &'static str {
        match self {
            Rejection::EmptyId => "empty_id",
            Rejection::IdTooLong { .. } => "id_too_long",
            Rejection::EmptyPrompt => "empty_prompt",
            Rejection::PromptTooLong { .. } => "prompt_too_long",
            Rejection::EmptyScope => "empty_scope",
            Rejection::TooManyPaths { .. } => "too_many_paths",
            Rejection::PathEscapesWorkspace { .. } => "path_escapes_workspace",
            Rejection::TooManyTools { .. } => "too_many_tools",
            Rejection::UnknownTool { .. } => "unknown_tool",
            Rejection::WriteToolNotDeclared { .. } => "write_tool_not_declared",
            Rejection::WriteNotTrusted => "write_not_trusted",
            Rejection::WriteToolNotTrusted { .. } => "write_tool_not_trusted",
            Rejection::BoundZero { .. } => "bound_zero",
            Rejection::BoundExceedsCeiling { .. } => "bound_exceeds_ceiling",
            Rejection::TooManyChecks { .. } => "too_many_checks",
            Rejection::UnknownCheck { .. } => "unknown_check",
        }
    }

    /// Render this reason as a JSON object for intake reports.
    pub fn to_value(&self) -> Value {
        let mut map = Map::new();
        map.insert("kind".to_owned(), Value::String(self.kind().to_owned()));
        map.insert("detail".to_owned(), Value::String(self.to_string()));
        match self {
            Rejection::PathEscapesWorkspace { path } => {
                map.insert(
                    "path".to_owned(),
                    Value::String(path.to_string_lossy().into_owned()),
                );
            }
            Rejection::UnknownTool { tool }
            | Rejection::WriteToolNotDeclared { tool }
            | Rejection::WriteToolNotTrusted { tool } => {
                map.insert("tool".to_owned(), Value::String(tool.clone()));
            }
            Rejection::UnknownCheck { name } => {
                map.insert("name".to_owned(), Value::String(name.clone()));
            }
            Rejection::BoundZero { field } => {
                map.insert("field".to_owned(), Value::String((*field).to_owned()));
            }
            Rejection::BoundExceedsCeiling {
                field,
                requested,
                ceiling,
            } => {
                map.insert("field".to_owned(), Value::String((*field).to_owned()));
                map.insert("requested".to_owned(), Value::Number((*requested).into()));
                map.insert("ceiling".to_owned(), Value::Number((*ceiling).into()));
            }
            _ => {}
        }
        Value::Object(map)
    }
}

impl fmt::Display for Rejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Rejection::EmptyId => write!(f, "task id is empty"),
            Rejection::IdTooLong { bytes, max } => {
                write!(f, "task id is {bytes} bytes, over the {max}-byte cap")
            }
            Rejection::EmptyPrompt => write!(f, "task prompt is empty"),
            Rejection::PromptTooLong { chars, max } => {
                write!(f, "task prompt is {chars} chars, over the {max}-char cap")
            }
            Rejection::EmptyScope => {
                write!(f, "task scope declares no paths and no tools")
            }
            Rejection::TooManyPaths { count, max } => {
                write!(f, "task scope declares {count} paths, over the {max} cap")
            }
            Rejection::PathEscapesWorkspace { path } => {
                write!(f, "scope path {} escapes the workspace", path.display())
            }
            Rejection::TooManyTools { count, max } => {
                write!(f, "task scope declares {count} tools, over the {max} cap")
            }
            Rejection::UnknownTool { tool } => {
                write!(f, "tool {tool} does not exist on this host")
            }
            Rejection::WriteToolNotDeclared { tool } => {
                write!(f, "write tool {tool} declared without write intent")
            }
            Rejection::WriteNotTrusted => {
                write!(f, "write intent declared on an untrusted host")
            }
            Rejection::WriteToolNotTrusted { tool } => {
                write!(f, "write tool {tool} declared on an untrusted host")
            }
            Rejection::BoundZero { field } => write!(f, "bound {field} is zero"),
            Rejection::BoundExceedsCeiling {
                field,
                requested,
                ceiling,
            } => write!(
                f,
                "bound {field} is {requested}, over the host ceiling {ceiling}"
            ),
            Rejection::TooManyChecks { count, max } => {
                write!(f, "task requires {count} checks, over the {max} cap")
            }
            Rejection::UnknownCheck { name } => {
                write!(f, "check {name} is not configured on this host")
            }
        }
    }
}

/// A Mandate pass: the approved spec snapshot plus the fingerprints of
/// the spec and the limits it was approved under. The snapshot is the
/// record resume revalidates against; it is data the caller may persist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Approval {
    /// The spec exactly as approved.
    pub spec: TaskSpec,
    /// [`TaskSpec::fingerprint`] of `spec` at approval time.
    pub spec_fingerprint: String,
    /// [`HostLimits::fingerprint`] of the limits at approval time.
    pub limits_fingerprint: String,
}

/// The Mandate's verdict: approval, or the collected mechanical reasons
/// the task may not proceed. At most [`REASONS_MAX`] reasons are kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MandateVerdict {
    /// The task may proceed to Eval.
    Approved(Approval),
    /// The task may not proceed; no model is consulted.
    Rejected {
        /// Why not, in evaluation order.
        reasons: Vec<Rejection>,
    },
}

impl MandateVerdict {
    /// Render this verdict as a JSON object for intake reports.
    pub fn to_value(&self) -> Value {
        let mut map = Map::new();
        match self {
            MandateVerdict::Approved(approval) => {
                map.insert("status".to_owned(), Value::String("approved".to_owned()));
                map.insert(
                    "spec_fingerprint".to_owned(),
                    Value::String(approval.spec_fingerprint.clone()),
                );
                map.insert(
                    "limits_fingerprint".to_owned(),
                    Value::String(approval.limits_fingerprint.clone()),
                );
            }
            MandateVerdict::Rejected { reasons } => {
                map.insert("status".to_owned(), Value::String("rejected".to_owned()));
                map.insert(
                    "reasons".to_owned(),
                    Value::Array(reasons.iter().map(Rejection::to_value).collect()),
                );
            }
        }
        Value::Object(map)
    }
}

/// Push one reason, honoring the [`REASONS_MAX`] cap.
fn push_reason(reasons: &mut Vec<Rejection>, reason: Rejection) {
    if reasons.len() < REASONS_MAX {
        reasons.push(reason);
    }
}

/// Run the Mandate: judge `spec` against `limits`, deterministically.
///
/// Checks, in order: identity and prompt shape; scope cardinality and
/// path containment; tool existence and write coherence against host
/// trust; bounds against the ceiling; required checks against the
/// operator configuration. The function reads the filesystem only to
/// canonicalize scope paths that exist (symlink-escape check); it keeps
/// no state and consults no model.
pub fn evaluate_mandate(spec: &TaskSpec, limits: &HostLimits) -> MandateVerdict {
    let mut reasons: Vec<Rejection> = Vec::new();
    check_identity(spec, limits, &mut reasons);
    check_scope_paths(spec, limits, &mut reasons);
    check_scope_tools(spec, limits, &mut reasons);
    check_bounds(spec, limits, &mut reasons);
    check_required_checks(spec, limits, &mut reasons);
    if reasons.is_empty() {
        MandateVerdict::Approved(Approval {
            spec: spec.clone(),
            spec_fingerprint: spec.fingerprint(),
            limits_fingerprint: limits.fingerprint(),
        })
    } else {
        MandateVerdict::Rejected { reasons }
    }
}

fn check_identity(spec: &TaskSpec, limits: &HostLimits, reasons: &mut Vec<Rejection>) {
    if spec.id.is_empty() {
        push_reason(reasons, Rejection::EmptyId);
    } else if spec.id.len() > TASK_ID_BYTES_MAX {
        push_reason(
            reasons,
            Rejection::IdTooLong {
                bytes: spec.id.len(),
                max: TASK_ID_BYTES_MAX,
            },
        );
    }
    if spec.prompt.trim().is_empty() {
        push_reason(reasons, Rejection::EmptyPrompt);
    } else {
        let chars = u32::try_from(spec.prompt.chars().count()).unwrap_or(u32::MAX);
        if chars > limits.max_task_chars {
            push_reason(
                reasons,
                Rejection::PromptTooLong {
                    chars,
                    max: limits.max_task_chars,
                },
            );
        }
    }
}

fn check_scope_paths(spec: &TaskSpec, limits: &HostLimits, reasons: &mut Vec<Rejection>) {
    if spec.scope.paths.is_empty() && spec.scope.tools.is_empty() {
        push_reason(reasons, Rejection::EmptyScope);
    }
    if spec.scope.paths.len() > PATHS_MAX {
        push_reason(
            reasons,
            Rejection::TooManyPaths {
                count: spec.scope.paths.len(),
                max: PATHS_MAX,
            },
        );
        return;
    }
    for path in sorted_unique_paths(&spec.scope.paths) {
        if !path_within_workspace(&path, &limits.workspace_root) {
            push_reason(reasons, Rejection::PathEscapesWorkspace { path });
        }
    }
}

fn check_scope_tools(spec: &TaskSpec, limits: &HostLimits, reasons: &mut Vec<Rejection>) {
    if spec.scope.tools.len() > TOOLS_MAX {
        push_reason(
            reasons,
            Rejection::TooManyTools {
                count: spec.scope.tools.len(),
                max: TOOLS_MAX,
            },
        );
        return;
    }
    let tools: BTreeSet<&String> = spec.scope.tools.iter().collect();
    for tool in tools {
        if !limits.tool_universe.contains(tool) {
            push_reason(reasons, Rejection::UnknownTool { tool: tool.clone() });
            continue;
        }
        if limits.write_tools.contains(tool) {
            if !spec.scope.write {
                push_reason(
                    reasons,
                    Rejection::WriteToolNotDeclared { tool: tool.clone() },
                );
            } else if !limits.trusted {
                push_reason(
                    reasons,
                    Rejection::WriteToolNotTrusted { tool: tool.clone() },
                );
            }
        }
    }
    if spec.scope.write && !limits.trusted {
        push_reason(reasons, Rejection::WriteNotTrusted);
    }
}

fn check_bounds(spec: &TaskSpec, limits: &HostLimits, reasons: &mut Vec<Rejection>) {
    if let Some(field) = spec.bounds.first_zero() {
        push_reason(reasons, Rejection::BoundZero { field });
    }
    if let Some((field, requested, ceiling)) = spec.bounds.first_excess(&limits.bounds_ceiling) {
        push_reason(
            reasons,
            Rejection::BoundExceedsCeiling {
                field,
                requested,
                ceiling,
            },
        );
    }
}

fn check_required_checks(spec: &TaskSpec, limits: &HostLimits, reasons: &mut Vec<Rejection>) {
    if spec.required_checks.len() > CHECKS_MAX {
        push_reason(
            reasons,
            Rejection::TooManyChecks {
                count: spec.required_checks.len(),
                max: CHECKS_MAX,
            },
        );
        return;
    }
    let checks: BTreeSet<&String> = spec.required_checks.iter().collect();
    for name in checks {
        if !limits.configured_checks.contains(name) {
            push_reason(reasons, Rejection::UnknownCheck { name: name.clone() });
        }
    }
}

fn sorted_unique_paths(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut unique: Vec<PathBuf> = paths.to_vec();
    unique.sort();
    unique.dedup();
    unique
}

/// Lexically normalize a path: drop `.` components and resolve `..`
/// against prior components, without touching the filesystem or
/// following symlinks. A `..` at the root is dropped (the root has no
/// parent); containment is judged on the result, component by component.
pub fn normalize_lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// True when `raw` — read relative to `root` unless absolute — stays
/// inside `root`. Containment is component-wise on the lexically
/// normalized path, never a string-prefix test; when the path exists,
/// its canonical form must also stay inside, so a symlink planted in
/// the tree cannot smuggle scope outside the workspace.
fn path_within_workspace(raw: &Path, root: &Path) -> bool {
    let joined = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        root.join(raw)
    };
    let normalized = normalize_lexical(&joined);
    if !normalized.starts_with(root) {
        return false;
    }
    match std::fs::canonicalize(&normalized) {
        Ok(canonical) => canonical.starts_with(root),
        Err(_) => true,
    }
}

/// How clear the task reads to the assessing model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvalLevel {
    /// Unambiguous; a worker could start without asking anything.
    Clear,
    /// Workable with stated assumptions.
    Workable,
    /// Too unclear to dispatch responsibly.
    Unclear,
}

impl EvalLevel {
    /// Parse the wire name; unknown names are rejected, never defaulted.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "clear" => Some(EvalLevel::Clear),
            "workable" => Some(EvalLevel::Workable),
            "unclear" => Some(EvalLevel::Unclear),
            _ => None,
        }
    }

    /// The wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            EvalLevel::Clear => "clear",
            EvalLevel::Workable => "workable",
            EvalLevel::Unclear => "unclear",
        }
    }
}

/// How well the task fits the runtime's shape of work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FitLevel {
    /// Squarely the work this runtime does.
    Fits,
    /// Possible, but awkward or oversized for one run.
    Strained,
    /// Not this runtime's work.
    Misfit,
}

impl FitLevel {
    /// Parse the wire name; unknown names are rejected, never defaulted.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "fits" => Some(FitLevel::Fits),
            "strained" => Some(FitLevel::Strained),
            "misfit" => Some(FitLevel::Misfit),
            _ => None,
        }
    }

    /// The wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            FitLevel::Fits => "fits",
            FitLevel::Strained => "strained",
            FitLevel::Misfit => "misfit",
        }
    }
}

/// The assessor's dispatch advice. Advisory only: a `Decline` informs
/// the dispatcher; it is not a gate, and a `Proceed` grants nothing the
/// Mandate did not already allow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recommendation {
    /// Dispatch under the admitted bounds.
    Proceed,
    /// Advise against dispatch.
    Decline,
}

impl Recommendation {
    /// Parse the wire name; unknown names are rejected, never defaulted.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "proceed" => Some(Recommendation::Proceed),
            "decline" => Some(Recommendation::Decline),
            _ => None,
        }
    }

    /// The wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Recommendation::Proceed => "proceed",
            Recommendation::Decline => "decline",
        }
    }
}

/// One model assessment of an approved task. `narrowed_bounds` is the
/// only place an assessment may touch the task's limits, and only
/// downward: [`admit`] enforces that mechanically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvalReport {
    /// Clarity judgment.
    pub clarity: EvalLevel,
    /// Fit judgment.
    pub fit: FitLevel,
    /// Free-text risk notes, bounded by [`RISK_NOTES_MAX`] and
    /// [`RISK_NOTE_BYTES_MAX`] at parse time.
    pub risk_notes: Vec<String>,
    /// Optional suggested bounds, each field less than or equal to the
    /// approved spec's.
    pub narrowed_bounds: Option<TaskBounds>,
    /// Dispatch advice.
    pub recommendation: Recommendation,
}

impl EvalReport {
    /// Render this report as a JSON object for intake reports.
    pub fn to_value(&self) -> Value {
        let mut map = Map::new();
        map.insert(
            "clarity".to_owned(),
            Value::String(self.clarity.as_str().to_owned()),
        );
        map.insert(
            "fit".to_owned(),
            Value::String(self.fit.as_str().to_owned()),
        );
        map.insert(
            "risk_notes".to_owned(),
            Value::Array(
                self.risk_notes
                    .iter()
                    .map(|note| Value::String(note.clone()))
                    .collect(),
            ),
        );
        map.insert(
            "narrowed_bounds".to_owned(),
            self.narrowed_bounds
                .map_or(Value::Null, |bounds| bounds_to_value(&bounds)),
        );
        map.insert(
            "recommendation".to_owned(),
            Value::String(self.recommendation.as_str().to_owned()),
        );
        Value::Object(map)
    }
}

/// Render bounds as a JSON object for intake reports.
pub fn bounds_to_value(bounds: &TaskBounds) -> Value {
    let mut map = Map::new();
    map.insert(
        "max_iterations".to_owned(),
        Value::Number(bounds.max_iterations.into()),
    );
    map.insert(
        "max_cycles".to_owned(),
        Value::Number(bounds.max_cycles.into()),
    );
    map.insert(
        "max_tool_calls".to_owned(),
        Value::Number(bounds.max_tool_calls.into()),
    );
    map.insert(
        "max_context_chars".to_owned(),
        Value::Number(bounds.max_context_chars.into()),
    );
    Value::Object(map)
}

/// A source of model assessments. Implementations make exactly one
/// bounded model call per `assess`, with no tools offered; the trait is
/// the seam the runtime's model assessor and tests both plug into.
pub trait TaskAssessor {
    /// Assess an approved spec. Called only with a Mandate [`Approval`]
    /// in hand — an implementation may rely on the spec having passed.
    fn assess(&self, spec: &TaskSpec, approval: &Approval) -> Result<EvalReport, IntakeError>;
}

/// Intake failures that are not Mandate rejections: the Eval layer's
/// transport and shape failures, its one hard refusal (widening), and
/// admission misuse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntakeError {
    /// The assessment call itself failed (backend down, timeout).
    EvalUnavailable(String),
    /// The model's reply was not a well-formed Eval report.
    EvalMalformed(String),
    /// The Eval suggested bounds wider than the Mandate approved.
    /// Refused outright; the suggestion is never clamped silently.
    EvalWidening {
        /// The offending field.
        field: &'static str,
        /// Suggested value.
        suggested: u32,
        /// Approved value.
        approved: u32,
    },
    /// The Eval suggested bounds containing a zero field.
    EvalInvalidBounds {
        /// The offending field.
        field: &'static str,
    },
}

impl fmt::Display for IntakeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IntakeError::EvalUnavailable(detail) => {
                write!(f, "eval unavailable: {detail}")
            }
            IntakeError::EvalMalformed(detail) => {
                write!(f, "eval reply malformed: {detail}")
            }
            IntakeError::EvalWidening {
                field,
                suggested,
                approved,
            } => write!(
                f,
                "eval suggested {field} = {suggested}, wider than the approved {approved}"
            ),
            IntakeError::EvalInvalidBounds { field } => {
                write!(f, "eval suggested a zero {field}")
            }
        }
    }
}

impl std::error::Error for IntakeError {}

/// A task admitted for dispatch: the Mandate approval, the Eval report,
/// and the effective bounds (the Eval's narrowing when it offered one,
/// else the approved spec's bounds).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Admission {
    /// The Mandate approval this admission rests on.
    pub approval: Approval,
    /// The Eval report, kept whole so dispatch sees the advice.
    pub eval: EvalReport,
    /// The bounds dispatch must honor.
    pub effective_bounds: TaskBounds,
}

impl Admission {
    /// Render this admission as a JSON object for intake reports.
    pub fn to_value(&self) -> Value {
        let mut map = Map::new();
        map.insert(
            "spec_fingerprint".to_owned(),
            Value::String(self.approval.spec_fingerprint.clone()),
        );
        map.insert("eval".to_owned(), self.eval.to_value());
        map.insert(
            "effective_bounds".to_owned(),
            bounds_to_value(&self.effective_bounds),
        );
        Value::Object(map)
    }
}

/// Combine a Mandate approval and an Eval report into an admission.
///
/// The Eval's one hard limit: suggested bounds must be field-wise less
/// than or equal to the approved spec's. A wider suggestion is refused
/// with [`IntakeError::EvalWidening`], never clamped, so a misbehaving
/// assessor is visible instead of silently reinterpreted. A `Decline`
/// recommendation does not fail admission — it is advisory, and travels
/// with the admission for the dispatcher to weigh.
pub fn admit(approval: Approval, eval: EvalReport) -> Result<Admission, IntakeError> {
    let effective_bounds = match eval.narrowed_bounds {
        None => approval.spec.bounds,
        Some(narrowed) => {
            if let Some(field) = narrowed.first_zero() {
                return Err(IntakeError::EvalInvalidBounds { field });
            }
            if let Some((field, suggested, approved)) = narrowed.first_excess(&approval.spec.bounds)
            {
                return Err(IntakeError::EvalWidening {
                    field,
                    suggested,
                    approved,
                });
            }
            narrowed
        }
    };
    Ok(Admission {
        approval,
        eval,
        effective_bounds,
    })
}

/// Why a resume halted. Either failure halts the resume; both are
/// reported together when both hold, so the caller fixes everything in
/// one pass instead of discovering the second failure next attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevalidationFailure {
    /// The presented spec differs from the approved spec.
    pub spec_mutated: bool,
    /// The Mandate, re-run on the presented spec against current
    /// limits, rejected it — for these reasons.
    pub reasons: Vec<Rejection>,
}

impl RevalidationFailure {
    /// Render this failure as a JSON object for intake reports.
    pub fn to_value(&self) -> Value {
        let mut map = Map::new();
        map.insert("spec_mutated".to_owned(), Value::Bool(self.spec_mutated));
        map.insert(
            "reasons".to_owned(),
            Value::Array(self.reasons.iter().map(Rejection::to_value).collect()),
        );
        Value::Object(map)
    }
}

/// Revalidate an approval for resume, against current limits.
///
/// Two independent checks, both must hold:
///
/// - **Identity**: the presented spec's fingerprint equals the approved
///   one. Any mutation since approval — a widened path, an added tool,
///   a raised bound — halts the resume even when the mutated spec would
///   pass the Mandate on its own merits; it is a different task than the
///   one that was approved, and it must go through intake afresh.
/// - **Currency**: the Mandate re-runs on the presented spec against
///   the *current* limits and tree state. A config change since approval
///   (lowered ceiling, revoked trust, a removed check, a symlink planted
///   in the tree) that now fails the spec halts the resume with the
///   fresh reasons.
///
/// On success the returned approval is fresh: same spec, fingerprint of
/// the current limits. Compare its `limits_fingerprint` with the old
/// approval's to learn whether the host state moved under the approval.
pub fn revalidate(
    approval: &Approval,
    presented: &TaskSpec,
    limits: &HostLimits,
) -> Result<Approval, RevalidationFailure> {
    let spec_mutated = presented.fingerprint() != approval.spec_fingerprint;
    match evaluate_mandate(presented, limits) {
        MandateVerdict::Approved(fresh) => {
            if spec_mutated {
                Err(RevalidationFailure {
                    spec_mutated: true,
                    reasons: Vec::new(),
                })
            } else {
                Ok(fresh)
            }
        }
        MandateVerdict::Rejected { reasons } => Err(RevalidationFailure {
            spec_mutated,
            reasons,
        }),
    }
}

/// The system prompt for the Eval call. It asks for assessment only and
/// states the narrowing rule in the open; enforcement is mechanical in
/// [`admit`], not delegated to this prompt's persuasion.
pub const EVAL_SYSTEM_PROMPT: &str = "\
You are phlow's intake assessor. You assess a task that a deterministic \
host gate has already approved; you do not approve or reject it. Judge \
only the task text and its declared scope and bounds. Reply with exactly \
one JSON object, no prose around it, with fields: \"clarity\" (\"clear\", \
\"workable\", or \"unclear\"), \"fit\" (\"fits\", \"strained\", or \
\"misfit\"), \"risk_notes\" (a list of short strings, may be empty), \
\"narrowed_bounds\" (null, or an object with integer fields \
\"max_iterations\", \"max_cycles\", \"max_tool_calls\", \
\"max_context_chars\" suggesting tighter work limits — never wider than \
the declared bounds), and \"recommendation\" (\"proceed\" or \"decline\").";

/// Build the Eval call's user content: the approved spec as JSON. The
/// content is bounded by the Mandate's own caps (prompt length, list
/// sizes), so no further truncation is needed.
pub fn eval_prompt(spec: &TaskSpec, approval: &Approval) -> String {
    let mut map = Map::new();
    map.insert("id".to_owned(), Value::String(spec.id.clone()));
    map.insert("prompt".to_owned(), Value::String(spec.prompt.clone()));
    map.insert(
        "paths".to_owned(),
        Value::Array(
            spec.scope
                .paths
                .iter()
                .map(|path| Value::String(path.to_string_lossy().into_owned()))
                .collect(),
        ),
    );
    map.insert(
        "tools".to_owned(),
        Value::Array(
            spec.scope
                .tools
                .iter()
                .map(|tool| Value::String(tool.clone()))
                .collect(),
        ),
    );
    map.insert("write".to_owned(), Value::Bool(spec.scope.write));
    map.insert("bounds".to_owned(), bounds_to_value(&spec.bounds));
    map.insert(
        "required_checks".to_owned(),
        Value::Array(
            spec.required_checks
                .iter()
                .map(|name| Value::String(name.clone()))
                .collect(),
        ),
    );
    map.insert(
        "spec_fingerprint".to_owned(),
        Value::String(approval.spec_fingerprint.clone()),
    );
    let value = Value::Object(map);
    serde_json::to_string(&value).unwrap_or_else(|_| "{}".to_owned())
}

/// Parse a model reply into an [`EvalReport`]. The reply may carry prose
/// or code fences around the JSON object; the first `{` to the last `}`
/// is taken. Unknown enum names, missing fields, and oversized replies
/// are [`IntakeError::EvalMalformed`], never defaulted. Risk notes are
/// bounded by truncation (count and bytes), since dropping an excess
/// note loses less than failing the whole assessment.
pub fn parse_eval_report(text: &str) -> Result<EvalReport, IntakeError> {
    if text.len() > EVAL_RESPONSE_BYTES_MAX {
        return Err(IntakeError::EvalMalformed(format!(
            "reply is {} bytes, over the {EVAL_RESPONSE_BYTES_MAX}-byte cap",
            text.len()
        )));
    }
    let start = text
        .find('{')
        .ok_or_else(|| IntakeError::EvalMalformed("no JSON object in reply".to_owned()))?;
    let end = text
        .rfind('}')
        .ok_or_else(|| IntakeError::EvalMalformed("no JSON object in reply".to_owned()))?;
    if end < start {
        return Err(IntakeError::EvalMalformed(
            "no JSON object in reply".to_owned(),
        ));
    }
    let value: Value = serde_json::from_str(&text[start..=end])
        .map_err(|error| IntakeError::EvalMalformed(format!("invalid JSON: {error}")))?;
    let object = value
        .as_object()
        .ok_or_else(|| IntakeError::EvalMalformed("reply is not an object".to_owned()))?;
    let clarity = req_enum(object, "clarity", EvalLevel::parse)?;
    let fit = req_enum(object, "fit", FitLevel::parse)?;
    let recommendation = req_enum(object, "recommendation", Recommendation::parse)?;
    Ok(EvalReport {
        clarity,
        fit,
        risk_notes: parse_risk_notes(object)?,
        narrowed_bounds: parse_narrowed_bounds(object)?,
        recommendation,
    })
}

fn req_enum<T>(
    object: &Map<String, Value>,
    field: &str,
    parse: fn(&str) -> Option<T>,
) -> Result<T, IntakeError> {
    let raw = object
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| IntakeError::EvalMalformed(format!("missing string field {field}")))?;
    parse(raw).ok_or_else(|| IntakeError::EvalMalformed(format!("unknown {field} value {raw:?}")))
}

fn parse_risk_notes(object: &Map<String, Value>) -> Result<Vec<String>, IntakeError> {
    let Some(raw) = object.get("risk_notes") else {
        return Ok(Vec::new());
    };
    let items = raw
        .as_array()
        .ok_or_else(|| IntakeError::EvalMalformed("risk_notes is not a list".to_owned()))?;
    let mut notes = Vec::new();
    for item in items.iter().take(RISK_NOTES_MAX) {
        let note = item
            .as_str()
            .ok_or_else(|| IntakeError::EvalMalformed("risk note is not a string".to_owned()))?;
        notes.push(truncate_chars(note, RISK_NOTE_BYTES_MAX));
    }
    Ok(notes)
}

fn parse_narrowed_bounds(object: &Map<String, Value>) -> Result<Option<TaskBounds>, IntakeError> {
    let Some(raw) = object.get("narrowed_bounds") else {
        return Ok(None);
    };
    if raw.is_null() {
        return Ok(None);
    }
    let bounds = raw
        .as_object()
        .ok_or_else(|| IntakeError::EvalMalformed("narrowed_bounds is not an object".to_owned()))?;
    let field = |name: &str| -> Result<u32, IntakeError> {
        let value = bounds.get(name).and_then(Value::as_u64).ok_or_else(|| {
            IntakeError::EvalMalformed(format!("narrowed_bounds missing integer {name}"))
        })?;
        u32::try_from(value)
            .map_err(|_| IntakeError::EvalMalformed(format!("narrowed_bounds {name} out of range")))
    };
    Ok(Some(TaskBounds {
        max_iterations: field("max_iterations")?,
        max_cycles: field("max_cycles")?,
        max_tool_calls: field("max_tool_calls")?,
        max_context_chars: field("max_context_chars")?,
    }))
}

/// Truncate to at most `max_bytes` bytes on a char boundary.
fn truncate_chars(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_owned();
    }
    let mut end = max_bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds() -> TaskBounds {
        TaskBounds {
            max_iterations: 8,
            max_cycles: 2,
            max_tool_calls: 32,
            max_context_chars: 65_536,
        }
    }

    fn limits() -> HostLimits {
        HostLimits {
            bounds_ceiling: TaskBounds {
                max_iterations: 32,
                max_cycles: 5,
                max_tool_calls: 256,
                max_context_chars: 1_000_000,
            },
            max_task_chars: 100_000,
            workspace_root: PathBuf::from("/work"),
            tool_universe: ["file_list", "file_read", "file_write", "flow_check"]
                .iter()
                .map(|name| (*name).to_owned())
                .collect(),
            write_tools: ["file_write", "flow_check"]
                .iter()
                .map(|name| (*name).to_owned())
                .collect(),
            configured_checks: ["smoke"].iter().map(|name| (*name).to_owned()).collect(),
            trusted: true,
        }
    }

    fn spec() -> TaskSpec {
        TaskSpec {
            id: "task-1".to_owned(),
            prompt: "Write a greeting file.".to_owned(),
            scope: TaskScope {
                paths: vec![PathBuf::from("src")],
                tools: vec!["file_read".to_owned(), "file_write".to_owned()],
                write: true,
            },
            bounds: bounds(),
            required_checks: vec!["smoke".to_owned()],
        }
    }

    fn approved() -> Approval {
        match evaluate_mandate(&spec(), &limits()) {
            MandateVerdict::Approved(approval) => approval,
            MandateVerdict::Rejected { reasons } => {
                panic!("fixture spec must approve: {reasons:?}")
            }
        }
    }

    fn eval_ok() -> EvalReport {
        EvalReport {
            clarity: EvalLevel::Clear,
            fit: FitLevel::Fits,
            risk_notes: vec!["touches one directory".to_owned()],
            narrowed_bounds: None,
            recommendation: Recommendation::Proceed,
        }
    }

    // ------------------------------------------------------------------
    // Validation: well-formed inputs behave as specified.
    // ------------------------------------------------------------------

    #[test]
    fn mandate_approves_a_well_formed_spec() {
        let approval = approved();
        assert_eq!(approval.spec, spec());
        assert_eq!(approval.spec_fingerprint, spec().fingerprint());
        assert_eq!(approval.limits_fingerprint, limits().fingerprint());
    }

    #[test]
    fn fingerprint_is_set_order_insensitive() {
        let mut shuffled = spec();
        shuffled.scope.tools.reverse();
        shuffled.required_checks.reverse();
        assert_eq!(spec().fingerprint(), shuffled.fingerprint());
    }

    #[test]
    fn fingerprint_changes_with_any_declaration() {
        let mut changed = spec();
        changed.bounds.max_tool_calls += 1;
        assert_ne!(spec().fingerprint(), changed.fingerprint());
    }

    #[test]
    fn mandate_accepts_paths_at_and_below_the_root() {
        let mut at_root = spec();
        at_root.scope.paths = vec![PathBuf::from(".")];
        assert!(matches!(
            evaluate_mandate(&at_root, &limits()),
            MandateVerdict::Approved(_)
        ));
        let mut deep = spec();
        deep.scope.paths = vec![PathBuf::from("a/b/../c")];
        assert!(matches!(
            evaluate_mandate(&deep, &limits()),
            MandateVerdict::Approved(_)
        ));
    }

    #[test]
    fn admit_without_narrowing_keeps_spec_bounds() {
        let admission = admit(approved(), eval_ok()).expect("admission");
        assert_eq!(admission.effective_bounds, bounds());
    }

    #[test]
    fn admit_applies_a_narrower_suggestion() {
        let mut eval = eval_ok();
        let narrower = TaskBounds {
            max_iterations: 4,
            ..bounds()
        };
        eval.narrowed_bounds = Some(narrower);
        let admission = admit(approved(), eval).expect("admission");
        assert_eq!(admission.effective_bounds, narrower);
    }

    #[test]
    fn admit_keeps_a_decline_as_advice_not_a_gate() {
        let mut eval = eval_ok();
        eval.recommendation = Recommendation::Decline;
        let admission = admit(approved(), eval).expect("decline is advisory");
        assert_eq!(admission.eval.recommendation, Recommendation::Decline);
    }

    #[test]
    fn parse_eval_report_reads_a_fenced_reply() {
        let text = concat!(
            "Assessment:\n```json\n",
            "{\"clarity\":\"workable\",\"fit\":\"strained\",",
            "\"risk_notes\":[\"large tree\"],\"narrowed_bounds\":null,",
            "\"recommendation\":\"proceed\"}\n```"
        );
        let report = parse_eval_report(text).expect("parses");
        assert_eq!(report.clarity, EvalLevel::Workable);
        assert_eq!(report.fit, FitLevel::Strained);
        assert_eq!(report.risk_notes, vec!["large tree".to_owned()]);
        assert_eq!(report.narrowed_bounds, None);
    }

    #[test]
    fn revalidate_passes_an_unchanged_spec_under_unchanged_limits() {
        let fresh = revalidate(&approved(), &spec(), &limits()).expect("revalidates");
        assert_eq!(fresh.spec_fingerprint, spec().fingerprint());
    }

    #[test]
    fn revalidate_reports_limits_changed_but_still_passes() {
        // A config change that does not affect this spec's verdict (a new
        // unrelated check appears) refreshes the approval and moves the
        // limits fingerprint — visible, not a halt.
        let mut moved = limits();
        moved.configured_checks.insert("extra".to_owned());
        let fresh = revalidate(&approved(), &spec(), &moved).expect("still valid");
        assert_ne!(fresh.limits_fingerprint, approved().limits_fingerprint);
    }

    // ------------------------------------------------------------------
    // Adversarial: hostile or mutated inputs are refused, never widened,
    // never defaulted.
    // ------------------------------------------------------------------

    #[test]
    fn mandate_rejects_an_escaping_path() {
        let mut hostile = spec();
        hostile.scope.paths = vec![PathBuf::from("../outside")];
        match evaluate_mandate(&hostile, &limits()) {
            MandateVerdict::Rejected { reasons } => assert!(
                reasons
                    .iter()
                    .any(|r| matches!(r, Rejection::PathEscapesWorkspace { .. }))
            ),
            MandateVerdict::Approved(_) => panic!("escape must not approve"),
        }
    }

    #[test]
    fn mandate_rejects_an_absolute_escape_and_an_unknown_tool() {
        let mut hostile = spec();
        hostile.scope.paths = vec![PathBuf::from("/etc")];
        hostile.scope.tools = vec!["shell_exec".to_owned()];
        match evaluate_mandate(&hostile, &limits()) {
            MandateVerdict::Rejected { reasons } => {
                assert!(
                    reasons
                        .iter()
                        .any(|r| matches!(r, Rejection::PathEscapesWorkspace { .. }))
                );
                assert!(
                    reasons
                        .iter()
                        .any(|r| matches!(r, Rejection::UnknownTool { .. }))
                );
            }
            MandateVerdict::Approved(_) => panic!("hostile spec must not approve"),
        }
    }

    #[test]
    fn mandate_rejects_write_tools_on_an_untrusted_host() {
        let mut untrusted = limits();
        untrusted.trusted = false;
        match evaluate_mandate(&spec(), &untrusted) {
            MandateVerdict::Rejected { reasons } => {
                assert!(
                    reasons
                        .iter()
                        .any(|r| matches!(r, Rejection::WriteToolNotTrusted { .. }))
                );
                assert!(
                    reasons
                        .iter()
                        .any(|r| matches!(r, Rejection::WriteNotTrusted))
                );
            }
            MandateVerdict::Approved(_) => panic!("untrusted write must not approve"),
        }
    }

    #[test]
    fn mandate_rejects_a_write_tool_without_write_intent() {
        let mut contradictory = spec();
        contradictory.scope.write = false;
        match evaluate_mandate(&contradictory, &limits()) {
            MandateVerdict::Rejected { reasons } => assert!(
                reasons
                    .iter()
                    .any(|r| matches!(r, Rejection::WriteToolNotDeclared { .. }))
            ),
            MandateVerdict::Approved(_) => panic!("contradiction must not approve"),
        }
    }

    #[test]
    fn mandate_rejects_bounds_over_the_ceiling_and_zero() {
        let mut greedy = spec();
        greedy.bounds.max_tool_calls = 10_000;
        match evaluate_mandate(&greedy, &limits()) {
            MandateVerdict::Rejected { reasons } => assert!(reasons.iter().any(|r| matches!(
                r,
                Rejection::BoundExceedsCeiling {
                    field: "max_tool_calls",
                    requested: 10_000,
                    ceiling: 256
                }
            ))),
            MandateVerdict::Approved(_) => panic!("over-ceiling must not approve"),
        }
        let mut zeroed = spec();
        zeroed.bounds.max_iterations = 0;
        match evaluate_mandate(&zeroed, &limits()) {
            MandateVerdict::Rejected { reasons } => assert!(
                reasons
                    .iter()
                    .any(|r| matches!(r, Rejection::BoundZero { .. }))
            ),
            MandateVerdict::Approved(_) => panic!("zero bound must not approve"),
        }
    }

    #[test]
    fn mandate_rejects_an_unconfigured_required_check() {
        let mut needy = spec();
        needy.required_checks = vec!["does_not_exist".to_owned()];
        match evaluate_mandate(&needy, &limits()) {
            MandateVerdict::Rejected { reasons } => assert!(
                reasons
                    .iter()
                    .any(|r| matches!(r, Rejection::UnknownCheck { .. }))
            ),
            MandateVerdict::Approved(_) => panic!("unknown check must not approve"),
        }
    }

    #[test]
    fn admit_refuses_an_eval_that_widens_bounds() {
        // The assessor is a model: it may be confused, injected, or
        // compromised. A widening suggestion is refused, never clamped.
        let mut eval = eval_ok();
        eval.narrowed_bounds = Some(TaskBounds {
            max_tool_calls: bounds().max_tool_calls + 1,
            ..bounds()
        });
        match admit(approved(), eval) {
            Err(IntakeError::EvalWidening {
                field,
                suggested,
                approved,
            }) => {
                assert_eq!(field, "max_tool_calls");
                assert_eq!(suggested, bounds().max_tool_calls + 1);
                assert_eq!(approved, bounds().max_tool_calls);
            }
            other => panic!("widening must be refused, got {other:?}"),
        }
    }

    #[test]
    fn admit_refuses_an_eval_that_zeroes_bounds() {
        let mut eval = eval_ok();
        eval.narrowed_bounds = Some(TaskBounds {
            max_cycles: 0,
            ..bounds()
        });
        assert!(matches!(
            admit(approved(), eval),
            Err(IntakeError::EvalInvalidBounds {
                field: "max_cycles"
            })
        ));
    }

    #[test]
    fn parse_eval_report_rejects_unknown_enum_and_missing_fields() {
        let unknown = concat!(
            "{\"clarity\":\"great\",\"fit\":\"fits\",\"risk_notes\":[],",
            "\"narrowed_bounds\":null,\"recommendation\":\"proceed\"}"
        );
        assert!(matches!(
            parse_eval_report(unknown),
            Err(IntakeError::EvalMalformed(_))
        ));
        let missing = "{\"clarity\":\"clear\"}";
        assert!(matches!(
            parse_eval_report(missing),
            Err(IntakeError::EvalMalformed(_))
        ));
        let prose = "I think this task looks fine.";
        assert!(matches!(
            parse_eval_report(prose),
            Err(IntakeError::EvalMalformed(_))
        ));
    }

    #[test]
    fn revalidate_catches_a_scope_mutated_after_approval() {
        // The spec passed the Mandate, was approved — then its scope
        // quietly grew a path before resume. The mutated spec would pass
        // the Mandate on its own merits; the fingerprint is what catches
        // it. A stale approval must not carry.
        let mut mutated = spec();
        mutated.scope.paths.push(PathBuf::from("other"));
        let failure = revalidate(&approved(), &mutated, &limits())
            .expect_err("mutated spec must halt the resume");
        assert!(failure.spec_mutated);
        assert!(failure.reasons.is_empty());
    }

    #[test]
    fn revalidate_catches_a_bound_raised_after_approval() {
        let mut mutated = spec();
        mutated.bounds.max_iterations += 1;
        let failure = revalidate(&approved(), &mutated, &limits())
            .expect_err("raised bound must halt the resume");
        assert!(failure.spec_mutated);
    }

    #[test]
    fn revalidate_halts_after_a_config_change_lowers_the_ceiling() {
        // Resume after the operator tightened the host ceiling below the
        // approved spec's bounds: same spec, new world — the fresh
        // Mandate run is what halts it.
        let mut tightened = limits();
        tightened.bounds_ceiling.max_tool_calls = 16;
        let failure = revalidate(&approved(), &spec(), &tightened)
            .expect_err("tightened ceiling must halt the resume");
        assert!(!failure.spec_mutated);
        assert!(failure.reasons.iter().any(|r| matches!(
            r,
            Rejection::BoundExceedsCeiling {
                field: "max_tool_calls",
                ..
            }
        )));
    }

    #[test]
    fn revalidate_halts_after_trust_is_revoked() {
        let mut revoked = limits();
        revoked.trusted = false;
        let failure = revalidate(&approved(), &spec(), &revoked)
            .expect_err("revoked trust must halt the resume");
        assert!(!failure.spec_mutated);
        assert!(!failure.reasons.is_empty());
    }

    #[test]
    fn revalidate_halts_after_a_required_check_is_removed() {
        let mut shrunk = limits();
        shrunk.configured_checks.clear();
        let failure = revalidate(&approved(), &spec(), &shrunk)
            .expect_err("removed check must halt the resume");
        assert!(
            failure
                .reasons
                .iter()
                .any(|r| matches!(r, Rejection::UnknownCheck { .. }))
        );
    }
}
