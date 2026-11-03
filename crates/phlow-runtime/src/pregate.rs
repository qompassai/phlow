//! The utility pre-gate: cheap-model task scoring before dispatch.
//!
//! When `[pregate]` is enabled in the operator config, [`Runtime::run`]
//! consults this gate before the first (expensive) model dispatch: a
//! small local model — the *scorer* — rates the task's expected utility
//! and the gate routes on that score. Thresholds are operator
//! configuration ([`PreGateConfig`]), never constants in this module.
//!
//! Routing:
//!
//! - score at or above `high_threshold` → dispatch as planned;
//! - mid score → downgrade: the run uses `cheap_model` for every role
//!   when one is configured, otherwise it keeps the planned models with
//!   the cycle budget narrowed to `narrow_max_cycles`;
//! - score at or below `low_threshold` → the task is not dispatched;
//!   the run report says `queued` and carries the score and reasons, so
//!   the caller (CLI, TUI, or MCP) can re-queue it with the evidence.
//!
//! Hard boundary: the gate advises *dispatch routing only*. It is
//! structurally unable to grant permission — this module imports no
//! permission surface (no `phlow-flowguard`, no `phlow-approval`),
//! [`PreGateRoute`] has no allow/deny variant, and nothing here is
//! consulted by any authorization path. A high score never widens the
//! run's scope; a low score never blocks permitted work outright (a
//! direct tool call through `Runtime::call_tool` is untouched by the
//! gate). Permission gates stay upstream and unchanged.
//!
//! Fail-open: every scorer failure — transport error, deadline expiry,
//! malformed reply — routes [`PreGateRoute::AsPlanned`] with the
//! failure recorded in the report. A courtesy scorer must never become
//! a chokepoint that deadlocks dispatch.
//!
//! [`Runtime::run`]: crate::runtime::Runtime::run

use std::fmt;
use std::time::Duration;

use phlow_config::PreGateConfig;
use phlow_llm::LlmTransport;
use phlow_llm::transport::OllamaBackend;
use phlow_workspace::Workspace;
use serde_json::{Map, Value};

/// Maximum reasons kept from one score. Bounds the report record no
/// matter what the scorer emits.
pub const SCORE_REASONS_MAX: usize = 8;
/// Maximum characters kept per reason.
pub const SCORE_REASON_CHARS_MAX: usize = 200;
/// Maximum task characters shown to the scorer model. The task is
/// already capped by `agent.max_task_chars`; the scorer judges the
/// task's shape, and a second full copy of a huge prompt would defeat
/// the point of a cheap pre-gate.
pub const SCORER_TASK_CHARS_MAX: usize = 4_000;

/// The scorer's system prompt. The reply contract is one JSON object,
/// `{"score": <integer 0..=100>, "reasons": [<string>, ...]}`; the
/// parser in this module enforces it and fails open on any deviation.
const SCORER_SYSTEM_PROMPT: &str = "You are the utility pre-gate of an agent runtime. \
Judge the expected utility of running the given task on an expensive frontier model now. \
Consider: (1) does the task merit a full frontier-model run, or is it trivial, vague, or \
duplicative? (2) does it look already answered by material in the workspace (see the \
workspace_files signal)? (3) is its shape one that historically burns a whole session for \
little result (unbounded scope, no verifiable outcome)? Reply with ONLY a JSON object of \
the form {\"score\": <integer 0-100>, \"reasons\": [\"<short reason>\", ...]}. A high score \
means a full run has strong expected utility; a low score means it does not.";

/// One scored assessment of a task's expected utility.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UtilityScore {
    /// Expected utility, 0..=100.
    score: u32,
    /// Bounded reasons from the scorer (at most [`SCORE_REASONS_MAX`],
    /// each at most [`SCORE_REASON_CHARS_MAX`] characters).
    reasons: Vec<String>,
}

impl UtilityScore {
    /// Build a score, truncating `reasons` to the module bounds.
    ///
    /// # Panics
    ///
    /// Panics when `score` exceeds 100: scores are a closed 0..=100
    /// scale and an out-of-range value is a programming error, not
    /// scorer data (scorer data enters through [`parse_score`], which
    /// rejects out-of-range values as malformed).
    pub fn new(score: u32, reasons: Vec<String>) -> UtilityScore {
        assert!(score <= 100, "utility score out of range: {score}");
        UtilityScore {
            score,
            reasons: bound_reasons(reasons),
        }
    }

    /// Expected utility, 0..=100.
    pub fn score(&self) -> u32 {
        self.score
    }

    /// The scorer's reasons, bounded at construction.
    pub fn reasons(&self) -> &[String] {
        &self.reasons
    }
}

/// Cheap host-side facts about a task, computed without any model.
/// They inform the scorer prompt; they never decide anything alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskSignals {
    /// Task length in characters.
    pub task_chars: u32,
    /// Regular files visible in the workspace (the workspace's own
    /// bounded listing; the count is capped by that listing's caps).
    pub workspace_files: u32,
}

impl TaskSignals {
    /// Gather the signals for `task` against `workspace`. The listing
    /// is the workspace's bounded walk; a listing failure reports zero
    /// files rather than failing the gate (fail-open starts here).
    pub fn gather(task: &str, workspace: &Workspace) -> TaskSignals {
        let task_chars = u32::try_from(task.chars().count()).unwrap_or(u32::MAX);
        let workspace_files = workspace
            .list(".")
            .map(|listed| u32::try_from(listed.files.len()).unwrap_or(u32::MAX))
            .unwrap_or(0);
        TaskSignals {
            task_chars,
            workspace_files,
        }
    }

    /// The signals as a report/prompt JSON object.
    fn to_value(self) -> Value {
        serde_json::json!({
            "task_chars": self.task_chars,
            "workspace_files": self.workspace_files,
        })
    }
}

/// Why scoring produced no usable score. Every variant fails open:
/// the gate routes as planned and records the failure.
#[derive(Debug)]
pub enum ScorerError {
    /// The scorer answered, but not in the contracted shape.
    Malformed(String),
    /// The scorer call itself failed (refused, reset, HTTP status, or
    /// the configured deadline expired in the transport).
    Transport(String),
}

impl ScorerError {
    /// Machine-stable error kind for the run report.
    pub fn kind(&self) -> &'static str {
        match self {
            ScorerError::Malformed(_) => "malformed",
            ScorerError::Transport(_) => "transport",
        }
    }
}

impl fmt::Display for ScorerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScorerError::Malformed(detail) => write!(formatter, "scorer reply malformed: {detail}"),
            ScorerError::Transport(detail) => write!(formatter, "scorer call failed: {detail}"),
        }
    }
}

impl std::error::Error for ScorerError {}

/// The scoring seam. Production uses [`ModelScorer`]; tests inject
/// scripted scorers so routing is testable without a model server.
pub trait UtilityScorer {
    /// Score `task` (with its host signals) for expected utility.
    fn score_task(
        &mut self,
        task: &str,
        signals: &TaskSignals,
    ) -> Result<UtilityScore, ScorerError>;
}

/// A scorer that always fails with a fixed reason. The runtime uses
/// it when no backend exists to score through, so that path takes the
/// same fail-open route (and produces the same record shape) as any
/// other scorer failure.
pub struct UnavailableScorer {
    reason: String,
}

impl UnavailableScorer {
    /// A scorer that fails every call with `reason`.
    pub fn new(reason: &str) -> UnavailableScorer {
        UnavailableScorer {
            reason: reason.to_owned(),
        }
    }
}

impl UtilityScorer for UnavailableScorer {
    fn score_task(
        &mut self,
        _task: &str,
        _signals: &TaskSignals,
    ) -> Result<UtilityScore, ScorerError> {
        Err(ScorerError::Transport(self.reason.clone()))
    }
}

/// The production scorer: one bounded chat call to the configured
/// cheap model over the runtime's existing backend, under the
/// configured deadline.
pub struct ModelScorer<'a, L: LlmTransport> {
    backend: &'a mut OllamaBackend<L>,
    model: String,
    timeout: Duration,
}

impl<'a, L: LlmTransport> ModelScorer<'a, L> {
    /// Score through `backend` using `model`, abandoning the call at
    /// `timeout` (enforced by the transport; expiry is a
    /// [`ScorerError::Transport`], never a hang).
    pub fn new(backend: &'a mut OllamaBackend<L>, model: &str, timeout: Duration) -> Self {
        ModelScorer {
            backend,
            model: model.to_owned(),
            timeout,
        }
    }
}

impl<L: LlmTransport> UtilityScorer for ModelScorer<'_, L> {
    fn score_task(
        &mut self,
        task: &str,
        signals: &TaskSignals,
    ) -> Result<UtilityScore, ScorerError> {
        let messages = vec![
            serde_json::json!({"role": "system", "content": SCORER_SYSTEM_PROMPT}),
            serde_json::json!({"role": "user", "content": scorer_prompt(task, signals)}),
        ];
        let body = self
            .backend
            .chat_with_timeout(&messages, &[], Some(&self.model), self.timeout)
            .map_err(|error| ScorerError::Transport(error.to_string()))?;
        let content = chat_content(&body)?;
        parse_score(&content)
    }
}

/// The dispatch route the gate advises. Deliberately permission-free:
/// no variant allows, denies, or widens anything — the variants only
/// choose which models run the task, or whether it runs now at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreGateRoute {
    /// Dispatch exactly as configured.
    AsPlanned,
    /// Dispatch downgraded: `model` replaces every role's model when
    /// set; otherwise the planned models run with the cycle budget
    /// narrowed to `max_cycles`. Exactly one of the two is set.
    Downgraded {
        /// The cheap model for every role, when one is configured.
        model: Option<String>,
        /// The narrowed cycle cap, when no cheap model is configured.
        max_cycles: Option<u32>,
    },
    /// Do not dispatch: the task returns to the caller's queue with
    /// the score and reasons recorded in the run report.
    Queued,
}

impl PreGateRoute {
    /// Machine-stable route name for the run report.
    pub fn as_str(&self) -> &'static str {
        match self {
            PreGateRoute::AsPlanned => "as_planned",
            PreGateRoute::Downgraded { .. } => "downgraded",
            PreGateRoute::Queued => "queued",
        }
    }
}

/// One consultation's outcome: the route plus the `utility_pregate`
/// object the runtime records in the run report.
#[derive(Debug)]
pub struct PreGateVerdict {
    /// The advised dispatch route.
    pub route: PreGateRoute,
    /// The report record (score, reasons, route, or the failure).
    pub record: Value,
}

/// Consult the gate for `task`. Fail-open by construction: a scorer
/// error yields [`PreGateRoute::AsPlanned`] and a record whose
/// `available` is false, so the report always shows whether the route
/// came from a real score or from the gate being unable to score.
pub fn consult<S: UtilityScorer + ?Sized>(
    scorer: &mut S,
    config: &PreGateConfig,
    task: &str,
    signals: &TaskSignals,
) -> PreGateVerdict {
    let mut record = Map::new();
    record.insert(
        "scorer_model".to_owned(),
        Value::String(config.scorer_model().to_owned()),
    );
    record.insert("signals".to_owned(), signals.to_value());
    let score = match scorer.score_task(task, signals) {
        Ok(score) => score,
        Err(error) => {
            record.insert("available".to_owned(), Value::Bool(false));
            record.insert(
                "error_kind".to_owned(),
                Value::String(error.kind().to_owned()),
            );
            record.insert(
                "error".to_owned(),
                Value::String(bounded_text(&error.to_string())),
            );
            record.insert(
                "route".to_owned(),
                Value::String(PreGateRoute::AsPlanned.as_str().to_owned()),
            );
            return PreGateVerdict {
                route: PreGateRoute::AsPlanned,
                record: Value::Object(record),
            };
        }
    };
    record.insert("available".to_owned(), Value::Bool(true));
    record.insert("score".to_owned(), Value::from(score.score()));
    record.insert(
        "reasons".to_owned(),
        Value::Array(
            score
                .reasons()
                .iter()
                .map(|reason| Value::String(reason.clone()))
                .collect(),
        ),
    );
    let route = route_for(config, &score);
    record.insert("route".to_owned(), Value::String(route.as_str().to_owned()));
    if let PreGateRoute::Downgraded { model, max_cycles } = &route {
        if let Some(model) = model {
            record.insert("downgrade_model".to_owned(), Value::String(model.clone()));
        }
        if let Some(max_cycles) = max_cycles {
            record.insert("narrow_max_cycles".to_owned(), Value::from(*max_cycles));
        }
    }
    PreGateVerdict {
        route,
        record: Value::Object(record),
    }
}

/// Map a score to its route under the configured thresholds. The
/// boundary scores belong to the outer routes: exactly `high` is a
/// full dispatch, exactly `low` is a queue.
fn route_for(config: &PreGateConfig, score: &UtilityScore) -> PreGateRoute {
    if score.score() >= config.high_threshold() {
        return PreGateRoute::AsPlanned;
    }
    if score.score() <= config.low_threshold() {
        return PreGateRoute::Queued;
    }
    if config.cheap_model().is_empty() {
        PreGateRoute::Downgraded {
            model: None,
            max_cycles: Some(config.narrow_max_cycles()),
        }
    } else {
        PreGateRoute::Downgraded {
            model: Some(config.cheap_model().to_owned()),
            max_cycles: None,
        }
    }
}

/// Parse a scorer reply into a [`UtilityScore`]. The reply must carry
/// a JSON object (surrounding prose is tolerated: the first `{` to the
/// last `}` is the candidate) with an integer `score` in 0..=100 and an
/// optional `reasons` array of strings. Anything else is
/// [`ScorerError::Malformed`] — the caller fails open on it.
pub fn parse_score(content: &str) -> Result<UtilityScore, ScorerError> {
    let start = content
        .find('{')
        .ok_or_else(|| ScorerError::Malformed("reply carries no JSON object".to_owned()))?;
    let end = content
        .rfind('}')
        .ok_or_else(|| ScorerError::Malformed("reply carries no JSON object".to_owned()))?;
    if end < start {
        return Err(ScorerError::Malformed(
            "reply carries no JSON object".to_owned(),
        ));
    }
    let value: Value = serde_json::from_str(&content[start..=end])
        .map_err(|error| ScorerError::Malformed(format!("reply is not valid JSON: {error}")))?;
    let raw_score = value
        .get("score")
        .ok_or_else(|| ScorerError::Malformed("reply has no score".to_owned()))?;
    let score = parse_score_value(raw_score)?;
    let reasons = match value.get("reasons") {
        None => Vec::new(),
        Some(Value::Array(items)) => {
            let mut reasons = Vec::with_capacity(items.len().min(SCORE_REASONS_MAX));
            for item in items {
                let Some(text) = item.as_str() else {
                    return Err(ScorerError::Malformed("reasons must be strings".to_owned()));
                };
                reasons.push(text.to_owned());
            }
            reasons
        }
        Some(_) => {
            return Err(ScorerError::Malformed(
                "reasons must be an array of strings".to_owned(),
            ));
        }
    };
    Ok(UtilityScore::new(score, reasons))
}

/// The score field itself: an integer in 0..=100. An integral float
/// (`72.0`) is accepted — small models emit it reliably — but a
/// fractional, negative, over-range, or non-numeric value is malformed.
fn parse_score_value(raw: &Value) -> Result<u32, ScorerError> {
    let malformed =
        || ScorerError::Malformed("score must be an integer between 0 and 100".to_owned());
    let Some(number) = raw.as_number() else {
        return Err(malformed());
    };
    if let Some(integer) = number.as_u64() {
        return match u32::try_from(integer) {
            Ok(score) if score <= 100 => Ok(score),
            _ => Err(malformed()),
        };
    }
    let Some(float) = number.as_f64() else {
        return Err(malformed());
    };
    if float.fract() == 0.0 && (0.0..=100.0).contains(&float) {
        // Integral and range-checked above: the conversion is exact.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let score = float as u32;
        return Ok(score);
    }
    Err(malformed())
}

/// The scorer's user prompt: the (truncated) task plus its signals as
/// one JSON object, so the model's reply contract stays about shape,
/// not about parsing prose.
fn scorer_prompt(task: &str, signals: &TaskSignals) -> String {
    let truncated: String = task.chars().take(SCORER_TASK_CHARS_MAX).collect();
    serde_json::json!({
        "task": truncated,
        "signals": signals.to_value(),
    })
    .to_string()
}

/// Extract `choices[0].message.content` from a chat body, mirroring
/// the runtime's own response discipline. Any shape deviation is a
/// malformed scorer reply (fail-open upstream), never a panic.
fn chat_content(body: &Value) -> Result<String, ScorerError> {
    let message = body
        .get("choices")
        .and_then(|choices| choices.get(0))
        .and_then(|choice| choice.get("message"))
        .filter(|message| message.is_object())
        .ok_or_else(|| ScorerError::Malformed("response has no choices[0].message".to_owned()))?;
    match message.get("content") {
        Some(Value::String(text)) => Ok(text.clone()),
        _ => Err(ScorerError::Malformed(
            "scorer message content must be a string".to_owned(),
        )),
    }
}

/// Truncate reasons to the module bounds (count, then characters).
fn bound_reasons(reasons: Vec<String>) -> Vec<String> {
    reasons
        .into_iter()
        .take(SCORE_REASONS_MAX)
        .map(|reason| reason.chars().take(SCORE_REASON_CHARS_MAX).collect())
        .collect()
}

/// Bound free text recorded into the report (error details).
fn bounded_text(text: &str) -> String {
    text.chars().take(SCORE_REASON_CHARS_MAX).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use phlow_config::OllamaConfig;
    use phlow_llm::LlmError;

    fn config(cheap_model: &str) -> PreGateConfig {
        // The validated constructor is crate-private to phlow-config;
        // tests here go through the public load path instead.
        let dir = std::env::temp_dir().join(format!(
            "phlow-pregate-unit-{}-{}",
            std::process::id(),
            cheap_model.len()
        ));
        std::fs::create_dir_all(&dir).expect("test dir");
        let toml = format!(
            "[pregate]\nenabled = true\nscorer_model = \"scorer:small\"\ncheap_model = \"{cheap_model}\"\nhigh_threshold = 70\nlow_threshold = 30\nnarrow_max_cycles = 1\ntimeout_ms = 5000\n"
        );
        std::fs::write(dir.join("config.toml"), toml).expect("config.toml");
        let config = phlow_config::load_config(&phlow_config::LoadOptions {
            config_path: Some(dir.join("config.toml")),
            workspace: Some(dir.clone()),
            trusted: false,
            model: None,
        })
        .expect("test config loads");
        config.pregate().clone()
    }

    fn signals() -> TaskSignals {
        TaskSignals {
            task_chars: 42,
            workspace_files: 7,
        }
    }

    /// A scorer with one scripted outcome.
    struct StubScorer {
        outcome: StubOutcome,
    }

    enum StubOutcome {
        Score(u32),
        Malformed,
        Transport,
    }

    impl UtilityScorer for StubScorer {
        fn score_task(
            &mut self,
            _task: &str,
            _signals: &TaskSignals,
        ) -> Result<UtilityScore, ScorerError> {
            match self.outcome {
                StubOutcome::Score(score) => {
                    Ok(UtilityScore::new(score, vec!["stub reason".to_owned()]))
                }
                StubOutcome::Malformed => Err(ScorerError::Malformed("stub malformed".to_owned())),
                StubOutcome::Transport => Err(ScorerError::Transport("stub transport".to_owned())),
            }
        }
    }

    // -- Validation half ----------------------------------------------------

    #[test]
    fn parse_score_reads_clean_json() {
        let score = parse_score(r#"{"score": 85, "reasons": ["clear scope", "verifiable"]}"#)
            .expect("clean JSON parses");
        assert_eq!(score.score(), 85);
        assert_eq!(score.reasons(), ["clear scope", "verifiable"]);
    }

    #[test]
    fn parse_score_tolerates_surrounding_prose() {
        let score = parse_score("Here is my assessment: {\"score\": 64, \"reasons\": []} — done.")
            .expect("prose-wrapped JSON parses");
        assert_eq!(score.score(), 64);
        assert!(score.reasons().is_empty());
    }

    #[test]
    fn parse_score_accepts_integral_float() {
        let score = parse_score(r#"{"score": 72.0}"#).expect("integral float parses");
        assert_eq!(score.score(), 72);
    }

    #[test]
    fn parse_score_truncates_reasons_to_bounds() {
        let many: Vec<String> = (0..20).map(|index| format!("reason {index}")).collect();
        let body = serde_json::json!({"score": 50, "reasons": many}).to_string();
        let score = parse_score(&body).expect("parses");
        assert_eq!(score.reasons().len(), SCORE_REASONS_MAX);
        let long = "x".repeat(SCORE_REASON_CHARS_MAX + 50);
        let body = serde_json::json!({"score": 50, "reasons": [long]}).to_string();
        let score = parse_score(&body).expect("parses");
        assert_eq!(score.reasons()[0].chars().count(), SCORE_REASON_CHARS_MAX);
    }

    #[test]
    fn routes_follow_the_configured_thresholds() {
        let with_cheap = config("cheap:small");
        let bare = config("");
        let cases = [
            (100, PreGateRoute::AsPlanned),
            (70, PreGateRoute::AsPlanned),
            (
                69,
                PreGateRoute::Downgraded {
                    model: Some("cheap:small".to_owned()),
                    max_cycles: None,
                },
            ),
            (
                31,
                PreGateRoute::Downgraded {
                    model: Some("cheap:small".to_owned()),
                    max_cycles: None,
                },
            ),
            (30, PreGateRoute::Queued),
            (0, PreGateRoute::Queued),
        ];
        for (raw, expected) in cases {
            let verdict = consult(
                &mut StubScorer {
                    outcome: StubOutcome::Score(raw),
                },
                &with_cheap,
                "task",
                &signals(),
            );
            assert_eq!(verdict.route, expected, "score {raw}");
            assert_eq!(verdict.record["score"], Value::from(raw));
            assert_eq!(verdict.record["available"], Value::Bool(true));
        }
        // Without a cheap model, a mid score narrows cycles instead.
        let verdict = consult(
            &mut StubScorer {
                outcome: StubOutcome::Score(50),
            },
            &bare,
            "task",
            &signals(),
        );
        assert_eq!(
            verdict.route,
            PreGateRoute::Downgraded {
                model: None,
                max_cycles: Some(1),
            }
        );
        assert_eq!(verdict.record["narrow_max_cycles"], Value::from(1));
    }

    // -- Adversarial half ---------------------------------------------------

    #[test]
    fn parse_score_rejects_every_malformed_shape() {
        for body in [
            "no json at all",
            "{}",
            r#"{"score": "eighty"}"#,
            r#"{"score": 101}"#,
            r#"{"score": -1}"#,
            r#"{"score": 55.5}"#,
            r#"{"score": 50, "reasons": "many"}"#,
            r#"{"score": 50, "reasons": [42]}"#,
            r#"{"score": 50"#,
        ] {
            let error = parse_score(body).expect_err("must be malformed");
            assert_eq!(error.kind(), "malformed", "body: {body}");
        }
    }

    #[test]
    fn scorer_failures_fail_open_to_as_planned() {
        for outcome in [StubOutcome::Transport, StubOutcome::Malformed] {
            let kind = match outcome {
                StubOutcome::Transport => "transport",
                _ => "malformed",
            };
            let verdict = consult(&mut StubScorer { outcome }, &config(""), "task", &signals());
            assert_eq!(verdict.route, PreGateRoute::AsPlanned);
            assert_eq!(verdict.record["available"], Value::Bool(false));
            assert_eq!(verdict.record["error_kind"], Value::String(kind.to_owned()));
            assert_eq!(
                verdict.record["route"],
                Value::String("as_planned".to_owned())
            );
        }
    }

    /// A transport that records the deadline it was handed and then
    /// fails the way a real deadline expiry surfaces.
    #[derive(Clone, Default)]
    struct DeadlineTransport {
        seen_timeout: std::sync::Arc<std::sync::Mutex<Option<Duration>>>,
    }

    impl LlmTransport for DeadlineTransport {
        fn post_chat(
            &mut self,
            _base_url: &str,
            _payload: &Map<String, Value>,
            timeout: Duration,
        ) -> Result<Value, LlmError> {
            *self.seen_timeout.lock().expect("deadline lock") = Some(timeout);
            Err(LlmError::Transport(
                "request timed out after deadline".to_owned(),
            ))
        }

        fn get_tags(&mut self, _base_url: &str, _timeout: Duration) -> Result<Value, LlmError> {
            Ok(serde_json::json!({"models": []}))
        }

        fn close(&mut self) {}
    }

    #[test]
    fn scorer_timeout_fails_open_and_uses_the_configured_deadline() {
        let transport = DeadlineTransport::default();
        let seen = transport.seen_timeout.clone();
        let mut backend = OllamaBackend::new(OllamaConfig::default(), transport);
        let deadline = Duration::from_millis(2_500);
        {
            let mut scorer = ModelScorer::new(&mut backend, "scorer:small", deadline);
            let error = scorer
                .score_task("task", &signals())
                .expect_err("a timed-out scorer call is an error");
            assert_eq!(error.kind(), "transport");
        }
        // The deadline the scorer was configured with is the deadline
        // the transport was handed: a hung scorer cannot stall
        // dispatch past it.
        assert_eq!(*seen.lock().expect("deadline lock"), Some(deadline));
        // And at the gate level the timeout routes as planned.
        let verdict = consult(
            &mut StubScorer {
                outcome: StubOutcome::Transport,
            },
            &config(""),
            "task",
            &signals(),
        );
        assert_eq!(verdict.route, PreGateRoute::AsPlanned);
    }

    #[test]
    fn model_scorer_rejects_a_non_json_reply_as_malformed() {
        let mut fake = phlow_llm::FakeLlmTransport::new();
        fake.queue_reply(Ok(serde_json::json!({
            "choices": [{"message": {"role": "assistant", "content": "This task looks great!"}}]
        })));
        let mut backend = OllamaBackend::new(OllamaConfig::default(), fake);
        let mut scorer = ModelScorer::new(&mut backend, "scorer:small", Duration::from_secs(5));
        let error = scorer
            .score_task("task", &signals())
            .expect_err("prose is not a score");
        assert_eq!(error.kind(), "malformed");
    }
}
