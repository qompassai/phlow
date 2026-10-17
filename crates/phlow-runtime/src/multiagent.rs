//! Concurrent multi-agent sessions over the shared runtime: many
//! agents, many unique tasks, one coordination board, one gated
//! tool dispatcher.
//!
//! Plain words: an [`Orchestrator`] runs a set of agent *sessions*
//! concurrently — one thread per session — where each session is a
//! small step loop driven by a [`SessionAgent`]. Three shared
//! structures keep the concurrency honest:
//!
//! - **The attention board** (`phlow-attention`): every session
//!   derives and publishes its focus from the run's shared
//!   [`Snapshot`] at every step boundary. When the coordination
//!   rule says a partner holds the session's top target, the
//!   session either *switches* to its next target — an explicit
//!   [`Event::FocusSwitched`] in its trajectory log — or ends
//!   `Deferred`. Two sessions never unknowingly work the same top
//!   target.
//! - **The flow gate** (`phlow-flowguard`): every session carries
//!   its own trajectory; every tool call and every message is
//!   proposed to the gate and decided from the log alone. Blocks run
//!   the configured remedies (sanitize, authority approval,
//!   isolation in a disposable child) and end the session `Blocked`
//!   when no remedy applies.
//! - **The single dispatcher thread**: all tool execution funnels
//!   through one thread that owns the [`ToolDispatcher`], which is
//!   what preserves the Runtime's single-writer contract no matter
//!   how many sessions run. The dispatcher *re-runs the gate* over
//!   the session's log snapshot before executing — a session cannot
//!   talk its way past the check, because the check happens again at
//!   the point of execution.
//!
//! Sub-agents are sessions too: a `Delegate` step spawns a child
//! whose trajectory starts at the parent's label joined with the
//! requested delta — never weaker than the parent — and whose end
//! label joins back into the parent when its result is accepted.
//! External destinations are reached through an [`ExternalSender`]
//! only after the gate allows the send to the policy destination of
//! that name; `agent:<id>` destinations deliver to a live session's
//! inbox, carrying the sender's label snapshot with them (which is
//! what makes cross-agent laundering fail: the receiver's label
//! narrows on receipt).
//!
//! Bounds: [`SESSIONS_MAX`] sessions per run, [`STEPS_MAX`] steps
//! per session, [`DELEGATE_DEPTH_MAX`] delegation depth,
//! [`CHILDREN_MAX`] children per session. Everything else is the
//! flowguard and attention crates' own bounds.
//!
//! [`Event::FocusSwitched`]: phlow_flowguard::event::Event::FocusSwitched

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::{Arc, Mutex, mpsc};

use phlow_attention::{Decision as AttentionDecision, Focus, FocusBoard, Snapshot, TargetId};
use phlow_editor::EditorTransport;
use phlow_flowguard::event::{Event, JoinSpec, ProposedAction};
use phlow_flowguard::gate::{BlockInfo, Decision, RemedyOption, apply_sanitizer};
use phlow_flowguard::label::{LabelDelta, LabelSnapshot};
use phlow_flowguard::{Policy, Trajectory};
use phlow_llm::LlmTransport;
use serde_json::{Value, json};

use crate::runtime::Runtime;

/// Most sessions in one orchestrated run.
pub const SESSIONS_MAX: usize = 32;

/// Most steps in one session (bounds every loop in this module).
pub const STEPS_MAX: usize = 256;

/// Deepest sub-agent delegation chain.
pub const DELEGATE_DEPTH_MAX: usize = 4;

/// Most children one session may spawn.
pub const CHILDREN_MAX: usize = 16;

/// One action a session agent chooses at a step boundary.
#[derive(Clone, Debug)]
pub enum Step {
    /// Call a tool (gated, then dispatched).
    ToolCall {
        /// Tool name.
        tool: String,
        /// Call arguments.
        args: Value,
    },
    /// Spawn a sub-agent for `task`, with an optional additional
    /// label restriction for the child.
    Delegate {
        /// The child's task text.
        task: String,
        /// Extra restriction joined into the child's start label.
        delta: Option<LabelDelta>,
    },
    /// Send a message to a policy destination (`agent:<id>` for a
    /// live session, anything else via the external sender).
    SendMessage {
        /// Destination name.
        destination: String,
        /// Message text.
        text: String,
    },
    /// Do nothing this step (waiting on inbox or attention).
    Wait,
    /// Finish the session.
    Done {
        /// The session's closing summary.
        summary: String,
    },
}

/// What a session agent sees at a step boundary.
#[derive(Clone, Debug)]
pub struct StepCx {
    /// This session's agent id.
    pub agent_id: String,
    /// The session's task text.
    pub task: String,
    /// The result of the previous step, if any (a tool's return
    /// value, a delivery acknowledgement, or a child's summary).
    pub last_result: Option<Value>,
    /// Messages delivered since the previous step. Their senders'
    /// labels have already joined this trajectory — reading them is
    /// not optional bookkeeping, it is the label doing its work.
    pub inbox: Vec<InboxItem>,
    /// Zero-based step counter.
    pub step_index: usize,
    /// The session's current planned focus targets, most preferred
    /// first, after any attention-driven switching.
    pub focus_targets: Vec<String>,
}

/// One delivered inter-agent message.
#[derive(Clone, Debug)]
pub struct InboxItem {
    /// Sender agent id.
    pub from: String,
    /// Message text.
    pub text: String,
}

/// A session's behaviour. Implementations are the agent logic
/// (planner-driven in production, scripted in tests); the
/// orchestrator owns everything around the loop.
pub trait SessionAgent: Send {
    /// Choose the next action.
    fn step(&mut self, cx: &StepCx) -> Step;
}

/// Produces agents for delegated tasks.
pub trait AgentFactory: Send + Sync {
    /// Build the agent for one delegated task.
    fn spawn(&self, task: &str) -> Box<dyn SessionAgent>;
}

/// A factory backed by a closure.
pub struct ClosureFactory<F: Fn(&str) -> Box<dyn SessionAgent> + Send + Sync>(pub F);

impl<F: Fn(&str) -> Box<dyn SessionAgent> + Send + Sync> AgentFactory for ClosureFactory<F> {
    fn spawn(&self, task: &str) -> Box<dyn SessionAgent> {
        (self.0)(task)
    }
}

/// Executes one tool call. The production implementation is
/// [`RuntimeToolDispatcher`]; the trait is the seam that keeps the
/// orchestrator testable and the Runtime's single-writer rule in
/// exactly one place (the dispatcher thread).
pub trait ToolDispatcher: Send {
    /// Execute `tool` with `args` on behalf of `agent_id`.
    fn dispatch(&mut self, agent_id: &str, tool: &str, args: &Value) -> Value;
}

/// The production dispatcher: forwards to
/// [`Runtime::call_tool`], so the role gates, the tool schemas, and
/// every other runtime contract apply unchanged — the flow gate is
/// additive, never a replacement. The `role` is fixed per
/// dispatcher: an orchestrated run dispatches under one role,
/// exactly as a CLI/TUI/MCP run does.
pub struct RuntimeToolDispatcher<L: LlmTransport, E: EditorTransport> {
    runtime: Runtime<L, E>,
    role: String,
}

impl<L: LlmTransport + Send, E: EditorTransport + Send> RuntimeToolDispatcher<L, E> {
    /// Wrap a runtime, dispatching under `role` (must be one of the
    /// runtime's roles; `call_tool` enforces that).
    pub fn new(runtime: Runtime<L, E>, role: &str) -> RuntimeToolDispatcher<L, E> {
        RuntimeToolDispatcher {
            runtime,
            role: role.to_string(),
        }
    }
}

impl<L: LlmTransport + Send, E: EditorTransport + Send> ToolDispatcher
    for RuntimeToolDispatcher<L, E>
{
    fn dispatch(&mut self, _agent_id: &str, tool: &str, args: &Value) -> Value {
        self.runtime.call_tool(tool, args, &self.role)
    }
}

/// One approval question for the authority callback.
#[derive(Clone, Debug)]
pub struct ApprovalRequest {
    /// The session asking.
    pub agent_id: String,
    /// Digest of the blocked action.
    pub digest: String,
    /// Human-readable summary of the action.
    pub summary: String,
    /// Attention marks the approval would clear.
    pub marks: Vec<String>,
}

/// The authority behind the approval remedy (in phlow, the
/// operator, reached through whatever UI the embedder drives).
pub trait AuthorityCallback: Send {
    /// Approve this one action? Approval is consumed by the gated
    /// execution; it never lifts the trajectory's label.
    fn approve(&mut self, request: &ApprovalRequest) -> bool;
}

/// Delivery to destinations outside this process (external agents).
/// The phlow-a2a registry is the production implementation shape;
/// the trait keeps the runtime crate free of a protocol dependency.
pub trait ExternalSender: Send {
    /// Send `text` to the policy destination named `destination`
    /// (gate-approved by the caller before this is invoked).
    fn send(&mut self, destination: &str, text: &str) -> Result<Value, String>;
}

/// Which remedies the orchestrator attempts on a gate block, in
/// the fixed order sanitize → authority → isolate.
#[derive(Clone, Copy, Debug, Default)]
pub struct RemedyPolicy {
    /// Try applicable sanitizers.
    pub sanitize: bool,
    /// Ask the authority callback.
    pub authority: bool,
    /// Isolate the step in a disposable child.
    pub isolate: bool,
}

impl RemedyPolicy {
    /// No remedies: a block ends the session.
    pub fn none() -> RemedyPolicy {
        RemedyPolicy::default()
    }

    /// All remedies the gate offers.
    pub fn all() -> RemedyPolicy {
        RemedyPolicy {
            sanitize: true,
            authority: true,
            isolate: true,
        }
    }
}

/// One session to run.
#[derive(Clone, Debug)]
pub struct SessionSpec {
    /// Unique agent id for the run.
    pub agent_id: String,
    /// The task text.
    pub task: String,
    /// Planned attention targets, most preferred first.
    pub targets: Vec<String>,
    /// Classification the task arrives with (e.g. from an A2A
    /// caller's declared source delta): joined into the
    /// trajectory before the first step.
    pub start_delta: Option<LabelDelta>,
}

/// How a session ended.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionStatus {
    /// The agent finished its task.
    Completed,
    /// Attention contention with no alternate target.
    Deferred,
    /// The flow gate blocked an action and no remedy applied.
    Blocked,
    /// A bound was hit, an agent panicked, or setup failed.
    Failed,
}

/// One session's outcome.
#[derive(Clone, Debug)]
pub struct SessionReport {
    /// The session's agent id.
    pub agent_id: String,
    /// How it ended.
    pub status: SessionStatus,
    /// The agent's closing summary (or the failure detail).
    pub summary: String,
    /// Steps actually executed.
    pub steps: usize,
    /// Attention switches, as (from, to, because) triples.
    pub switches: Vec<(String, String, Option<String>)>,
    /// Tool return values, in execution order.
    pub tool_results: Vec<Value>,
    /// The trajectory's final label, when it could be folded.
    pub final_label: Option<LabelSnapshot>,
    /// The full trajectory event log (the audit record).
    pub events: Vec<Event>,
}

/// The orchestrator: one policy, one snapshot frame, one board.
pub struct Orchestrator {
    policy: Arc<Policy>,
    snapshot: Arc<Snapshot>,
    board: Arc<Mutex<FocusBoard>>,
    remedies: RemedyPolicy,
    accept_isolated_results: bool,
}

impl Orchestrator {
    /// Build an orchestrator. `accept_isolated_results` decides
    /// whether a value produced by an isolation child is accepted
    /// into the parent (joining the child's end label) or
    /// quarantined (the parent learns only that the isolated step
    /// ran).
    pub fn new(
        policy: Arc<Policy>,
        snapshot: Arc<Snapshot>,
        board: Arc<Mutex<FocusBoard>>,
        remedies: RemedyPolicy,
        accept_isolated_results: bool,
    ) -> Orchestrator {
        Orchestrator {
            policy,
            snapshot,
            board,
            remedies,
            accept_isolated_results,
        }
    }

    /// Run all sessions concurrently and return their reports in
    /// the order the specs were given. Sessions beyond
    /// [`SESSIONS_MAX`] are reported `Failed` without running.
    pub fn run(
        &self,
        factory: &dyn AgentFactory,
        sessions: Vec<(SessionSpec, Box<dyn SessionAgent>)>,
        dispatcher: Box<dyn ToolDispatcher>,
        authority: Option<Box<dyn AuthorityCallback>>,
        external: Option<Box<dyn ExternalSender>>,
    ) -> Vec<SessionReport> {
        let (service_tx, service_rx) = mpsc::channel::<ServiceRequest>();
        let inboxes: Arc<Mutex<InboxMap>> = Arc::new(Mutex::new(HashMap::new()));
        let shared = Arc::new(RunShared {
            policy: self.policy.clone(),
            snapshot: self.snapshot.clone(),
            board: self.board.clone(),
            remedies: self.remedies,
            accept_isolated_results: self.accept_isolated_results,
            service: ServiceHandle {
                tx: service_tx.clone(),
            },
            inboxes,
        });
        let policy_for_service = self.policy.clone();
        let service_join = std::thread::spawn(move || {
            service_loop(
                service_rx,
                dispatcher,
                authority,
                external,
                policy_for_service,
            );
        });

        let mut reports: Vec<Option<SessionReport>> = Vec::new();
        reports.resize_with(sessions.len(), || None);
        let (report_tx, report_rx) = mpsc::channel::<(usize, SessionReport)>();
        std::thread::scope(|scope| {
            for (index, (spec, agent)) in sessions.into_iter().enumerate() {
                if index >= SESSIONS_MAX {
                    let _ = report_tx.send((
                        index,
                        SessionReport {
                            agent_id: spec.agent_id.clone(),
                            status: SessionStatus::Failed,
                            summary: format!("session bound {SESSIONS_MAX} reached"),
                            steps: 0,
                            switches: Vec::new(),
                            tool_results: Vec::new(),
                            final_label: None,
                            events: Vec::new(),
                        },
                    ));
                    continue;
                }
                if let Ok(mut inboxes) = shared.inboxes.lock() {
                    inboxes.entry(spec.agent_id.clone()).or_default();
                }
                let run_shared = shared.clone();
                let run_report_tx = report_tx.clone();
                scope.spawn(move || {
                    let report = run_top_session(&run_shared, factory, spec, agent);
                    let _ = run_report_tx.send((index, report));
                });
            }
            drop(report_tx);
            for (index, report) in report_rx {
                reports[index] = Some(report);
            }
        });
        // The service thread exits when its channel closes, and a
        // sender clone lives inside `shared` — drop both before
        // joining, or the join waits forever.
        drop(shared);
        drop(service_tx);
        let _ = service_join.join();
        reports
            .into_iter()
            .map(|report| report.expect("every session reports"))
            .collect()
    }
}

// ---------------------------------------------------------------------
// The service thread: sole owner of the dispatcher, the authority
// callback, and the external sender.

enum ServiceRequest {
    Dispatch {
        agent_id: String,
        tool: String,
        args: Value,
        log: Vec<Event>,
        reply: mpsc::Sender<Value>,
    },
    Approve {
        request: ApprovalRequest,
        reply: mpsc::Sender<bool>,
    },
    ExternalSend {
        destination: String,
        text: String,
        reply: mpsc::Sender<Value>,
    },
}

/// Inter-agent inboxes: agent id -> queued deliveries, each
/// (sender id, sender label snapshot, text).
type InboxMap = HashMap<String, Vec<(String, LabelSnapshot, String)>>;

#[derive(Clone)]
struct ServiceHandle {
    tx: mpsc::Sender<ServiceRequest>,
}

impl ServiceHandle {
    fn dispatch(&self, agent_id: &str, tool: &str, args: &Value, log: Vec<Event>) -> Value {
        let (reply_tx, reply_rx) = mpsc::channel();
        if self
            .tx
            .send(ServiceRequest::Dispatch {
                agent_id: agent_id.to_string(),
                tool: tool.to_string(),
                args: args.clone(),
                log,
                reply: reply_tx,
            })
            .is_err()
        {
            return json!({"error": "dispatcher service unavailable"});
        }
        reply_rx
            .recv()
            .unwrap_or_else(|_| json!({"error": "dispatcher service dropped the request"}))
    }

    fn approve(&self, request: ApprovalRequest) -> bool {
        let (reply_tx, reply_rx) = mpsc::channel();
        if self
            .tx
            .send(ServiceRequest::Approve {
                request,
                reply: reply_tx,
            })
            .is_err()
        {
            return false;
        }
        reply_rx.recv().unwrap_or(false)
    }

    fn external_send(&self, destination: &str, text: &str) -> Value {
        let (reply_tx, reply_rx) = mpsc::channel();
        if self
            .tx
            .send(ServiceRequest::ExternalSend {
                destination: destination.to_string(),
                text: text.to_string(),
                reply: reply_tx,
            })
            .is_err()
        {
            return json!({"error": "external sender service unavailable"});
        }
        reply_rx
            .recv()
            .unwrap_or_else(|_| json!({"error": "external sender dropped the request"}))
    }
}

fn service_loop(
    rx: mpsc::Receiver<ServiceRequest>,
    mut dispatcher: Box<dyn ToolDispatcher>,
    mut authority: Option<Box<dyn AuthorityCallback>>,
    mut external: Option<Box<dyn ExternalSender>>,
    policy: Arc<Policy>,
) {
    for request in rx {
        match request {
            ServiceRequest::Dispatch {
                agent_id,
                tool,
                args,
                log,
                reply,
            } => {
                // The dispatch-point re-check: the decision is made
                // again, here, over the session's recorded log —
                // execution never trusts that a check happened
                // somewhere else.
                let action = ProposedAction::ToolCall {
                    tool: tool.clone(),
                    args: args.clone(),
                };
                let value = match phlow_flowguard::decide(&policy, &log, &action) {
                    Decision::Allow(_) => dispatcher.dispatch(&agent_id, &tool, &args),
                    Decision::Block(info) => json!({
                        "error": "flow gate blocked at dispatch",
                        "reasons": format!("{:?}", info.reasons),
                    }),
                };
                let _ = reply.send(value);
            }
            ServiceRequest::Approve { request, reply } => {
                let approved = authority
                    .as_mut()
                    .map(|callback| callback.approve(&request))
                    .unwrap_or(false);
                let _ = reply.send(approved);
            }
            ServiceRequest::ExternalSend {
                destination,
                text,
                reply,
            } => {
                let value = match external.as_mut() {
                    Some(sender) => sender
                        .send(&destination, &text)
                        .unwrap_or_else(|err| json!({"error": err})),
                    None => json!({"error": "no external sender configured"}),
                };
                let _ = reply.send(value);
            }
        }
    }
}

// ---------------------------------------------------------------------
// Session execution.

struct RunShared {
    policy: Arc<Policy>,
    snapshot: Arc<Snapshot>,
    board: Arc<Mutex<FocusBoard>>,
    remedies: RemedyPolicy,
    accept_isolated_results: bool,
    service: ServiceHandle,
    inboxes: Arc<Mutex<InboxMap>>,
}

struct RunState {
    agent_id: String,
    task: String,
    trajectory: Trajectory,
    depth: usize,
    seq: u64,
    remaining: Vec<TargetId>,
    last_result: Option<Value>,
    tool_results: Vec<Value>,
    switches: Vec<(String, String, Option<String>)>,
    children: usize,
    steps: usize,
}

fn run_top_session(
    shared: &Arc<RunShared>,
    factory: &dyn AgentFactory,
    spec: SessionSpec,
    agent: Box<dyn SessionAgent>,
) -> SessionReport {
    let mut trajectory = Trajectory::new(shared.policy.clone());
    if let Some(delta) = &spec.start_delta {
        let _ = trajectory.record_message_received("tasking", JoinSpec::Delta(delta.clone()));
    }
    let remaining = match parse_targets(&spec.targets) {
        Ok(targets) => targets,
        Err(detail) => {
            return SessionReport {
                agent_id: spec.agent_id.clone(),
                status: SessionStatus::Failed,
                summary: detail,
                steps: 0,
                switches: Vec::new(),
                tool_results: Vec::new(),
                final_label: None,
                events: trajectory.events().to_vec(),
            };
        }
    };
    let state = RunState {
        agent_id: spec.agent_id.clone(),
        task: spec.task.clone(),
        trajectory,
        depth: 0,
        seq: 0,
        remaining,
        last_result: None,
        tool_results: Vec::new(),
        switches: Vec::new(),
        children: 0,
        steps: 0,
    };
    run_session(shared, factory, state, agent)
}

fn parse_targets(targets: &[String]) -> Result<Vec<TargetId>, String> {
    let mut parsed = Vec::with_capacity(targets.len());
    for target in targets {
        parsed.push(
            TargetId::new(target).map_err(|err| format!("invalid target {target:?}: {err}"))?,
        );
    }
    Ok(parsed)
}

enum AttentionOutcome {
    Proceed,
    Deferred { holder: String, target: String },
    Failed(String),
}

/// One attention consultation at a step boundary: derive and
/// publish this session's focus, then apply the coordination rule.
/// Contention on the top target with an alternate available is a
/// *switch* (recorded); contention with no alternate is a deferral.
fn attend(shared: &RunShared, state: &mut RunState) -> AttentionOutcome {
    let mut pending_switch: Option<(TargetId, String)> = None;
    loop {
        if state.remaining.is_empty() {
            return AttentionOutcome::Proceed;
        }
        state.seq += 1;
        let filter: BTreeSet<TargetId> = state.remaining.iter().cloned().collect();
        let focus = match Focus::derive(&state.agent_id, &shared.snapshot, state.seq, Some(&filter))
        {
            Ok(focus) => focus,
            Err(err) => {
                return AttentionOutcome::Failed(format!("focus derive failed: {err}"));
            }
        };
        if focus.targets.is_empty() {
            return AttentionOutcome::Proceed;
        }
        {
            let Ok(mut board) = shared.board.lock() else {
                return AttentionOutcome::Failed("attention board lock poisoned".to_string());
            };
            if let Err(err) = board.publish(&state.agent_id, focus.clone()) {
                return AttentionOutcome::Failed(format!("focus publish failed: {err}"));
            }
        }
        let _ = state
            .trajectory
            .record(Event::FocusPublished { seq: state.seq });
        if let Some((from, holder)) = pending_switch.take() {
            let to = focus
                .targets
                .first()
                .map(ToString::to_string)
                .unwrap_or_default();
            let _ = state.trajectory.record(Event::FocusSwitched {
                from: from.to_string(),
                to: to.clone(),
                because: Some(holder.clone()),
            });
            state.switches.push((from.to_string(), to, Some(holder)));
        }
        let Some(top) = focus.targets.first().cloned() else {
            return AttentionOutcome::Proceed;
        };
        let contention = {
            let Ok(board) = shared.board.lock() else {
                return AttentionOutcome::Failed("attention board lock poisoned".to_string());
            };
            let mut found = None;
            for partner in board.agent_ids() {
                if partner == state.agent_id {
                    continue;
                }
                if let Some(AttentionDecision::Serialize { holder, target }) =
                    board.decide_between(&state.agent_id, &partner)
                    && holder != state.agent_id
                    && target == top
                {
                    found = Some((holder, target));
                    break;
                }
            }
            found
        };
        match contention {
            None => return AttentionOutcome::Proceed,
            Some((holder, target)) => {
                if state.remaining.len() > 1 {
                    state.remaining.retain(|held| held != &target);
                    pending_switch = Some((target, holder));
                } else {
                    return AttentionOutcome::Deferred {
                        holder,
                        target: target.to_string(),
                    };
                }
            }
        }
    }
}

fn run_session(
    shared: &Arc<RunShared>,
    factory: &dyn AgentFactory,
    mut state: RunState,
    mut agent: Box<dyn SessionAgent>,
) -> SessionReport {
    let mut summary = String::new();
    let mut status = SessionStatus::Failed;
    for step_index in 0..STEPS_MAX {
        state.steps = step_index + 1;
        // Inbox first: receipts join their labels before anything
        // else this step can do.
        let inbox = drain_inbox(shared, &mut state);
        match attend(shared, &mut state) {
            AttentionOutcome::Proceed => {}
            AttentionOutcome::Deferred { holder, target } => {
                status = SessionStatus::Deferred;
                summary = format!("top target {target} is held by {holder}; no alternate target");
                break;
            }
            AttentionOutcome::Failed(detail) => {
                status = SessionStatus::Failed;
                summary = detail;
                break;
            }
        }
        let cx = StepCx {
            agent_id: state.agent_id.clone(),
            task: state.task.clone(),
            last_result: state.last_result.clone(),
            inbox,
            step_index,
            focus_targets: state.remaining.iter().map(ToString::to_string).collect(),
        };
        let step = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| agent.step(&cx)))
        {
            Ok(step) => step,
            Err(_) => {
                status = SessionStatus::Failed;
                summary = "agent panicked".to_string();
                break;
            }
        };
        match step {
            Step::Done { summary: done } => {
                status = SessionStatus::Completed;
                summary = done;
                break;
            }
            Step::Wait => {}
            Step::ToolCall { tool, args } => {
                match gated_tool_call(shared, factory, &mut state, &tool, &args) {
                    ToolOutcome::Value(value) => {
                        state.tool_results.push(value.clone());
                        state.last_result = Some(value);
                    }
                    ToolOutcome::Blocked(info) => {
                        status = SessionStatus::Blocked;
                        summary = format!("gate blocked {tool}: {:?}", info.reasons);
                        break;
                    }
                    ToolOutcome::Failed(detail) => {
                        status = SessionStatus::Failed;
                        summary = detail;
                        break;
                    }
                }
            }
            Step::SendMessage { destination, text } => {
                match gated_send(shared, factory, &mut state, &destination, &text) {
                    Ok(value) => state.last_result = Some(value),
                    Err(info) => {
                        status = SessionStatus::Blocked;
                        summary = format!("gate blocked send to {destination}: {:?}", info.reasons);
                        break;
                    }
                }
            }
            Step::Delegate { task, delta } => {
                let value = delegate(shared, factory, &mut state, &task, delta);
                state.last_result = Some(value);
            }
        }
        if step_index + 1 == STEPS_MAX {
            status = SessionStatus::Failed;
            summary = format!("step bound {STEPS_MAX} reached");
        }
    }
    SessionReport {
        agent_id: state.agent_id.clone(),
        status,
        summary,
        steps: state.steps,
        switches: state.switches.clone(),
        tool_results: state.tool_results.clone(),
        final_label: state.trajectory.snapshot().ok(),
        events: state.trajectory.events().to_vec(),
    }
}

fn drain_inbox(shared: &RunShared, state: &mut RunState) -> Vec<InboxItem> {
    let drained: Vec<(String, LabelSnapshot, String)> = match shared.inboxes.lock() {
        Ok(mut inboxes) => inboxes
            .get_mut(&state.agent_id)
            .map(std::mem::take)
            .unwrap_or_default(),
        Err(_) => Vec::new(),
    };
    let mut items = Vec::with_capacity(drained.len());
    for (from, snapshot, text) in drained {
        let _ = state
            .trajectory
            .record_message_received(&from, JoinSpec::Snapshot(snapshot));
        items.push(InboxItem { from, text });
    }
    items
}

// ---------------------------------------------------------------------
// Gated actions.

enum ToolOutcome {
    Value(Value),
    Blocked(BlockInfo),
    Failed(String),
}

/// A tool call through the gate, with remedies. The verdict the
/// session records is the verdict that stands; execution happens
/// only via the dispatcher thread, which re-decides.
fn gated_tool_call(
    shared: &Arc<RunShared>,
    factory: &dyn AgentFactory,
    state: &mut RunState,
    tool: &str,
    args: &Value,
) -> ToolOutcome {
    let action = ProposedAction::ToolCall {
        tool: tool.to_string(),
        args: args.clone(),
    };
    let decision = match state.trajectory.propose(&action) {
        Ok(decision) => decision,
        Err(err) => return ToolOutcome::Failed(err.to_string()),
    };
    let info = match decision {
        Decision::Allow(info) => {
            return dispatch_and_record(shared, state, tool, args, &info.digest);
        }
        Decision::Block(info) => info,
    };
    // Remedy 1: sanitize and retry (once).
    if shared.remedies.sanitize
        && let Some((name, would_allow)) = first_sanitize(&info)
    {
        let _ = would_allow;
        if let Ok(sanitized) = apply_sanitizer(&shared.policy, &name, args)
            && let Some(sanitizer) = shared.policy.sanitizer_named(&name)
        {
            let result_delta = sanitizer.result_delta.clone();
            let _ = state.trajectory.record(Event::Sanitized {
                sanitizer: name.clone(),
                result_delta,
            });
            let retry = ProposedAction::ToolCall {
                tool: tool.to_string(),
                args: sanitized.clone(),
            };
            if let Ok(Decision::Allow(allow)) = state.trajectory.propose(&retry) {
                return dispatch_and_record(shared, state, tool, &sanitized, &allow.digest);
            }
        }
    }
    // Remedy 2: authority approval for this one action.
    if shared.remedies.authority
        && let Some((authority, marks)) = first_approval(&info)
    {
        let request = ApprovalRequest {
            agent_id: state.agent_id.clone(),
            digest: info.digest.clone(),
            summary: format!("{tool} {args}"),
            marks: marks.clone(),
        };
        if shared.service.approve(request) {
            let _ = state.trajectory.record(Event::ApprovalGranted {
                authority,
                digest: info.digest.clone(),
                marks,
            });
            if let Ok(Decision::Allow(allow)) = state.trajectory.propose(&action) {
                return dispatch_and_record(shared, state, tool, args, &allow.digest);
            }
        }
    }
    // Remedy 3: isolate the step in a disposable child.
    if shared.remedies.isolate
        && info
            .remedies
            .options
            .iter()
            .any(|option| matches!(option, RemedyOption::Isolate))
    {
        return isolate_tool_call(shared, factory, state, tool, args, info);
    }
    ToolOutcome::Blocked(info)
}

fn dispatch_and_record(
    shared: &Arc<RunShared>,
    state: &mut RunState,
    tool: &str,
    args: &Value,
    digest: &str,
) -> ToolOutcome {
    let log = state.trajectory.events().to_vec();
    let value = shared.service.dispatch(&state.agent_id, tool, args, log);
    if value.get("error").is_some() {
        return ToolOutcome::Failed(format!("dispatch of {tool} refused: {value}"));
    }
    let _ = state.trajectory.record_tool_result(tool, digest);
    ToolOutcome::Value(value)
}

/// Run one blocked tool call in a disposable child trajectory. The
/// child's start label is the parent's current label — isolation
/// quarantines *effects and future contamination*, it does not
/// launder the label, and a call that is blocked for label reasons
/// blocks in the child too (reported honestly).
fn isolate_tool_call(
    shared: &Arc<RunShared>,
    factory: &dyn AgentFactory,
    state: &mut RunState,
    tool: &str,
    args: &Value,
    info: BlockInfo,
) -> ToolOutcome {
    let child_step = Step::ToolCall {
        tool: tool.to_string(),
        args: args.clone(),
    };
    match spawn_child(
        shared,
        factory,
        state,
        "isolated step",
        None,
        Box::new(OneShotAgent {
            step: Some(child_step),
        }),
    ) {
        ChildOutcome::Completed { value, end_label } => {
            if shared.accept_isolated_results {
                let _ = state.trajectory.record(Event::SubagentResult {
                    child: child_name(state),
                    label: end_label,
                });
                ToolOutcome::Value(value.unwrap_or(Value::Null))
            } else {
                ToolOutcome::Value(json!({
                    "isolated": true,
                    "accepted": false,
                    "note": "the isolated step ran in a disposable child; \
                             its result was quarantined, not accepted into this trajectory",
                }))
            }
        }
        ChildOutcome::NotCompleted { .. } => ToolOutcome::Blocked(BlockInfo {
            digest: info.digest.clone(),
            reasons: info.reasons.clone(),
            remedies: info.remedies.clone(),
        }),
        ChildOutcome::Unavailable(detail) => ToolOutcome::Failed(detail),
    }
}

fn child_name(state: &RunState) -> String {
    format!("{}/child-{}", state.agent_id, state.children)
}

enum ChildOutcome {
    Completed {
        value: Option<Value>,
        end_label: LabelSnapshot,
    },
    NotCompleted {
        detail: String,
    },
    Unavailable(String),
}

/// Spawn a child session and run it to completion on this thread.
/// The child's start label is the parent's current label joined
/// with `delta`; the spawn is recorded in the parent, and — for
/// delegation — so is the accepted result (by the caller of
/// [`delegate`], or here for isolation when accepted).
fn spawn_child(
    shared: &Arc<RunShared>,
    factory: &dyn AgentFactory,
    state: &mut RunState,
    task: &str,
    delta: Option<LabelDelta>,
    agent: Box<dyn SessionAgent>,
) -> ChildOutcome {
    let _ = factory;
    if state.depth >= DELEGATE_DEPTH_MAX {
        return ChildOutcome::Unavailable(format!(
            "delegation depth bound {DELEGATE_DEPTH_MAX} reached"
        ));
    }
    if state.children >= CHILDREN_MAX {
        return ChildOutcome::Unavailable(format!("child bound {CHILDREN_MAX} reached"));
    }
    state.children += 1;
    let child_id = child_name(state);
    let start = match state.trajectory.label() {
        Ok(label) => {
            let mut start = label;
            if let Some(delta) = &delta
                && let Ok(delta_label) = shared.policy.resolve_delta(delta)
            {
                start = start.join(&delta_label);
            }
            shared.policy.snapshot_of(&start)
        }
        Err(err) => return ChildOutcome::Unavailable(err.to_string()),
    };
    let _ = state.trajectory.record(Event::SubagentSpawned {
        child: child_id.clone(),
        start: start.clone(),
    });
    if let Ok(mut inboxes) = shared.inboxes.lock() {
        inboxes.entry(child_id.clone()).or_default();
    }
    let child_state = RunState {
        agent_id: child_id.clone(),
        task: task.to_string(),
        trajectory: Trajectory::child(shared.policy.clone(), &state.agent_id, start),
        depth: state.depth + 1,
        seq: 0,
        remaining: state.remaining.clone(),
        last_result: None,
        tool_results: Vec::new(),
        switches: Vec::new(),
        children: 0,
        steps: 0,
    };
    let report = run_session(shared, factory, child_state, agent);
    match report.status {
        SessionStatus::Completed => ChildOutcome::Completed {
            value: report.tool_results.into_iter().last(),
            end_label: match report.final_label {
                Some(label) => label,
                None => return ChildOutcome::Unavailable("child label unavailable".to_string()),
            },
        },
        other => ChildOutcome::NotCompleted {
            detail: format!("child ended {other:?}: {}", report.summary),
        },
    }
}

/// A `Delegate` step: spawn, run, and accept the result — the
/// child's end label joins the parent's trajectory with the result,
/// which is exactly what stops delegated laundering.
fn delegate(
    shared: &Arc<RunShared>,
    factory: &dyn AgentFactory,
    state: &mut RunState,
    task: &str,
    delta: Option<LabelDelta>,
) -> Value {
    let child_agent = factory.spawn(task);
    let child_id = format!("{}/child-{}", state.agent_id, state.children + 1);
    match spawn_child(shared, factory, state, task, delta, child_agent) {
        ChildOutcome::Completed { value, end_label } => {
            let _ = state.trajectory.record(Event::SubagentResult {
                child: child_id.clone(),
                label: end_label,
            });
            json!({
                "child": child_id,
                "status": "completed",
                "result": value.unwrap_or(Value::Null),
            })
        }
        ChildOutcome::NotCompleted { detail } => json!({
            "child": child_id,
            "status": "not-completed",
            "detail": detail,
        }),
        ChildOutcome::Unavailable(detail) => json!({
            "child": child_id,
            "status": "unavailable",
            "detail": detail,
        }),
    }
}

/// A message send through the gate, with the approval and
/// isolation remedies (sanitizing a text message is not meaningful;
/// payload sanitizers apply to tool calls and structured sends).
fn gated_send(
    shared: &Arc<RunShared>,
    factory: &dyn AgentFactory,
    state: &mut RunState,
    destination: &str,
    text: &str,
) -> Result<Value, BlockInfo> {
    let action = ProposedAction::Message {
        destination: destination.to_string(),
        payload_label: None,
    };
    let decision = state.trajectory.propose(&action).map_err(|err| BlockInfo {
        digest: String::new(),
        reasons: vec![phlow_flowguard::gate::BlockReason::PolicyMismatch {
            detail: err.to_string(),
        }],
        remedies: phlow_flowguard::gate::RemedyPlan::default(),
    })?;
    let info = match decision {
        Decision::Allow(_) => return route_message(shared, state, destination, text),
        Decision::Block(info) => info,
    };
    if shared.remedies.authority
        && let Some((authority, marks)) = first_approval(&info)
    {
        let request = ApprovalRequest {
            agent_id: state.agent_id.clone(),
            digest: info.digest.clone(),
            summary: format!("send to {destination}: {text}"),
            marks: marks.clone(),
        };
        if shared.service.approve(request) {
            let _ = state.trajectory.record(Event::ApprovalGranted {
                authority,
                digest: info.digest.clone(),
                marks,
            });
            if let Ok(Decision::Allow(_)) = state.trajectory.propose(&action) {
                return route_message(shared, state, destination, text);
            }
        }
    }
    if shared.remedies.isolate
        && info
            .remedies
            .options
            .iter()
            .any(|option| matches!(option, RemedyOption::Isolate))
    {
        let child_step = Step::SendMessage {
            destination: destination.to_string(),
            text: text.to_string(),
        };
        if let ChildOutcome::Completed { .. } = spawn_child(
            shared,
            factory,
            state,
            "isolated send",
            None,
            Box::new(OneShotAgent {
                step: Some(child_step),
            }),
        ) {
            return Ok(json!({"delivered": destination, "via": "isolated child"}));
        }
    }
    Err(info)
}

/// Route an allowed message: internal `agent:<id>` destinations go
/// to the live session's inbox carrying the sender's label
/// snapshot; everything else goes to the external sender.
fn route_message(
    shared: &RunShared,
    state: &mut RunState,
    destination: &str,
    text: &str,
) -> Result<Value, BlockInfo> {
    if let Some(target) = destination.strip_prefix("agent:") {
        let snapshot = state.trajectory.snapshot().map_err(|err| BlockInfo {
            digest: String::new(),
            reasons: vec![phlow_flowguard::gate::BlockReason::PolicyMismatch {
                detail: err.to_string(),
            }],
            remedies: phlow_flowguard::gate::RemedyPlan::default(),
        })?;
        if let Ok(mut inboxes) = shared.inboxes.lock()
            && let Some(queue) = inboxes.get_mut(target)
        {
            queue.push((state.agent_id.clone(), snapshot, text.to_string()));
            return Ok(json!({"delivered": destination}));
        }
        return Ok(json!({
            "delivered": destination,
            "note": "no live session with that id; the message was recorded, not delivered",
        }));
    }
    Ok(shared.service.external_send(destination, text))
}

fn first_sanitize(info: &BlockInfo) -> Option<(String, bool)> {
    let mut fallback = None;
    for option in &info.remedies.options {
        if let RemedyOption::Sanitize { name, would_allow } = option {
            if *would_allow {
                return Some((name.clone(), true));
            }
            fallback = fallback.or(Some((name.clone(), false)));
        }
    }
    fallback
}

fn first_approval(info: &BlockInfo) -> Option<(String, Vec<String>)> {
    info.remedies
        .options
        .iter()
        .find_map(|option| match option {
            RemedyOption::RequestApproval { authority, marks } => {
                Some((authority.clone(), marks.clone()))
            }
            _ => None,
        })
}

/// An agent that performs one scripted step and finishes (the
/// isolation worker).
struct OneShotAgent {
    step: Option<Step>,
}

impl SessionAgent for OneShotAgent {
    fn step(&mut self, _cx: &StepCx) -> Step {
        self.step.take().unwrap_or(Step::Done {
            summary: "isolated step finished".to_string(),
        })
    }
}

/// A scripted agent for tests and embedders' smoke checks: plays
/// its steps in order, then finishes.
pub struct ScriptAgent {
    steps: VecDeque<Step>,
}

impl ScriptAgent {
    /// An agent that will take `steps` in order.
    pub fn new(steps: Vec<Step>) -> ScriptAgent {
        ScriptAgent {
            steps: steps.into(),
        }
    }
}

impl SessionAgent for ScriptAgent {
    fn step(&mut self, _cx: &StepCx) -> Step {
        self.steps.pop_front().unwrap_or(Step::Done {
            summary: "script finished".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use phlow_attention::{SaliencyInput, Snapshot};
    use phlow_flowguard::label::ReaderSet;
    use std::sync::Barrier;

    const POLICY: &str = r#"
version = 1
trust_chain = ["suspicious", "trusted"]

[audiences]
self = ["op@example.com"]
internal = ["op@example.com", "dev@example.com"]

[[sources]]
name = "file:secret"
delta = { audience = ["self"] }

[[destinations]]
name = "agent:b"
readers = ["self"]

[[destinations]]
name = "external:world"
readers = ["public"]

[[tools]]
name = "read_note"
delta = { audience = ["self"] }

[[tools]]
name = "send_email"
requires = { audience_contains = ["$recipient"] }

[[tools]]
name = "deploy"
requires = { trust = "trusted" }

[[tools]]
name = "plan"
effects = ["dry-run"]

[[tools]]
name = "deploy_guarded"
requires = { effects_excludes = ["dry-run"] }

[[tools]]
name = "publish_digest"
requires = { audience_contains = ["internal"], trust = "trusted" }

[[sanitizers]]
name = "scrub"
tools = ["publish_digest"]
kind = "drop_parts"
result_delta = { audience = ["public"], trust = "trusted" }

[[authorities]]
name = "operator"
permits_tools = ["send_email", "publish_digest"]
"#;

    fn policy() -> Arc<Policy> {
        Arc::new(Policy::from_toml(POLICY).expect("test policy"))
    }

    fn snapshot() -> Arc<Snapshot> {
        let hot = SaliencyInput {
            named_in_task: true,
            recency: 1.0,
            ..SaliencyInput::quiet()
        };
        let warm = SaliencyInput {
            named_in_task: true,
            ..SaliencyInput::quiet()
        };
        Arc::new(
            Snapshot::build(vec![
                (TargetId::new("file:a").expect("id"), hot),
                (TargetId::new("file:b").expect("id"), warm),
            ])
            .expect("snapshot"),
        )
    }

    fn orchestrator(remedies: RemedyPolicy, accept_isolated: bool) -> Orchestrator {
        Orchestrator::new(
            policy(),
            snapshot(),
            Arc::new(Mutex::new(FocusBoard::new())),
            remedies,
            accept_isolated,
        )
    }

    fn spec(agent_id: &str) -> SessionSpec {
        SessionSpec {
            agent_id: agent_id.to_string(),
            task: format!("task for {agent_id}"),
            targets: Vec::new(),
            start_delta: None,
        }
    }

    /// Recorded tool calls: (agent id, tool, args).
    type DispatcherCalls = Arc<Mutex<Vec<(String, String, Value)>>>;

    #[derive(Default)]
    struct RecordingDispatcher {
        calls: DispatcherCalls,
    }

    impl ToolDispatcher for RecordingDispatcher {
        fn dispatch(&mut self, agent_id: &str, tool: &str, args: &Value) -> Value {
            self.calls.lock().expect("calls").push((
                agent_id.to_string(),
                tool.to_string(),
                args.clone(),
            ));
            json!({"ok": tool})
        }
    }

    impl RecordingDispatcher {
        fn boxed(self) -> (Box<dyn ToolDispatcher>, DispatcherCalls) {
            let calls = self.calls.clone();
            (Box::new(self), calls)
        }
    }

    #[derive(Default)]
    struct RecordingSender {
        sent: Arc<Mutex<Vec<(String, String)>>>,
    }

    impl ExternalSender for RecordingSender {
        fn send(&mut self, destination: &str, text: &str) -> Result<Value, String> {
            self.sent
                .lock()
                .expect("sent")
                .push((destination.to_string(), text.to_string()));
            Ok(json!({"delivered": destination}))
        }
    }

    struct ApprovingAuthority {
        requests: Arc<Mutex<Vec<String>>>,
    }

    impl AuthorityCallback for ApprovingAuthority {
        fn approve(&mut self, request: &ApprovalRequest) -> bool {
            self.requests
                .lock()
                .expect("requests")
                .push(request.digest.clone());
            true
        }
    }

    fn factory() -> ClosureFactory<impl Fn(&str) -> Box<dyn SessionAgent>> {
        ClosureFactory(|_task| -> Box<dyn SessionAgent> { Box::new(ScriptAgent::new(Vec::new())) })
    }

    // ------------------------------------------------------------
    // Validation.

    #[test]
    fn concurrent_sessions_complete_independently() {
        let orchestrator = orchestrator(RemedyPolicy::none(), false);
        let (dispatcher, calls) = RecordingDispatcher::default().boxed();
        let sessions: Vec<(SessionSpec, Box<dyn SessionAgent>)> = ["a", "b", "c"]
            .iter()
            .map(|id| {
                (
                    spec(id),
                    Box::new(ScriptAgent::new(vec![
                        Step::ToolCall {
                            tool: "read_note".to_string(),
                            args: json!({}),
                        },
                        Step::ToolCall {
                            tool: "deploy".to_string(),
                            args: json!({}),
                        },
                    ])) as Box<dyn SessionAgent>,
                )
            })
            .collect();
        let reports = orchestrator.run(&factory(), sessions, dispatcher, None, None);
        assert_eq!(reports.len(), 3);
        for report in &reports {
            // read_note narrows the label to `self` but deploy only
            // requires trust, so both calls pass for everyone.
            assert_eq!(
                report.status,
                SessionStatus::Completed,
                "{}",
                report.summary
            );
            assert_eq!(report.tool_results.len(), 2);
        }
        let calls = calls.lock().expect("calls");
        assert_eq!(calls.len(), 6);
        for id in ["a", "b", "c"] {
            assert_eq!(calls.iter().filter(|(agent, _, _)| agent == id).count(), 2);
        }
    }

    struct BarrierAgent {
        barrier: Arc<Barrier>,
        waited: bool,
        steps: VecDeque<Step>,
    }

    impl SessionAgent for BarrierAgent {
        fn step(&mut self, _cx: &StepCx) -> Step {
            if !self.waited {
                self.waited = true;
                self.barrier.wait();
            }
            self.steps.pop_front().unwrap_or(Step::Done {
                summary: "barrier agent finished".to_string(),
            })
        }
    }

    #[test]
    fn attention_contention_switches_exactly_one_session() {
        let orchestrator = orchestrator(RemedyPolicy::none(), false);
        let (dispatcher, _calls) = RecordingDispatcher::default().boxed();
        let barrier = Arc::new(Barrier::new(2));
        let make = |id: &str| {
            let mut spec = spec(id);
            spec.targets = vec!["file:a".to_string(), "file:b".to_string()];
            (
                spec,
                Box::new(BarrierAgent {
                    barrier: barrier.clone(),
                    waited: false,
                    steps: vec![Step::Wait, Step::Wait].into(),
                }) as Box<dyn SessionAgent>,
            )
        };
        let reports = orchestrator.run(
            &factory(),
            vec![make("a"), make("b")],
            dispatcher,
            None,
            None,
        );
        for report in &reports {
            assert_eq!(
                report.status,
                SessionStatus::Completed,
                "{}",
                report.summary
            );
        }
        let total_switches: usize = reports.iter().map(|report| report.switches.len()).sum();
        assert_eq!(
            total_switches, 1,
            "exactly one session must yield file:a: {reports:?}"
        );
        let switcher = reports
            .iter()
            .find(|report| !report.switches.is_empty())
            .expect("a switcher");
        assert!(
            switcher.events.iter().any(|event| matches!(
                event,
                Event::FocusSwitched { from, to, .. } if from == "file:a" && to == "file:b"
            )),
            "the switch must be a logged event: {:?}",
            switcher.events
        );
    }

    #[test]
    fn step_bound_ends_a_runaway_session() {
        struct Forever;
        impl SessionAgent for Forever {
            fn step(&mut self, _cx: &StepCx) -> Step {
                Step::ToolCall {
                    tool: "plan".to_string(),
                    args: json!({}),
                }
            }
        }
        let orchestrator = orchestrator(RemedyPolicy::none(), false);
        let (dispatcher, calls) = RecordingDispatcher::default().boxed();
        let reports = orchestrator.run(
            &factory(),
            vec![(spec("a"), Box::new(Forever) as Box<dyn SessionAgent>)],
            dispatcher,
            None,
            None,
        );
        assert_eq!(reports[0].status, SessionStatus::Failed);
        assert_eq!(reports[0].steps, STEPS_MAX);
        assert_eq!(calls.lock().expect("calls").len(), STEPS_MAX);
    }

    // ------------------------------------------------------------
    // Adversarial.

    #[test]
    fn laundering_between_sessions_is_blocked() {
        // A reads private data and hands it to B; B tries to send
        // it to the world. The receipt joined A's label into B, so
        // B's send is blocked and nothing reaches the sender.
        struct ForwardOnInbox;
        impl SessionAgent for ForwardOnInbox {
            fn step(&mut self, cx: &StepCx) -> Step {
                if let Some(item) = cx.inbox.first() {
                    return Step::SendMessage {
                        destination: "external:world".to_string(),
                        text: item.text.clone(),
                    };
                }
                // Wait like a real agent would — polling at a
                // human-scale cadence — rather than burning the
                // whole step bound before A's message can arrive.
                std::thread::sleep(std::time::Duration::from_millis(10));
                Step::Wait
            }
        }
        let orchestrator = orchestrator(RemedyPolicy::none(), false);
        let (dispatcher, _calls) = RecordingDispatcher::default().boxed();
        let sender = RecordingSender::default();
        let sent = sender.sent.clone();
        let a = (
            spec("a"),
            Box::new(ScriptAgent::new(vec![
                Step::ToolCall {
                    tool: "read_note".to_string(),
                    args: json!({}),
                },
                Step::SendMessage {
                    destination: "agent:b".to_string(),
                    text: "the quarterly secret".to_string(),
                },
            ])) as Box<dyn SessionAgent>,
        );
        let b = (spec("b"), Box::new(ForwardOnInbox) as Box<dyn SessionAgent>);
        let reports = orchestrator.run(
            &factory(),
            vec![a, b],
            dispatcher,
            None,
            Some(Box::new(sender)),
        );
        assert_eq!(reports[0].status, SessionStatus::Completed);
        assert_eq!(
            reports[1].status,
            SessionStatus::Blocked,
            "B must be blocked: {}",
            reports[1].summary
        );
        assert!(
            sent.lock().expect("sent").is_empty(),
            "nothing may reach the external sender"
        );
        // B's label carries A's narrowing — the receipt did it.
        let label = reports[1].final_label.clone().expect("label");
        match label.readers {
            ReaderSet::Only(readers) => {
                assert!(readers.contains("op@example.com"));
                assert_eq!(readers.len(), 1);
            }
            ReaderSet::All => panic!("B's audience must have narrowed"),
        }
    }

    #[test]
    fn low_trust_start_cannot_deploy() {
        // A session tasked with unvetted external content starts at
        // lowered trust; a privileged call is blocked even though
        // the trajectory is otherwise empty — the content that
        // "asked" for the deploy is exactly what disqualifies it.
        let orchestrator = orchestrator(RemedyPolicy::none(), false);
        let (dispatcher, calls) = RecordingDispatcher::default().boxed();
        let mut tainted = spec("tainted");
        tainted.start_delta = Some(LabelDelta::trust("suspicious"));
        let sessions = vec![
            (
                tainted,
                Box::new(ScriptAgent::new(vec![Step::ToolCall {
                    tool: "deploy".to_string(),
                    args: json!({}),
                }])) as Box<dyn SessionAgent>,
            ),
            (
                spec("clean"),
                Box::new(ScriptAgent::new(vec![Step::ToolCall {
                    tool: "deploy".to_string(),
                    args: json!({}),
                }])) as Box<dyn SessionAgent>,
            ),
        ];
        let reports = orchestrator.run(&factory(), sessions, dispatcher, None, None);
        assert_eq!(reports[0].status, SessionStatus::Blocked);
        assert_eq!(reports[1].status, SessionStatus::Completed);
        assert_eq!(
            calls.lock().expect("calls").len(),
            1,
            "only the clean deploy ran"
        );
    }

    #[test]
    fn delegation_result_joins_the_childs_label_into_the_parent() {
        // The child reads private data; its result comes back, and
        // the parent — which never touched the data itself — can no
        // longer email it outside. Delegation is not a laundry.
        let child_factory = ClosureFactory(|_task| -> Box<dyn SessionAgent> {
            Box::new(ScriptAgent::new(vec![Step::ToolCall {
                tool: "read_note".to_string(),
                args: json!({}),
            }]))
        });
        let orchestrator = orchestrator(RemedyPolicy::none(), false);
        let (dispatcher, _calls) = RecordingDispatcher::default().boxed();
        let parent = (
            spec("parent"),
            Box::new(ScriptAgent::new(vec![
                Step::Delegate {
                    task: "research the note".to_string(),
                    delta: None,
                },
                Step::ToolCall {
                    tool: "send_email".to_string(),
                    args: json!({"recipient": "ext@evil.example"}),
                },
            ])) as Box<dyn SessionAgent>,
        );
        let reports = orchestrator.run(&child_factory, vec![parent], dispatcher, None, None);
        assert_eq!(
            reports[0].status,
            SessionStatus::Blocked,
            "parent must inherit the child's restriction: {}",
            reports[0].summary
        );
        assert!(
            reports[0]
                .events
                .iter()
                .any(|event| matches!(event, Event::SubagentResult { .. })),
            "the join must be a recorded event"
        );
    }

    #[test]
    fn sanitize_remedy_lets_a_lowered_trajectory_publish() {
        let orchestrator = orchestrator(
            RemedyPolicy {
                sanitize: true,
                ..RemedyPolicy::none()
            },
            false,
        );
        let (dispatcher, calls) = RecordingDispatcher::default().boxed();
        let mut lowered = spec("a");
        lowered.start_delta = Some(LabelDelta::trust("suspicious"));
        let sessions = vec![(
            lowered,
            Box::new(ScriptAgent::new(vec![Step::ToolCall {
                tool: "publish_digest".to_string(),
                args: json!({"parts": [{"text": "summary"}, {"file": "raw.bin"}]}),
            }])) as Box<dyn SessionAgent>,
        )];
        let reports = orchestrator.run(&factory(), sessions, dispatcher, None, None);
        assert_eq!(
            reports[0].status,
            SessionStatus::Completed,
            "{}",
            reports[0].summary
        );
        let calls = calls.lock().expect("calls");
        assert_eq!(calls.len(), 1, "only the sanitized retry dispatches");
        let parts = calls[0].2["parts"].as_array().expect("parts");
        assert_eq!(parts.len(), 1, "the file part was dropped by the sanitizer");
    }

    #[test]
    fn authority_approval_is_spent_by_execution_and_asked_again() {
        let orchestrator = orchestrator(
            RemedyPolicy {
                authority: true,
                ..RemedyPolicy::none()
            },
            false,
        );
        let (dispatcher, calls) = RecordingDispatcher::default().boxed();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let authority = ApprovingAuthority {
            requests: requests.clone(),
        };
        let email = Step::ToolCall {
            tool: "send_email".to_string(),
            args: json!({"recipient": "cfo@example.com"}),
        };
        let sessions = vec![(
            spec("a"),
            Box::new(ScriptAgent::new(vec![
                Step::ToolCall {
                    tool: "read_note".to_string(),
                    args: json!({}),
                },
                email.clone(),
                email,
            ])) as Box<dyn SessionAgent>,
        )];
        let reports = orchestrator.run(
            &factory(),
            sessions,
            dispatcher,
            Some(Box::new(authority)),
            None,
        );
        assert_eq!(
            reports[0].status,
            SessionStatus::Completed,
            "{}",
            reports[0].summary
        );
        assert_eq!(
            requests.lock().expect("requests").len(),
            2,
            "each send needs its own approval — the first was spent by the first result"
        );
        assert_eq!(
            calls.lock().expect("calls").len(),
            3,
            "read + two approved sends"
        );
    }

    #[test]
    fn isolation_runs_the_step_in_a_child_and_quarantines_the_result() {
        // The parent's log carries the `dry-run` effect, which the
        // guarded deploy excludes. The isolated child has a clean
        // log, so the step runs there; with acceptance off, the
        // parent's trajectory never records the deploy.
        let orchestrator = orchestrator(
            RemedyPolicy {
                isolate: true,
                ..RemedyPolicy::none()
            },
            false,
        );
        let (dispatcher, calls) = RecordingDispatcher::default().boxed();
        let sessions = vec![(
            spec("parent"),
            Box::new(ScriptAgent::new(vec![
                Step::ToolCall {
                    tool: "plan".to_string(),
                    args: json!({}),
                },
                Step::ToolCall {
                    tool: "deploy_guarded".to_string(),
                    args: json!({}),
                },
            ])) as Box<dyn SessionAgent>,
        )];
        let reports = orchestrator.run(&factory(), sessions, dispatcher, None, None);
        assert_eq!(
            reports[0].status,
            SessionStatus::Completed,
            "{}",
            reports[0].summary
        );
        let calls = calls.lock().expect("calls");
        let deploys: Vec<&(String, String, Value)> = calls
            .iter()
            .filter(|(_, tool, _)| tool == "deploy_guarded")
            .collect();
        assert_eq!(deploys.len(), 1);
        assert!(
            deploys[0].0.starts_with("parent/child-"),
            "the deploy ran in the child, got {}",
            deploys[0].0
        );
        assert!(
            !reports[0]
                .events
                .iter()
                .any(|event| matches!(event, Event::SubagentResult { .. })),
            "a quarantined result must not join the parent"
        );
        let last = reports[0].tool_results.last().expect("result");
        assert_eq!(last["isolated"], true);
        assert_eq!(last["accepted"], false);
    }
}
