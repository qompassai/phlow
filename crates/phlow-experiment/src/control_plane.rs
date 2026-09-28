//! Host-owned control plane: scheduler types and pure scheduling logic.
//!
//! The scheduler owns a bounded directed acyclic graph of tasks. Models may
//! *propose* tasks and dependencies, but the host validates the graph before
//! admitting work. Everything in this module is pure logic over owned data —
//! there is no runtime concurrency here, only the invariants a future
//! asynchronous scheduler must preserve:
//!
//! - one exhaustive [`NodeState`] enum (no contradictory booleans),
//! - delegation passes a *strict* subset of the parent's capabilities,
//! - results are published at most once, only for the current generation,
//! - cancellation is terminal and blocks later publication,
//! - a full queue rejects explicitly instead of dropping silently.

use crate::error::ExperimentError;
use std::collections::{BTreeMap, VecDeque};

// ---------------------------------------------------------------------------
// Bounds (all with units)
// ---------------------------------------------------------------------------

/// Maximum characters in any scheduler identifier.
pub const ID_CHARS_MAX: usize = 128;
/// Maximum characters in a content digest string.
pub const DIGEST_CHARS_MAX: usize = 256;
/// Maximum characters in a revision or snapshot label.
pub const REVISION_CHARS_MAX: usize = 128;
/// Maximum dependency ids on one node.
pub const DEPENDENCY_IDS_MAX: usize = 32;
/// Maximum tool names in one capability set.
pub const CAPABILITY_TOOLS_MAX: usize = 64;
/// Maximum path entries in one capability set.
pub const CAPABILITY_PATHS_MAX: usize = 64;
/// Maximum characters in one capability tool/path entry.
pub const CAPABILITY_ENTRY_CHARS_MAX: usize = 256;
/// Maximum cancelled run ids retained for late-result rejection.
pub const CANCELLED_RUNS_MAX: usize = 1024;

/// Default maximum concurrent workers.
pub const WORKERS_MAX_DEFAULT: u64 = 4;
/// Default scheduler queue capacity, in nodes.
pub const QUEUE_CAPACITY_DEFAULT: usize = 16;
/// Default maximum children admitted per task.
pub const CHILDREN_PER_TASK_MAX_DEFAULT: u64 = 8;
/// Default maximum delegation depth (generations).
pub const DEPTH_MAX_DEFAULT: u64 = 3;
/// Default per-task deadline, in milliseconds (5 minutes).
pub const TASK_DEADLINE_MS_DEFAULT: u64 = 300_000;
/// Default aggregate tool-call budget for one run.
pub const AGGREGATE_TOOL_CALLS_MAX_DEFAULT: u64 = 1_024;
/// Default aggregate output budget for one run, in bytes (16 MiB).
pub const AGGREGATE_OUTPUT_BYTES_MAX_DEFAULT: u64 = 16_777_216;

// ---------------------------------------------------------------------------
// Node state
// ---------------------------------------------------------------------------

/// The lifecycle state of one scheduler node.
///
/// One exhaustive enum replaces the contradictory boolean triples
/// (`done`/`failed`/`cancelled`) the experiment plan forbids. Terminal
/// states are exactly the seven the plan allows; see
/// [`NodeState::is_terminal`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeState {
    /// Proposed by a model; not yet validated by the host.
    Proposed,
    /// Validated and queued by the host.
    Admitted,
    /// Preparing inputs and the workspace snapshot.
    Preparing,
    /// Running.
    Executing,
    /// Host is verifying the result against evidence.
    Verifying,
    /// Under independent review.
    Reviewing,
    /// Terminal: completed and accepted.
    Succeeded,
    /// Terminal: completed with failure.
    Failed,
    /// Terminal: cancelled by the host or an ancestor.
    Cancelled,
    /// Terminal: exceeded its absolute deadline.
    TimedOut,
    /// Terminal: rejected by host policy.
    Rejected,
    /// Terminal: inputs changed after the worker's snapshot.
    Stale,
    /// Terminal: replaced by a newer generation.
    Superseded,
}

impl NodeState {
    /// Returns true for the seven terminal states the plan allows.
    ///
    /// Non-terminal states are exactly: Proposed, Admitted, Preparing,
    /// Executing, Verifying, Reviewing.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded
                | Self::Failed
                | Self::Cancelled
                | Self::TimedOut
                | Self::Rejected
                | Self::Stale
                | Self::Superseded
        )
    }

    /// The stable machine-readable name used in events and error text.
    pub fn name(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Admitted => "admitted",
            Self::Preparing => "preparing",
            Self::Executing => "executing",
            Self::Verifying => "verifying",
            Self::Reviewing => "reviewing",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::TimedOut => "timed_out",
            Self::Rejected => "rejected",
            Self::Stale => "stale",
            Self::Superseded => "superseded",
        }
    }
}

// ---------------------------------------------------------------------------
// Worker roles
// ---------------------------------------------------------------------------

/// The role a worker plays. Roles bound what the worker may do; they never
/// grant new authority. In particular, [`WorkerRole::can_write_production`]
/// is false for every role — no experiment worker writes production state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkerRole {
    /// Produces a bounded task graph and acceptance criteria. Read-only.
    Planner,
    /// Makes the smallest requested change in a scoped candidate workspace.
    Implementer,
    /// Adds adversarial and regression tests in a scoped test workspace.
    TestAuthor,
    /// Reviews trust, authority, injection, path, process, network bounds.
    SecurityReviewer,
    /// Detects resource regressions and invalid performance claims.
    PerformanceReviewer,
    /// Reviews behavior against acceptance criteria.
    CorrectnessReviewer,
    /// Attempts prompt injection, escalation, and policy bypass with inert
    /// fixtures. Never writes production state.
    Adversary,
    /// Produces a candidate promotion report from host-verified evidence.
    Integrator,
}

impl WorkerRole {
    /// All eight roles, for exhaustive audits (e.g. asserting no role may
    /// write production).
    pub fn all() -> [WorkerRole; 8] {
        [
            Self::Planner,
            Self::Implementer,
            Self::TestAuthor,
            Self::SecurityReviewer,
            Self::PerformanceReviewer,
            Self::CorrectnessReviewer,
            Self::Adversary,
            Self::Integrator,
        ]
    }

    /// The stable machine-readable name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Planner => "planner",
            Self::Implementer => "implementer",
            Self::TestAuthor => "test_author",
            Self::SecurityReviewer => "security_reviewer",
            Self::PerformanceReviewer => "performance_reviewer",
            Self::CorrectnessReviewer => "correctness_reviewer",
            Self::Adversary => "adversary",
            Self::Integrator => "integrator",
        }
    }

    /// The write access this role holds.
    ///
    /// Implementer and TestAuthor are scoped to their candidate workspace;
    /// the Adversary explicitly has no production write (it attacks with
    /// controlled fixtures, not production mutation); everyone else has
    /// none.
    pub fn write_access(self) -> WriteAccess {
        match self {
            Self::Implementer | Self::TestAuthor => WriteAccess::ScopedCandidate,
            Self::Adversary => WriteAccess::NoProductionWrite,
            _ => WriteAccess::None,
        }
    }

    /// Always false. No role may write production state; this is asserted,
    /// not configured.
    pub fn can_write_production(self) -> bool {
        false
    }
}

/// The write access a worker role holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteAccess {
    /// No write access at all.
    None,
    /// May write only inside its assigned candidate workspace.
    ScopedCandidate,
    /// Explicitly denied production write (the adversary role).
    NoProductionWrite,
}

// ---------------------------------------------------------------------------
// Capability sets
// ---------------------------------------------------------------------------

/// The tools, paths, and budgets delegated to one worker.
///
/// A child always receives a *strict* subset of its parent's capabilities:
/// [`CapabilitySet::derive_child`] rejects escalation
/// ([`ExperimentError::CapabilityEscalation`]) and non-strict delegation
/// ([`ExperimentError::NotStrictSubset`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilitySet {
    tools: Vec<String>,
    paths: Vec<String>,
    tool_calls_max: u64,
    output_bytes_max: u64,
}

impl CapabilitySet {
    /// Builds a root capability set (host-constructed, not delegated).
    ///
    /// Accepted: non-empty tool/path entries within bounds, positive
    /// budgets. Rejected: oversized lists or entries, zero budgets.
    pub fn new(
        tools: Vec<String>,
        paths: Vec<String>,
        tool_calls_max: u64,
        output_bytes_max: u64,
    ) -> Result<Self, ExperimentError> {
        Self::check_entries(&tools, &paths)?;
        Self::check_budgets(tool_calls_max, output_bytes_max)?;
        Ok(Self {
            tools,
            paths,
            tool_calls_max,
            output_bytes_max,
        })
    }

    /// Derives a child capability set from `self` (the parent).
    ///
    /// Accepted: a strict subset of the parent's tools, paths, and budgets.
    /// Rejected: any broader tool, path, or budget
    /// ([`ExperimentError::CapabilityEscalation`]); an equal set
    /// ([`ExperimentError::NotStrictSubset`]) — delegation must shrink
    /// authority, never merely re-label it.
    pub fn derive_child(
        &self,
        tools: Vec<String>,
        paths: Vec<String>,
        tool_calls_max: u64,
        output_bytes_max: u64,
    ) -> Result<Self, ExperimentError> {
        Self::check_entries(&tools, &paths)?;
        Self::check_budgets(tool_calls_max, output_bytes_max)?;
        let child = Self {
            tools,
            paths,
            tool_calls_max,
            output_bytes_max,
        };
        if !child.is_subset_of(self) {
            let detail = if child.tool_calls_max > self.tool_calls_max
                || child.output_bytes_max > self.output_bytes_max
            {
                "budget exceeds parent"
            } else if child.paths.iter().any(|p| !self.paths.contains(p)) {
                "path not in parent set"
            } else {
                "tool not in parent set"
            };
            return Err(ExperimentError::CapabilityEscalation { detail });
        }
        if !child.is_strictly_smaller(self) {
            return Err(ExperimentError::NotStrictSubset);
        }
        Ok(child)
    }

    /// True when every tool, path, and budget of `self` fits inside `parent`.
    pub fn is_subset_of(&self, parent: &CapabilitySet) -> bool {
        self.tools.iter().all(|t| parent.tools.contains(t))
            && self.paths.iter().all(|p| parent.paths.contains(p))
            && self.tool_calls_max <= parent.tool_calls_max
            && self.output_bytes_max <= parent.output_bytes_max
    }

    /// The tool names delegated.
    pub fn tools(&self) -> &[String] {
        &self.tools
    }

    /// The path entries delegated.
    pub fn paths(&self) -> &[String] {
        &self.paths
    }

    /// The delegated tool-call budget.
    pub fn tool_calls_max(&self) -> u64 {
        self.tool_calls_max
    }

    /// The delegated output budget, in bytes.
    pub fn output_bytes_max(&self) -> u64 {
        self.output_bytes_max
    }

    /// True when at least one dimension is strictly smaller than the
    /// parent's. Callers must check [`CapabilitySet::is_subset_of`] first;
    /// given a subset, a smaller dimension proves strictness.
    fn is_strictly_smaller(&self, parent: &CapabilitySet) -> bool {
        self.tools.len() < parent.tools.len()
            || self.paths.len() < parent.paths.len()
            || self.tool_calls_max < parent.tool_calls_max
            || self.output_bytes_max < parent.output_bytes_max
    }

    fn check_entries(tools: &[String], paths: &[String]) -> Result<(), ExperimentError> {
        if tools.len() > CAPABILITY_TOOLS_MAX {
            return Err(ExperimentError::TooManyItems {
                field: "capability tools",
                max: CAPABILITY_TOOLS_MAX,
            });
        }
        if paths.len() > CAPABILITY_PATHS_MAX {
            return Err(ExperimentError::TooManyItems {
                field: "capability paths",
                max: CAPABILITY_PATHS_MAX,
            });
        }
        for entry in tools.iter().chain(paths.iter()) {
            if entry.is_empty() {
                return Err(ExperimentError::EmptyField {
                    field: "capability entry",
                });
            }
            if entry.len() > CAPABILITY_ENTRY_CHARS_MAX {
                return Err(ExperimentError::TextTooLong {
                    field: "capability entry",
                    max: CAPABILITY_ENTRY_CHARS_MAX,
                    got: entry.len(),
                });
            }
        }
        Ok(())
    }

    fn check_budgets(tool_calls_max: u64, output_bytes_max: u64) -> Result<(), ExperimentError> {
        if tool_calls_max == 0 {
            return Err(ExperimentError::InvalidBudget {
                field: "tool_calls_max",
            });
        }
        if output_bytes_max == 0 {
            return Err(ExperimentError::InvalidBudget {
                field: "output_bytes_max",
            });
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Identifiers: distinct types for distinct concepts
// ---------------------------------------------------------------------------

/// Validates one identifier: non-empty, at most [`ID_CHARS_MAX`] bytes.
fn check_id(field: &'static str, value: &str) -> Result<String, ExperimentError> {
    if value.is_empty() {
        return Err(ExperimentError::EmptyField { field });
    }
    if value.len() > ID_CHARS_MAX {
        return Err(ExperimentError::TextTooLong {
            field,
            max: ID_CHARS_MAX,
            got: value.len(),
        });
    }
    Ok(value.to_string())
}

/// Validates one digest string: non-empty, at most [`DIGEST_CHARS_MAX`] bytes.
fn check_digest(field: &'static str, value: &str) -> Result<String, ExperimentError> {
    if value.is_empty() {
        return Err(ExperimentError::EmptyField { field });
    }
    if value.len() > DIGEST_CHARS_MAX {
        return Err(ExperimentError::TextTooLong {
            field,
            max: DIGEST_CHARS_MAX,
            got: value.len(),
        });
    }
    Ok(value.to_string())
}

/// Validates one revision/snapshot label: non-empty, at most
/// [`REVISION_CHARS_MAX`] bytes.
fn check_revision(field: &'static str, value: &str) -> Result<String, ExperimentError> {
    if value.is_empty() {
        return Err(ExperimentError::EmptyField { field });
    }
    if value.len() > REVISION_CHARS_MAX {
        return Err(ExperimentError::TextTooLong {
            field,
            max: REVISION_CHARS_MAX,
            got: value.len(),
        });
    }
    Ok(value.to_string())
}

/// Identifies one experiment run. Distinct from [`NodeId`] and
/// [`ExperimentId`] so the type system rejects mixing them.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RunId(String);

impl RunId {
    /// Builds a run id; rejects empty or over-long values.
    pub fn new(value: &str) -> Result<Self, ExperimentError> {
        check_id("run_id", value).map(RunId)
    }

    /// The id text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Identifies one experiment (a family of runs over one hypothesis).
/// Distinct from [`RunId`] and [`NodeId`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ExperimentId(String);

impl ExperimentId {
    /// Builds an experiment id; rejects empty or over-long values.
    pub fn new(value: &str) -> Result<Self, ExperimentError> {
        check_id("experiment_id", value).map(ExperimentId)
    }

    /// The id text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Identifies one scheduler node. Distinct from [`RunId`] and
/// [`ExperimentId`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(String);

impl NodeId {
    /// Builds a node id; rejects empty or over-long values.
    pub fn new(value: &str) -> Result<Self, ExperimentError> {
        check_id("node_id", value).map(NodeId)
    }

    /// The id text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

// ---------------------------------------------------------------------------
// Scheduler node
// ---------------------------------------------------------------------------

/// Constructor parameters for [`SchedulerNode`]. All fields are required
/// except `parent_node_id` (absent for roots) — the plan's required
/// scheduler fields, with `deadline`/`cpu_budget`/`memory_budget` carrying
/// explicit `_ms`/`_bytes` units.
#[derive(Debug, Clone)]
pub struct NodeParams {
    /// The run this node belongs to.
    pub run_id: RunId,
    /// The experiment this node belongs to.
    pub experiment_id: ExperimentId,
    /// Pinned baseline revision the node works against.
    pub baseline_revision: String,
    /// Immutable workspace snapshot label the node was prepared from.
    pub workspace_snapshot: String,
    /// This node's id.
    pub node_id: NodeId,
    /// The parent node, if any.
    pub parent_node_id: Option<NodeId>,
    /// The role this node plays.
    pub role: WorkerRole,
    /// The delegated capability set.
    pub capabilities: CapabilitySet,
    /// Digest of the immutable input the node was given.
    pub input_digest: String,
    /// Ids of nodes this node depends on (at most [`DEPENDENCY_IDS_MAX`]).
    pub dependency_ids: Vec<NodeId>,
    /// Delegation generation (depth); the scheduler bounds it.
    pub generation: u64,
    /// Attempt number for this node (0 for the first attempt).
    pub attempt: u32,
    /// Absolute deadline, in milliseconds since the run epoch.
    pub deadline_ms: u64,
    /// CPU budget, in milliseconds.
    pub cpu_budget_ms: u64,
    /// Memory budget, in bytes.
    pub memory_budget_bytes: u64,
    /// Maximum output the node may produce, in bytes.
    pub output_bytes_max: u64,
    /// Remaining tool calls delegated to this node.
    pub tool_calls_remaining: u64,
}

/// One node in the host-owned scheduler graph.
///
/// Nodes start in [`NodeState::Proposed`]; only the [`Scheduler`] moves them
/// thereafter. The result digest is absent until a terminal result is
/// published exactly once.
#[derive(Debug, Clone)]
pub struct SchedulerNode {
    run_id: RunId,
    experiment_id: ExperimentId,
    baseline_revision: String,
    workspace_snapshot: String,
    node_id: NodeId,
    parent_node_id: Option<NodeId>,
    role: WorkerRole,
    capabilities: CapabilitySet,
    input_digest: String,
    dependency_ids: Vec<NodeId>,
    generation: u64,
    attempt: u32,
    state: NodeState,
    deadline_ms: u64,
    cpu_budget_ms: u64,
    memory_budget_bytes: u64,
    output_bytes_max: u64,
    tool_calls_remaining: u64,
    result_digest: Option<String>,
}

impl SchedulerNode {
    /// Builds a node in [`NodeState::Proposed`] after validating every
    /// field. Rejected: over-long ids/digests/revisions, more than
    /// [`DEPENDENCY_IDS_MAX`] dependencies, or any zero budget.
    pub fn new(params: NodeParams) -> Result<Self, ExperimentError> {
        if params.dependency_ids.len() > DEPENDENCY_IDS_MAX {
            return Err(ExperimentError::TooManyItems {
                field: "dependency_ids",
                max: DEPENDENCY_IDS_MAX,
            });
        }
        let baseline_revision = check_revision("baseline_revision", &params.baseline_revision)?;
        let workspace_snapshot = check_revision("workspace_snapshot", &params.workspace_snapshot)?;
        let input_digest = check_digest("input_digest", &params.input_digest)?;
        Self::check_positive("deadline_ms", params.deadline_ms)?;
        Self::check_positive("cpu_budget_ms", params.cpu_budget_ms)?;
        Self::check_positive("memory_budget_bytes", params.memory_budget_bytes)?;
        Self::check_positive("output_bytes_max", params.output_bytes_max)?;
        Self::check_positive("tool_calls_remaining", params.tool_calls_remaining)?;
        Ok(Self {
            run_id: params.run_id,
            experiment_id: params.experiment_id,
            baseline_revision,
            workspace_snapshot,
            node_id: params.node_id,
            parent_node_id: params.parent_node_id,
            role: params.role,
            capabilities: params.capabilities,
            input_digest,
            dependency_ids: params.dependency_ids,
            generation: params.generation,
            attempt: params.attempt,
            state: NodeState::Proposed,
            deadline_ms: params.deadline_ms,
            cpu_budget_ms: params.cpu_budget_ms,
            memory_budget_bytes: params.memory_budget_bytes,
            output_bytes_max: params.output_bytes_max,
            tool_calls_remaining: params.tool_calls_remaining,
            result_digest: None,
        })
    }

    fn check_positive(field: &'static str, value: u64) -> Result<(), ExperimentError> {
        if value == 0 {
            return Err(ExperimentError::InvalidBudget { field });
        }
        Ok(())
    }

    /// The run id.
    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }
    /// The experiment id.
    pub fn experiment_id(&self) -> &ExperimentId {
        &self.experiment_id
    }
    /// This node's id.
    pub fn node_id(&self) -> &NodeId {
        &self.node_id
    }
    /// The parent node id, if any.
    pub fn parent_node_id(&self) -> Option<&NodeId> {
        self.parent_node_id.as_ref()
    }
    /// The worker role.
    pub fn role(&self) -> WorkerRole {
        self.role
    }
    /// The delegated capabilities.
    pub fn capabilities(&self) -> &CapabilitySet {
        &self.capabilities
    }
    /// The delegation generation (depth).
    pub fn generation(&self) -> u64 {
        self.generation
    }
    /// The attempt number.
    pub fn attempt(&self) -> u32 {
        self.attempt
    }
    /// The current state.
    pub fn state(&self) -> NodeState {
        self.state
    }
    /// The published result digest, if any.
    pub fn result_digest(&self) -> Option<&str> {
        self.result_digest.as_deref()
    }
    /// The absolute deadline in milliseconds.
    pub fn deadline_ms(&self) -> u64 {
        self.deadline_ms
    }
    /// The pinned baseline revision the node was prepared from.
    pub fn baseline_revision(&self) -> &str {
        &self.baseline_revision
    }
    /// The immutable workspace snapshot label.
    pub fn workspace_snapshot(&self) -> &str {
        &self.workspace_snapshot
    }
    /// The digest of the immutable input the node was given.
    pub fn input_digest(&self) -> &str {
        &self.input_digest
    }
    /// Ids of nodes this node depends on.
    pub fn dependency_ids(&self) -> &[NodeId] {
        &self.dependency_ids
    }
    /// The CPU budget in milliseconds.
    pub fn cpu_budget_ms(&self) -> u64 {
        self.cpu_budget_ms
    }
    /// The memory budget in bytes.
    pub fn memory_budget_bytes(&self) -> u64 {
        self.memory_budget_bytes
    }
    /// The maximum output the node may produce, in bytes.
    pub fn output_bytes_max(&self) -> u64 {
        self.output_bytes_max
    }
    /// The remaining tool calls delegated to this node.
    pub fn tool_calls_remaining(&self) -> u64 {
        self.tool_calls_remaining
    }

    /// Moves the node to `state`. Crate-internal: only the [`Scheduler`]
    /// transitions nodes, after validating the move.
    pub(crate) fn set_state(&mut self, state: NodeState) {
        self.state = state;
    }

    /// Records the published result digest. Crate-internal: only the
    /// [`Scheduler`] publishes, at most once per node.
    pub(crate) fn set_result_digest(&mut self, digest: String) {
        self.result_digest = Some(digest);
    }
}

// ---------------------------------------------------------------------------
// Scheduler limits
// ---------------------------------------------------------------------------

/// Operator-set bounds for one scheduler. All values are validated positive
/// by [`Scheduler::new`]; the defaults are the plan's initial experimental
/// limits, not universal tuning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedulerLimits {
    /// Maximum concurrent workers.
    pub workers_max: u64,
    /// Maximum queued (admitted, not yet running) nodes.
    pub queue_capacity: usize,
    /// Maximum children admitted per task.
    pub children_per_task_max: u64,
    /// Maximum delegation depth (generations).
    pub depth_max: u64,
    /// Per-task deadline, in milliseconds.
    pub task_deadline_ms: u64,
    /// Aggregate tool-call budget for the run.
    pub aggregate_tool_calls_max: u64,
    /// Aggregate output budget for the run, in bytes.
    pub aggregate_output_bytes_max: u64,
}

impl Default for SchedulerLimits {
    /// The plan's initial experimental limits.
    fn default() -> Self {
        Self {
            workers_max: WORKERS_MAX_DEFAULT,
            queue_capacity: QUEUE_CAPACITY_DEFAULT,
            children_per_task_max: CHILDREN_PER_TASK_MAX_DEFAULT,
            depth_max: DEPTH_MAX_DEFAULT,
            task_deadline_ms: TASK_DEADLINE_MS_DEFAULT,
            aggregate_tool_calls_max: AGGREGATE_TOOL_CALLS_MAX_DEFAULT,
            aggregate_output_bytes_max: AGGREGATE_OUTPUT_BYTES_MAX_DEFAULT,
        }
    }
}

impl SchedulerLimits {
    /// Rejects any zero bound. Called by [`Scheduler::new`].
    pub fn validate(&self) -> Result<(), ExperimentError> {
        let fields: &[(&'static str, u64)] = &[
            ("workers_max", self.workers_max),
            ("queue_capacity", self.queue_capacity as u64),
            ("children_per_task_max", self.children_per_task_max),
            ("depth_max", self.depth_max),
            ("task_deadline_ms", self.task_deadline_ms),
            ("aggregate_tool_calls_max", self.aggregate_tool_calls_max),
            (
                "aggregate_output_bytes_max",
                self.aggregate_output_bytes_max,
            ),
        ];
        for (field, value) in fields {
            if *value == 0 {
                return Err(ExperimentError::InvalidBudget { field });
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Scheduler: pure scheduling logic (no concurrency)
// ---------------------------------------------------------------------------

/// A published result: the generation it was produced for and its digest.
#[derive(Debug, Clone)]
struct PublishedResult {
    generation: u64,
    result_digest: String,
}

/// The host-owned scheduler: pure logic over admitted nodes.
///
/// This is the invariant core a future asynchronous executor must preserve;
/// it spawns nothing, runs nothing, and holds no threads. It enforces:
/// bounded admission (queue-full rejects explicitly), generation-checked
/// at-most-once publication, and terminal cancellation that blocks later
/// publication.
#[derive(Debug)]
pub struct Scheduler {
    limits: SchedulerLimits,
    nodes: BTreeMap<NodeId, SchedulerNode>,
    queue: VecDeque<NodeId>,
    published: BTreeMap<NodeId, PublishedResult>,
    cancelled_runs: Vec<RunId>,
}

impl Scheduler {
    /// Builds a scheduler for `limits`; rejects zero bounds.
    pub fn new(limits: SchedulerLimits) -> Result<Self, ExperimentError> {
        limits.validate()?;
        Ok(Self {
            limits,
            nodes: BTreeMap::new(),
            queue: VecDeque::new(),
            published: BTreeMap::new(),
            cancelled_runs: Vec::new(),
        })
    }

    /// The configured limits.
    pub fn limits(&self) -> &SchedulerLimits {
        &self.limits
    }

    /// Number of admitted nodes waiting in the queue.
    pub fn queue_len(&self) -> usize {
        self.queue.len()
    }

    /// Number of published results.
    pub fn published_count(&self) -> usize {
        self.published.len()
    }

    /// The digest published for `id`, if a result was published.
    pub fn published_digest(&self, id: &NodeId) -> Option<&str> {
        self.published
            .get(id)
            .map(|result| result.result_digest.as_str())
    }

    /// The generation a published result was produced for, if any.
    pub fn published_generation(&self, id: &NodeId) -> Option<u64> {
        self.published.get(id).map(|result| result.generation)
    }

    /// Looks up an admitted node.
    pub fn node(&self, id: &NodeId) -> Option<&SchedulerNode> {
        self.nodes.get(id)
    }

    /// True when the run was cancelled; late results are rejected.
    pub fn is_run_cancelled(&self, run_id: &RunId) -> bool {
        self.cancelled_runs.contains(run_id)
    }

    /// Admits a proposed node into the queue.
    ///
    /// Accepted: a node in [`NodeState::Proposed`] whose generation fits
    /// the depth bound, while the queue has room. Rejected:
    /// [`ExperimentError::QueueFull`], [`ExperimentError::DepthExceeded`],
    /// [`ExperimentError::DuplicateNode`],
    /// [`ExperimentError::RunCancelled`], or a node that is not Proposed
    /// ([`ExperimentError::BadTransition`]).
    pub fn admit(&mut self, node: SchedulerNode) -> Result<(), ExperimentError> {
        if node.state() != NodeState::Proposed {
            return Err(ExperimentError::BadTransition {
                from: node.state().name(),
                event: "admit",
            });
        }
        if node.generation() > self.limits.depth_max {
            return Err(ExperimentError::DepthExceeded {
                max: self.limits.depth_max,
            });
        }
        if self.queue.len() >= self.limits.queue_capacity {
            return Err(ExperimentError::QueueFull {
                capacity: self.limits.queue_capacity,
            });
        }
        if self.nodes.contains_key(node.node_id()) {
            return Err(ExperimentError::DuplicateNode {
                id: node.node_id().as_str().to_string(),
            });
        }
        if self.is_run_cancelled(node.run_id()) {
            return Err(ExperimentError::RunCancelled {
                run: node.run_id().as_str().to_string(),
            });
        }
        let id = node.node_id().clone();
        let mut admitted = node;
        admitted.set_state(NodeState::Admitted);
        self.nodes.insert(id.clone(), admitted);
        self.queue.push_back(id);
        Ok(())
    }

    /// Publishes a terminal result for an admitted node — at most once.
    ///
    /// Accepted: the node's current generation, a non-empty digest, and a
    /// terminal state. Rejected, in check order: non-terminal state
    /// ([`ExperimentError::NotTerminal`]), unknown node
    /// ([`ExperimentError::UnknownNode`]), cancelled run
    /// ([`ExperimentError::RunCancelled`]), generation mismatch
    /// ([`ExperimentError::StaleGeneration`]), second publication
    /// ([`ExperimentError::DuplicateResult`]), or an already-terminal node
    /// ([`ExperimentError::BadTransition`]). A cancelled, stale, timed-out,
    /// or superseded result never publishes.
    pub fn publish_result(
        &mut self,
        node_id: &NodeId,
        generation: u64,
        result_digest: &str,
        terminal: NodeState,
    ) -> Result<(), ExperimentError> {
        if !terminal.is_terminal() {
            return Err(ExperimentError::NotTerminal {
                state: terminal.name(),
            });
        }
        let run_id: RunId = {
            let node = self
                .nodes
                .get(node_id)
                .ok_or_else(|| ExperimentError::UnknownNode {
                    id: node_id.as_str().to_string(),
                })?;
            node.run_id().clone()
        };
        if self.is_run_cancelled(&run_id) {
            return Err(ExperimentError::RunCancelled {
                run: run_id.as_str().to_string(),
            });
        }
        let node = self
            .nodes
            .get_mut(node_id)
            .ok_or_else(|| ExperimentError::UnknownNode {
                id: node_id.as_str().to_string(),
            })?;
        if generation != node.generation() {
            return Err(ExperimentError::StaleGeneration {
                node: node_id.as_str().to_string(),
                expected: node.generation(),
                got: generation,
            });
        }
        if self.published.contains_key(node_id) {
            return Err(ExperimentError::DuplicateResult {
                node: node_id.as_str().to_string(),
            });
        }
        if node.state().is_terminal() {
            return Err(ExperimentError::BadTransition {
                from: node.state().name(),
                event: "publish_result",
            });
        }
        let digest = check_digest("result_digest", result_digest)?;
        node.set_state(terminal);
        node.set_result_digest(digest.clone());
        self.published.insert(
            node_id.clone(),
            PublishedResult {
                generation,
                result_digest: digest,
            },
        );
        self.queue.retain(|queued| queued != node_id);
        Ok(())
    }

    /// Cancels a run: every non-terminal node moves to
    /// [`NodeState::Cancelled`], the queue is drained of its nodes, and the
    /// run id is retained (bounded) so late results are rejected.
    ///
    /// Returns the number of nodes transitioned to Cancelled.
    pub fn cancel_run(&mut self, run_id: &RunId) -> u64 {
        let mut transitioned: u64 = 0;
        for node in self.nodes.values_mut() {
            if node.run_id() == run_id && !node.state().is_terminal() {
                node.set_state(NodeState::Cancelled);
                transitioned += 1;
            }
        }
        let cancelled: Vec<NodeId> = self
            .queue
            .iter()
            .filter(|id| {
                self.nodes
                    .get(*id)
                    .is_some_and(|node| node.run_id() == run_id)
            })
            .cloned()
            .collect();
        for id in cancelled {
            self.queue.retain(|queued| queued != &id);
        }
        if !self.is_run_cancelled(run_id) && self.cancelled_runs.len() < CANCELLED_RUNS_MAX {
            self.cancelled_runs.push(run_id.clone());
        }
        transitioned
    }
}
