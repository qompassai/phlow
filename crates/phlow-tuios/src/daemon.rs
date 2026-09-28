//! The session daemon: sessions own windows, windows host agent panes, and a
//! registry of verbs exposes them over the control socket.
//!
//! Adapted from tuios's `internal/session` daemon: tuios multiplexes real
//! terminal panes; this daemon multiplexes *agent* panes — a window is an
//! agent's place in a session, with its reported state, its inbox view over
//! the session mailbox, and the hooks that fire on its transitions. The
//! session/window/hook/event vocabulary is tuios's; the verb surface below is
//! phlow's, because the protocol serves agent orchestration rather than
//! terminal emulation.
//!
//! Dispatch returns [`ProtocolError`] rather than [`TuiosError`] so hints
//! (`did_you_mean`, accepted params) survive to the response envelope.

use std::collections::HashMap;
use std::time::Instant;

use serde_json::{Value, json};

use crate::error::{ErrorCode, TuiosError};
use crate::hooks::{HookContext, HookEvent, HookManager};
use crate::mailbox::{
    AskGraph, HUMAN, Mailbox, Message, MessageKind, READ_DEFAULT_LIMIT, ReadFilter,
};
use crate::protocol::{ProtocolError, VerbHint, closest_verb};
use crate::state::{AgentPaneState, AgentState};

/// Bound: sessions one daemon holds.
pub const SESSIONS_MAX: usize = 256;
/// Bound: session name length, in bytes.
pub const SESSION_NAME_BYTES_MAX: usize = 128;
/// Bound: windows in one session.
pub const WINDOWS_PER_SESSION_MAX: usize = 64;
/// Bound: window name length, in bytes.
pub const WINDOW_NAME_BYTES_MAX: usize = 128;
/// Bound: an ask question, in bytes.
pub const ASK_QUESTION_BYTES_MAX: usize = 8 * 1024;
/// Bound: `ask-agent` timeout, in seconds. Accepted for wire compatibility;
/// delivery is synchronous, so the wait never happens (see the verb docs).
pub const ASK_TIMEOUT_SECS_MAX: u64 = 3600;
/// Bound: inbox/mail reads per call. Reads are already capped by the mailbox.
pub const READ_LIMIT_MAX: usize = 256;

/// One agent's place in a session: a name, a stable id, and the daemon-owned
/// record of what the agent reported about itself.
#[derive(Debug)]
pub struct AgentPane {
    name: String,
    id: u64,
    agent: AgentPaneState,
}

impl AgentPane {
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn id(&self) -> u64 {
        self.id
    }

    #[must_use]
    pub fn state(&self) -> AgentState {
        self.agent.state()
    }
}

/// One session: windows, the focused window, the per-session mailbox ring,
/// and the ask graph. The mailbox lives here — not on the pane — because
/// tuios's ring is per session and a pane's inbox is a view over it.
#[derive(Debug)]
pub struct PhlowSession {
    name: String,
    windows: HashMap<u64, AgentPane>,
    focused: Option<u64>,
    next_window_id: u64,
    mailbox: Mailbox,
    ask_graph: AskGraph,
}

impl PhlowSession {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            windows: HashMap::new(),
            focused: Option::None,
            next_window_id: 1,
            mailbox: Mailbox::new(),
            ask_graph: AskGraph::new(),
        }
    }

    fn resolve_window(&self, window: &str) -> Result<u64, TuiosError> {
        if let Ok(id) = window.parse::<u64>()
            && self.windows.contains_key(&id)
        {
            return Ok(id);
        }
        self.windows
            .iter()
            .find(|(_, pane)| pane.name == window)
            .map(|(id, _)| *id)
            .ok_or_else(|| {
                TuiosError::protocol(
                    ErrorCode::WindowNotFound,
                    format!("no window {window:?} in session {:?}", self.name),
                )
            })
    }
}

/// The daemon: the verb registry and every session it owns.
#[derive(Debug)]
pub struct SessionDaemon {
    sessions: HashMap<String, PhlowSession>,
    hooks: HookManager,
}

impl SessionDaemon {
    #[must_use]
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            hooks: HookManager::new(),
        }
    }

    /// The hook manager, for tests and embedding.
    #[must_use]
    pub fn hooks(&self) -> &HookManager {
        &self.hooks
    }

    /// Dispatch one verb call. Unknown verbs get `unknown_verb` with the
    /// available verbs and a near-miss suggestion, as probed on the real
    /// daemon.
    ///
    /// # Errors
    ///
    /// Whatever the verb's handler returns; see each verb.
    pub fn dispatch(&mut self, verb: &str, params: &Value) -> Result<Value, ProtocolError> {
        // The shape contract holds for every verb, including ping and
        // list-verbs, which take no params of their own.
        require_object_params(params)?;
        match verb {
            "ping" => {
                reject_unknown_params(params, &[])?;
                Ok(json!({"ok": true}))
            }
            "list-verbs" => {
                reject_unknown_params(params, &[])?;
                Ok(json!({"verbs": VERBS}))
            }
            "new-session" => self.verb_new_session(params),
            "close-session" => self.verb_close_session(params),
            "list-sessions" => self.verb_list_sessions(params),
            "new-window" => self.verb_new_window(params),
            "close-window" => self.verb_close_window(params),
            "focus-window" => self.verb_focus_window(params),
            "list-windows" => self.verb_list_windows(params),
            "set-agent-state" => self.verb_set_agent_state(params),
            "get-agent-state" => self.verb_get_agent_state(params),
            "send-mail" => self.verb_send_mail(params),
            "mail-list" => self.verb_mail_list(params),
            "send-agent-message" => self.verb_send_agent_message(params),
            "inbox" => self.verb_inbox(params),
            "ask-agent" => self.verb_ask_agent(params),
            "set-hook" => self.verb_set_hook(params),
            "list-hooks" => self.verb_list_hooks(params),
            _ => Err(unknown_verb(verb)),
        }
    }

    fn session_mut(&mut self, name: &str) -> Result<&mut PhlowSession, TuiosError> {
        self.sessions.get_mut(name).ok_or_else(|| {
            TuiosError::protocol(ErrorCode::SessionNotFound, format!("no session {name:?}"))
        })
    }

    fn verb_new_session(&mut self, params: &Value) -> Result<Value, ProtocolError> {
        reject_unknown_params(params, &["name"])?;
        let name = req_str(params, "name")?;
        if name.len() > SESSION_NAME_BYTES_MAX {
            return invalid_params("session name is longer than the cap");
        }
        if self.sessions.len() >= SESSIONS_MAX {
            return Err(ProtocolError::without_hint(
                ErrorCode::Internal,
                "the daemon holds as many sessions as it can",
            ));
        }
        if self.sessions.contains_key(&name) {
            return Err(ProtocolError::without_hint(
                ErrorCode::SessionExists,
                format!("session {name:?} already exists"),
            ));
        }
        self.sessions.insert(name.clone(), PhlowSession::new(&name));
        Ok(json!({"name": name}))
    }

    fn verb_close_session(&mut self, params: &Value) -> Result<Value, ProtocolError> {
        reject_unknown_params(params, &["session"])?;
        let name = req_str(params, "session")?;
        self.sessions.remove(&name).ok_or_else(|| {
            ProtocolError::without_hint(ErrorCode::SessionNotFound, format!("no session {name:?}"))
        })?;
        Ok(json!({"closed": name}))
    }

    fn verb_list_sessions(&mut self, params: &Value) -> Result<Value, ProtocolError> {
        reject_unknown_params(params, &[])?;
        let mut names: Vec<&String> = self.sessions.keys().collect();
        names.sort();
        Ok(json!({"sessions": names}))
    }

    fn verb_new_window(&mut self, params: &Value) -> Result<Value, ProtocolError> {
        reject_unknown_params(params, &["session", "name"])?;
        let session_name = req_str(params, "session")?;
        let base = opt_str(params, "name").unwrap_or_else(|| "agent".to_owned());
        if base.len() > WINDOW_NAME_BYTES_MAX {
            return invalid_params("window name is longer than the cap");
        }
        let hooks = self.hooks.clone();
        let session = self.session_mut(&session_name)?;
        if session.windows.len() >= WINDOWS_PER_SESSION_MAX {
            return Err(ProtocolError::without_hint(
                ErrorCode::Internal,
                "the session holds as many windows as it can",
            ));
        }
        let name = unique_window_name(session, &base);
        let id = session.next_window_id;
        session.next_window_id += 1;
        session.windows.insert(
            id,
            AgentPane {
                name: name.clone(),
                id,
                agent: AgentPaneState::new(),
            },
        );
        if session.focused.is_none() {
            session.focused = Some(id);
        }
        hooks.fire(
            HookEvent::AfterNewWindow,
            &HookContext {
                window_id: id.to_string(),
                window_name: name.clone(),
                session_id: session.name.clone(),
                event: Some(HookEvent::AfterNewWindow),
                ..Default::default()
            },
        );
        Ok(json!({"window": name, "id": id}))
    }

    fn verb_close_window(&mut self, params: &Value) -> Result<Value, ProtocolError> {
        reject_unknown_params(params, &["session", "window"])?;
        let session_name = req_str(params, "session")?;
        let window = req_str(params, "window")?;
        let hooks = self.hooks.clone();
        let session = self.session_mut(&session_name)?;
        let id = session.resolve_window(&window)?;
        let pane = session.windows.remove(&id).ok_or_else(|| {
            ProtocolError::without_hint(ErrorCode::WindowNotFound, format!("no window {window:?}"))
        })?;
        if session.focused == Some(id) {
            session.focused = session.windows.keys().next().copied();
        }
        hooks.fire(
            HookEvent::AfterCloseWindow,
            &HookContext {
                window_id: id.to_string(),
                window_name: pane.name.clone(),
                session_id: session.name.clone(),
                event: Some(HookEvent::AfterCloseWindow),
                ..Default::default()
            },
        );
        Ok(json!({"closed": pane.name}))
    }

    fn verb_focus_window(&mut self, params: &Value) -> Result<Value, ProtocolError> {
        reject_unknown_params(params, &["session", "window"])?;
        let session_name = req_str(params, "session")?;
        let window = req_str(params, "window")?;
        let hooks = self.hooks.clone();
        let session = self.session_mut(&session_name)?;
        let id = session.resolve_window(&window)?;
        let previous = session.focused;
        session.focused = Some(id);
        let name = session.windows[&id].name.clone();
        if previous != Some(id) {
            hooks.fire(
                HookEvent::AfterFocusChange,
                &HookContext {
                    window_id: id.to_string(),
                    window_name: name.clone(),
                    session_id: session.name.clone(),
                    event: Some(HookEvent::AfterFocusChange),
                    ..Default::default()
                },
            );
        }
        Ok(json!({"focused": name}))
    }

    fn verb_list_windows(&mut self, params: &Value) -> Result<Value, ProtocolError> {
        reject_unknown_params(params, &["session"])?;
        let session_name = req_str(params, "session")?;
        let session = self.session_mut(&session_name)?;
        let mut windows: Vec<Value> = session
            .windows
            .values()
            .map(|pane| {
                json!({
                    "id": pane.id,
                    "name": pane.name,
                    "state": pane.agent.state().name(),
                    "needs_you": pane.agent.state().needs_you(),
                    "focused": Some(pane.id) == session.focused,
                })
            })
            .collect();
        windows.sort_by(|a, b| a["id"].as_u64().cmp(&b["id"].as_u64()));
        Ok(json!({"windows": windows}))
    }

    fn verb_set_agent_state(&mut self, params: &Value) -> Result<Value, ProtocolError> {
        reject_unknown_params(
            params,
            &["session", "window", "state", "message", "kind", "harness"],
        )?;
        let session_name = req_str(params, "session")?;
        let window = opt_str(params, "window");
        let state_name = req_str(params, "state")?;
        let state = AgentState::parse(&state_name).ok_or_else(|| {
            ProtocolError::without_hint(
                ErrorCode::InvalidParams,
                format!("unknown agent state {state_name:?}"),
            )
        })?;
        let message = opt_str(params, "message").unwrap_or_default();
        let kind = opt_str(params, "kind").unwrap_or_default();
        let harness = opt_str(params, "harness").unwrap_or_default();
        let hooks = self.hooks.clone();
        let session = self.session_mut(&session_name)?;
        let id = match window {
            Some(name) => session.resolve_window(&name)?,
            Option::None => session.focused.ok_or_else(|| {
                ProtocolError::without_hint(
                    ErrorCode::WindowNotFound,
                    "the session has no windows to address",
                )
            })?,
        };
        let pane = session.windows.get_mut(&id).ok_or_else(|| {
            ProtocolError::without_hint(ErrorCode::WindowNotFound, "window vanished mid-call")
        })?;
        let previous = pane.agent.state();
        pane.agent.set(state, &message, &kind, &harness);
        let pane_name = pane.name.clone();
        hooks.fire(
            HookEvent::AfterAgentState,
            &HookContext {
                window_id: id.to_string(),
                window_name: pane_name.clone(),
                session_id: session_name.clone(),
                event: Some(HookEvent::AfterAgentState),
                agent_state: state.name().to_owned(),
                prev_agent_state: previous.name().to_owned(),
                agent_harness: harness.clone(),
                agent_message: message.clone(),
                ..Default::default()
            },
        );
        Ok(json!({"window": pane_name, "state": state.name()}))
    }

    fn verb_get_agent_state(&mut self, params: &Value) -> Result<Value, ProtocolError> {
        reject_unknown_params(params, &["session", "window"])?;
        let session_name = req_str(params, "session")?;
        let window = opt_str(params, "window");
        let session = self.session_mut(&session_name)?;
        let id = match window {
            Some(name) => session.resolve_window(&name)?,
            Option::None => session.focused.ok_or_else(|| {
                ProtocolError::without_hint(
                    ErrorCode::WindowNotFound,
                    "the session has no windows to address",
                )
            })?,
        };
        let pane = &session.windows[&id];
        Ok(json!({
            "window": pane.name,
            "state": pane.agent.state().name(),
            "needs_you": pane.agent.state().needs_you(),
            "message": pane.agent.message(),
            "kind": pane.agent.kind(),
            "harness": pane.agent.harness(),
        }))
    }

    /// Session-wide notice. `from` names a window id; the socket never vouches
    /// for human origin, so `from: "human"` is refused with `forbidden` by the
    /// mailbox — the person's client must use a path that does vouch.
    fn verb_send_mail(&mut self, params: &Value) -> Result<Value, ProtocolError> {
        reject_unknown_params(params, &["session", "from", "subject", "text"])?;
        let session_name = req_str(params, "session")?;
        let from = req_str(params, "from")?;
        let subject = opt_str(params, "subject").unwrap_or_default();
        let text = req_str(params, "text")?;
        let session = self.session_mut(&session_name)?;
        let message_id = session.mailbox.send(
            MessageKind::Notice,
            &from,
            Option::None,
            &subject,
            &text,
            &[],
            false,
            Instant::now(),
        )?;
        Ok(json!({"message_id": message_id}))
    }

    fn verb_mail_list(&mut self, params: &Value) -> Result<Value, ProtocolError> {
        reject_unknown_params(params, &["session", "limit"])?;
        let session_name = req_str(params, "session")?;
        let limit = opt_limit(params)?;
        let session = self.session_mut(&session_name)?;
        let messages = session.mailbox.read(&ReadFilter {
            inbox: Option::None,
            notices: true,
            peek: true,
            limit: Some(limit),
            ..Default::default()
        });
        Ok(json!({"messages": messages.iter().map(message_json).collect::<Vec<_>>()}))
    }

    /// A directed message between agent panes. Both ends must be windows of
    /// the session; `from: "human"` is refused with `forbidden`, because the
    /// socket does not vouch for human origin.
    fn verb_send_agent_message(&mut self, params: &Value) -> Result<Value, ProtocolError> {
        reject_unknown_params(params, &["session", "from", "to", "subject", "text"])?;
        let session_name = req_str(params, "session")?;
        let from = req_str(params, "from")?;
        let to = req_str(params, "to")?;
        let subject = opt_str(params, "subject").unwrap_or_default();
        let text = req_str(params, "text")?;
        if from == HUMAN {
            return Err(ProtocolError::without_hint(
                ErrorCode::Forbidden,
                "only the person may send as human; the socket does not vouch for human origin",
            ));
        }
        let session = self.session_mut(&session_name)?;
        let from_id = parse_window_id(&from, &session_name)?;
        let to_id = parse_window_id(&to, &session_name)?;
        for (label, id) in [("from", from_id), ("to", to_id)] {
            if !session.windows.contains_key(&id) {
                return Err(ProtocolError::without_hint(
                    ErrorCode::WindowNotFound,
                    format!("{label} window {id} is not in session {session_name:?}"),
                ));
            }
        }
        let message_id = session.mailbox.send(
            MessageKind::Direct,
            &from,
            Some(&to),
            &subject,
            &text,
            &[],
            false,
            Instant::now(),
        )?;
        Ok(json!({"message_id": message_id, "to": to}))
    }

    /// One pane's inbox: its directed mail plus session notices. Ask records
    /// are session history, not inbox mail, and never appear here.
    fn verb_inbox(&mut self, params: &Value) -> Result<Value, ProtocolError> {
        reject_unknown_params(
            params,
            &["session", "window", "unread", "mark_read", "limit"],
        )?;
        let session_name = req_str(params, "session")?;
        let window = req_str(params, "window")?;
        let unread = opt_bool(params, "unread").unwrap_or(false);
        let mark_read = opt_bool(params, "mark_read").unwrap_or(true);
        let limit = opt_limit(params)?;
        let session = self.session_mut(&session_name)?;
        let id = session.resolve_window(&window)?;
        let inbox_id = id.to_string();
        let messages = session.mailbox.read(&ReadFilter {
            inbox: Some(&inbox_id),
            unread,
            notices: true,
            peek: !mark_read,
            limit: Some(limit),
            ..Default::default()
        });
        Ok(json!({"messages": messages.iter().map(message_json).collect::<Vec<_>>()}))
    }

    /// Ask one agent pane a question. This is delivery, not a blocking wait:
    /// the question lands in the target's inbox as a directed message and an
    /// ask record joins the session history, and the call returns. A live
    /// agent binding could hold the ask open across the wait; here the edge
    /// opens for the delivery and closes with it, so the cycle check guards
    /// the only window where a cycle can form. `timeout_secs` is accepted for
    /// wire compatibility and validated, but nothing waits.
    fn verb_ask_agent(&mut self, params: &Value) -> Result<Value, ProtocolError> {
        reject_unknown_params(
            params,
            &["session", "from", "to", "question", "timeout_secs"],
        )?;
        let session_name = req_str(params, "session")?;
        let from = req_str(params, "from")?;
        let to = req_str(params, "to")?;
        let question = req_str(params, "question")?;
        let timeout_secs = opt_u64(params, "timeout_secs").unwrap_or(60);
        if question.len() > ASK_QUESTION_BYTES_MAX {
            return invalid_params("question is longer than the cap");
        }
        if timeout_secs > ASK_TIMEOUT_SECS_MAX {
            return invalid_params("timeout_secs is past the cap");
        }
        if from == HUMAN || to == HUMAN {
            return Err(ProtocolError::without_hint(
                ErrorCode::Forbidden,
                "ask ends must be agent windows; the socket does not vouch for human origin",
            ));
        }
        let from_id = parse_window_id(&from, &session_name)?;
        let to_id = parse_window_id(&to, &session_name)?;
        let now = Instant::now();
        let session = self.session_mut(&session_name)?;
        for (label, id) in [("from", from_id), ("to", to_id)] {
            if !session.windows.contains_key(&id) {
                return Err(ProtocolError::without_hint(
                    ErrorCode::WindowNotFound,
                    format!("{label} window {id} is not in session {session_name:?}"),
                ));
            }
        }
        let target_state = session.windows[&to_id].agent.state();
        session.ask_graph.open_ask(&from, &to)?;
        let delivered = session.mailbox.send(
            MessageKind::Direct,
            &from,
            Some(&to),
            "ask",
            &question,
            &[],
            false,
            now,
        );
        let ask_id = match delivered {
            Ok(_) => {
                let ask_id = session.mailbox.record(
                    MessageKind::Ask,
                    &from,
                    Some(&to),
                    &question,
                    "",
                    Some("delivered"),
                )?;
                session.ask_graph.close_ask(&from, &to);
                ask_id
            }
            Err(err) => {
                session.ask_graph.close_ask(&from, &to);
                return Err(err.into());
            }
        };
        Ok(json!({
            "ask_id": ask_id,
            "delivered": true,
            "state": target_state.name(),
        }))
    }

    fn verb_set_hook(&mut self, params: &Value) -> Result<Value, ProtocolError> {
        reject_unknown_params(params, &["event", "argv"])?;
        let event_name = req_str(params, "event")?;
        let event = HookEvent::parse(&event_name).ok_or_else(|| {
            ProtocolError::without_hint(
                ErrorCode::InvalidParams,
                format!("unknown hook event {event_name:?}"),
            )
        })?;
        let argv = params
            .get("argv")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                ProtocolError::without_hint(
                    ErrorCode::InvalidParams,
                    "argv must be an array of strings",
                )
            })?;
        let mut command = Vec::with_capacity(argv.len());
        for arg in argv {
            command.push(
                arg.as_str()
                    .ok_or_else(|| {
                        ProtocolError::without_hint(
                            ErrorCode::InvalidParams,
                            "argv must be an array of strings",
                        )
                    })?
                    .to_owned(),
            );
        }
        self.hooks.register(event, command)?;
        Ok(json!({"event": event.name(), "registered": true}))
    }

    fn verb_list_hooks(&mut self, params: &Value) -> Result<Value, ProtocolError> {
        reject_unknown_params(params, &[])?;
        let registered = self.hooks.registered();
        let mut events: Vec<Value> = registered
            .iter()
            .map(|(event, commands)| json!({"event": event.name(), "commands": commands}))
            .collect();
        events.sort_by(|a, b| a["event"].as_str().cmp(&b["event"].as_str()));
        Ok(json!({"hooks": events}))
    }
}

impl Default for SessionDaemon {
    fn default() -> Self {
        Self::new()
    }
}

/// The verb registry, in stable order. Part of the protocol surface.
pub const VERBS: &[&str] = &[
    "ping",
    "list-verbs",
    "new-session",
    "close-session",
    "list-sessions",
    "new-window",
    "close-window",
    "focus-window",
    "list-windows",
    "set-agent-state",
    "get-agent-state",
    "send-mail",
    "mail-list",
    "send-agent-message",
    "inbox",
    "ask-agent",
    "set-hook",
    "list-hooks",
];

fn unknown_verb(verb: &str) -> ProtocolError {
    let available: Vec<String> = VERBS.iter().map(ToString::to_string).collect();
    let mut hint = VerbHint {
        verb: Some(verb.to_owned()),
        available: available.clone(),
        command: Some("list-verbs".to_owned()),
        detail: Some("call list-verbs for the full registry".to_owned()),
        ..Default::default()
    };
    if let Some(suggestion) = closest_verb(verb, &available) {
        hint.did_you_mean = Some(suggestion.to_owned());
    }
    ProtocolError::new(ErrorCode::UnknownVerb, format!("unknown verb {verb}"), hint)
}

fn invalid_params(message: &str) -> Result<Value, ProtocolError> {
    Err(ProtocolError::without_hint(
        ErrorCode::InvalidParams,
        message,
    ))
}

/// Refuse unknown parameter names: a misspelled param is a bug, and silently
/// ignoring it would hide the bug. Params must be absent, null, or an
/// object; any other JSON type is rejected, matching the probed daemon.
fn require_object_params(params: &Value) -> Result<(), ProtocolError> {
    if params.is_null() || params.is_object() {
        return Ok(());
    }
    Err(ProtocolError::without_hint(
        ErrorCode::InvalidParams,
        "params must be an object",
    ))
}

fn reject_unknown_params(params: &Value, allowed: &[&str]) -> Result<(), ProtocolError> {
    require_object_params(params)?;
    if let Value::Object(object) = params {
        for key in object.keys() {
            if !allowed.contains(&key.as_str()) {
                let hint = VerbHint {
                    param: Some(key.clone()),
                    accepted: allowed.iter().map(ToString::to_string).collect(),
                    ..Default::default()
                };
                return Err(ProtocolError::new(
                    ErrorCode::InvalidParams,
                    format!("unknown parameter {key:?}"),
                    hint,
                ));
            }
        }
    }
    Ok(())
}

fn req_str(params: &Value, key: &str) -> Result<String, ProtocolError> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            ProtocolError::without_hint(
                ErrorCode::InvalidParams,
                format!("missing or non-string parameter {key:?}"),
            )
        })
}

fn opt_str(params: &Value, key: &str) -> Option<String> {
    params.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn opt_bool(params: &Value, key: &str) -> Option<bool> {
    params.get(key).and_then(Value::as_bool)
}

fn opt_u64(params: &Value, key: &str) -> Option<u64> {
    params.get(key).and_then(Value::as_u64)
}

fn opt_limit(params: &Value) -> Result<usize, ProtocolError> {
    let limit = params
        .get("limit")
        .and_then(Value::as_u64)
        .map_or(READ_DEFAULT_LIMIT, |n| n as usize);
    if limit > READ_LIMIT_MAX {
        return Err(ProtocolError::without_hint(
            ErrorCode::InvalidParams,
            "limit is past the cap",
        ));
    }
    Ok(limit)
}

fn parse_window_id(window: &str, session: &str) -> Result<u64, ProtocolError> {
    window.parse::<u64>().map_err(|_| {
        ProtocolError::without_hint(
            ErrorCode::InvalidParams,
            format!("{window:?} is not a window id in session {session:?}; pass the numeric id"),
        )
    })
}

/// A fresh name from `base`: the base when free, else `base-2`, `base-3`,
/// ... The window cap bounds the search, and the session's window ids are
/// unique, so the id-suffixed fallback always terminates with a free name.
fn unique_window_name(session: &PhlowSession, base: &str) -> String {
    if !session.windows.values().any(|pane| pane.name == base) {
        return base.to_owned();
    }
    for n in 2..=WINDOWS_PER_SESSION_MAX {
        let candidate = format!("{base}-{n}");
        if !session.windows.values().any(|pane| pane.name == candidate) {
            return candidate;
        }
    }
    format!("{base}-{}", session.next_window_id)
}

fn message_json(message: &Message) -> Value {
    let kind = match message.kind() {
        MessageKind::Direct => "direct",
        MessageKind::Notice => "notice",
        MessageKind::Ask => "ask",
    };
    json!({
        "id": message.id(),
        "kind": kind,
        "from": message.from(),
        "to": message.to(),
        "subject": message.subject(),
        "text": message.text(),
        "thread_id": message.thread_id(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(daemon: &mut SessionDaemon, verb: &str, params: Value) -> Result<Value, ProtocolError> {
        daemon.dispatch(verb, &params)
    }

    fn new_session(daemon: &mut SessionDaemon, name: &str) {
        call(daemon, "new-session", json!({"name": name})).expect("new-session");
    }

    fn new_window(daemon: &mut SessionDaemon, session: &str, name: &str) -> u64 {
        let result = call(
            daemon,
            "new-window",
            json!({"session": session, "name": name}),
        )
        .expect("new-window");
        result["id"].as_u64().expect("window id")
    }

    fn agent_state(daemon: &mut SessionDaemon, session: &str, window: &str) -> String {
        call(
            daemon,
            "get-agent-state",
            json!({"session": session, "window": window}),
        )
        .expect("get-agent-state")["state"]
            .as_str()
            .expect("state")
            .to_owned()
    }

    // --- validation ---

    #[test]
    fn ping_and_list_verbs() {
        let mut daemon = SessionDaemon::new();
        let pong = call(&mut daemon, "ping", Value::Null).expect("ping");
        assert_eq!(pong["ok"], json!(true));
        let verbs = call(&mut daemon, "list-verbs", Value::Null).expect("list-verbs");
        let names: Vec<&str> = verbs["verbs"]
            .as_array()
            .expect("array")
            .iter()
            .map(|v| v.as_str().expect("str"))
            .collect();
        assert_eq!(names, VERBS);
    }

    #[test]
    fn session_lifecycle() {
        let mut daemon = SessionDaemon::new();
        new_session(&mut daemon, "s");
        let listed = call(&mut daemon, "list-sessions", Value::Null).expect("list");
        assert_eq!(listed["sessions"], json!(["s"]));
        let closed = call(&mut daemon, "close-session", json!({"session": "s"})).expect("close");
        assert_eq!(closed["closed"], json!("s"));
        let listed = call(&mut daemon, "list-sessions", Value::Null).expect("list");
        assert_eq!(listed["sessions"], json!([]));
    }

    #[test]
    fn window_lifecycle_and_focus() {
        let mut daemon = SessionDaemon::new();
        new_session(&mut daemon, "s");
        let a = new_window(&mut daemon, "s", "a");
        let b = new_window(&mut daemon, "s", "b");
        assert_ne!(a, b);
        // First window is focused by default.
        let windows = call(&mut daemon, "list-windows", json!({"session": "s"})).expect("list");
        assert_eq!(windows["windows"][0]["focused"], json!(true));
        call(
            &mut daemon,
            "focus-window",
            json!({"session": "s", "window": "b"}),
        )
        .expect("focus");
        let windows = call(&mut daemon, "list-windows", json!({"session": "s"})).expect("list");
        let focused: Vec<&str> = windows["windows"]
            .as_array()
            .expect("array")
            .iter()
            .filter(|w| w["focused"].as_bool().unwrap_or(false))
            .map(|w| w["name"].as_str().expect("name"))
            .collect();
        assert_eq!(focused, ["b"]);
        call(
            &mut daemon,
            "close-window",
            json!({"session": "s", "window": "a"}),
        )
        .expect("close");
        let windows = call(&mut daemon, "list-windows", json!({"session": "s"})).expect("list");
        assert_eq!(windows["windows"].as_array().expect("array").len(), 1);
    }

    #[test]
    fn duplicate_window_name_gets_a_suffix() {
        let mut daemon = SessionDaemon::new();
        new_session(&mut daemon, "s");
        new_window(&mut daemon, "s", "agent");
        let result = call(
            &mut daemon,
            "new-window",
            json!({"session": "s", "name": "agent"}),
        )
        .expect("second");
        assert_eq!(result["window"], json!("agent-2"));
    }

    #[test]
    fn agent_state_round_trip_with_block_note() {
        let mut daemon = SessionDaemon::new();
        new_session(&mut daemon, "s");
        new_window(&mut daemon, "s", "a");
        call(
            &mut daemon,
            "set-agent-state",
            json!({
                "session": "s", "window": "a", "state": "needs_input",
                "message": "approve?", "kind": "approval", "harness": "claude",
            }),
        )
        .expect("set");
        let got = call(
            &mut daemon,
            "get-agent-state",
            json!({"session": "s", "window": "a"}),
        )
        .expect("get");
        assert_eq!(got["state"], json!("needs_input"));
        assert_eq!(got["needs_you"], json!(true));
        assert_eq!(got["message"], json!("approve?"));
        assert_eq!(got["kind"], json!("approval"));
        assert_eq!(got["harness"], json!("claude"));
        // Leaving the blocking state clears the note.
        call(
            &mut daemon,
            "set-agent-state",
            json!({"session": "s", "window": "a", "state": "working"}),
        )
        .expect("set");
        let got = call(
            &mut daemon,
            "get-agent-state",
            json!({"session": "s", "window": "a"}),
        )
        .expect("get");
        assert_eq!(got["message"], json!(""));
    }

    #[test]
    fn set_agent_state_defaults_to_the_focused_window() {
        let mut daemon = SessionDaemon::new();
        new_session(&mut daemon, "s");
        new_window(&mut daemon, "s", "a");
        call(
            &mut daemon,
            "set-agent-state",
            json!({"session": "s", "state": "idle"}),
        )
        .expect("set");
        assert_eq!(agent_state(&mut daemon, "s", "a"), "idle");
    }

    #[test]
    fn mail_notice_is_session_visible() {
        let mut daemon = SessionDaemon::new();
        new_session(&mut daemon, "s");
        let a = new_window(&mut daemon, "s", "a");
        let sent = call(
            &mut daemon,
            "send-mail",
            json!({"session": "s", "from": a.to_string(), "text": "hello all"}),
        )
        .expect("send-mail");
        assert!(sent["message_id"].as_u64().is_some());
        let listed = call(&mut daemon, "mail-list", json!({"session": "s"})).expect("mail-list");
        let messages = listed["messages"].as_array().expect("array");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["kind"], json!("notice"));
        assert_eq!(messages[0]["text"], json!("hello all"));
    }

    #[test]
    fn agent_message_reaches_the_target_inbox_only() {
        let mut daemon = SessionDaemon::new();
        new_session(&mut daemon, "s");
        let a = new_window(&mut daemon, "s", "a");
        let b = new_window(&mut daemon, "s", "b");
        call(
            &mut daemon,
            "send-agent-message",
            json!({
                "session": "s",
                "from": a.to_string(), "to": b.to_string(),
                "subject": "ping", "text": "are you there",
            }),
        )
        .expect("send");
        let inbox_b = call(
            &mut daemon,
            "inbox",
            json!({"session": "s", "window": b.to_string()}),
        )
        .expect("inbox b");
        assert_eq!(inbox_b["messages"].as_array().expect("array").len(), 1);
        let inbox_a = call(
            &mut daemon,
            "inbox",
            json!({"session": "s", "window": a.to_string()}),
        )
        .expect("inbox a");
        assert!(inbox_a["messages"].as_array().expect("array").is_empty());
    }

    #[test]
    fn inbox_unread_and_mark_read() {
        let mut daemon = SessionDaemon::new();
        new_session(&mut daemon, "s");
        let a = new_window(&mut daemon, "s", "a");
        let b = new_window(&mut daemon, "s", "b");
        for text in ["one", "two"] {
            call(
                &mut daemon,
                "send-agent-message",
                json!({
                    "session": "s",
                    "from": a.to_string(), "to": b.to_string(), "text": text,
                }),
            )
            .expect("send");
        }
        // Peek without marking: both stay unread.
        let peeked = call(
            &mut daemon,
            "inbox",
            json!({"session": "s", "window": b.to_string(), "mark_read": false}),
        )
        .expect("peek");
        assert_eq!(peeked["messages"].as_array().expect("array").len(), 2);
        let unread = call(
            &mut daemon,
            "inbox",
            json!({"session": "s", "window": b.to_string(), "unread": true, "mark_read": false}),
        )
        .expect("unread");
        assert_eq!(unread["messages"].as_array().expect("array").len(), 2);
        // A marking read drains the unread view.
        call(
            &mut daemon,
            "inbox",
            json!({"session": "s", "window": b.to_string()}),
        )
        .expect("read");
        let unread = call(
            &mut daemon,
            "inbox",
            json!({"session": "s", "window": b.to_string(), "unread": true, "mark_read": false}),
        )
        .expect("unread");
        assert!(unread["messages"].as_array().expect("array").is_empty());
    }

    #[test]
    fn ask_agent_delivers_and_records() {
        let mut daemon = SessionDaemon::new();
        new_session(&mut daemon, "s");
        let a = new_window(&mut daemon, "s", "a");
        let b = new_window(&mut daemon, "s", "b");
        let asked = call(
            &mut daemon,
            "ask-agent",
            json!({
                "session": "s",
                "from": a.to_string(), "to": b.to_string(),
                "question": "what is the status?",
            }),
        )
        .expect("ask");
        assert_eq!(asked["delivered"], json!(true));
        // The question is in the target's inbox.
        let inbox = call(
            &mut daemon,
            "inbox",
            json!({"session": "s", "window": b.to_string()}),
        )
        .expect("inbox");
        let messages = inbox["messages"].as_array().expect("array");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["text"], json!("what is the status?"));
        // The ask record is session history, not inbox mail: it shows in
        // mail-list with kind "ask".
        let listed = call(&mut daemon, "mail-list", json!({"session": "s"})).expect("mail-list");
        let kinds: Vec<&str> = listed["messages"]
            .as_array()
            .expect("array")
            .iter()
            .map(|m| m["kind"].as_str().expect("kind"))
            .collect();
        assert!(kinds.contains(&"ask"), "{kinds:?}");
    }

    #[test]
    fn hook_registration_lists_back() {
        let mut daemon = SessionDaemon::new();
        call(
            &mut daemon,
            "set-hook",
            json!({"event": "after-new-window", "argv": ["/bin/true"]}),
        )
        .expect("set-hook");
        let listed = call(&mut daemon, "list-hooks", Value::Null).expect("list-hooks");
        let hooks = listed["hooks"].as_array().expect("array");
        assert_eq!(hooks.len(), 1);
        assert_eq!(hooks[0]["event"], json!("after-new-window"));
    }

    #[test]
    fn new_window_fires_after_new_window_hook() {
        let mut daemon = SessionDaemon::new();
        new_session(&mut daemon, "s");
        let log = std::env::temp_dir().join(format!("phlow-tuios-hook-{}.log", std::process::id()));
        let _ = std::fs::remove_file(&log);
        call(
            &mut daemon,
            "set-hook",
            json!({
                "event": "after-new-window",
                "argv": ["sh", "-c", format!("echo fired >> {}", log.display())],
            }),
        )
        .expect("set-hook");
        new_window(&mut daemon, "s", "hooked");
        // Hooks run on spawned threads; wait for the log line.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let body = std::fs::read_to_string(&log).unwrap_or_default();
            if body.contains("fired") {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "hook never fired within 10s"
            );
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let _ = std::fs::remove_file(&log);
    }

    #[test]
    fn close_session_drops_its_windows_and_mail() {
        let mut daemon = SessionDaemon::new();
        new_session(&mut daemon, "s");
        new_window(&mut daemon, "s", "a");
        call(&mut daemon, "close-session", json!({"session": "s"})).expect("close");
        let err = call(&mut daemon, "list-windows", json!({"session": "s"}))
            .expect_err("windows of a closed session");
        assert_eq!(err.code, ErrorCode::SessionNotFound);
        // The name is reusable immediately.
        new_session(&mut daemon, "s");
        let listed = call(&mut daemon, "list-windows", json!({"session": "s"})).expect("list");
        assert!(listed["windows"].as_array().expect("array").is_empty());
    }

    #[test]
    fn list_windows_reports_needs_you() {
        let mut daemon = SessionDaemon::new();
        new_session(&mut daemon, "s");
        new_window(&mut daemon, "s", "a");
        new_window(&mut daemon, "s", "b");
        call(
            &mut daemon,
            "set-agent-state",
            json!({"session": "s", "window": "a", "state": "errored"}),
        )
        .expect("set");
        let windows = call(&mut daemon, "list-windows", json!({"session": "s"})).expect("list");
        let needy: Vec<&str> = windows["windows"]
            .as_array()
            .expect("array")
            .iter()
            .filter(|w| w["needs_you"].as_bool().unwrap_or(false))
            .map(|w| w["name"].as_str().expect("name"))
            .collect();
        assert_eq!(needy, ["a"]);
    }

    #[test]
    fn get_agent_state_defaults_to_the_focused_window() {
        let mut daemon = SessionDaemon::new();
        new_session(&mut daemon, "s");
        new_window(&mut daemon, "s", "a");
        new_window(&mut daemon, "s", "b");
        call(
            &mut daemon,
            "set-agent-state",
            json!({"session": "s", "state": "done"}),
        )
        .expect("set");
        // "a" was focused first and focus never moved: the default read hits it.
        assert_eq!(agent_state(&mut daemon, "s", "a"), "done");
        assert_eq!(agent_state(&mut daemon, "s", "b"), "none");
    }

    #[test]
    fn ask_agent_reports_the_target_state() {
        let mut daemon = SessionDaemon::new();
        new_session(&mut daemon, "s");
        let a = new_window(&mut daemon, "s", "a");
        let b = new_window(&mut daemon, "s", "b");
        call(
            &mut daemon,
            "set-agent-state",
            json!({"session": "s", "window": b.to_string(), "state": "working"}),
        )
        .expect("set");
        let asked = call(
            &mut daemon,
            "ask-agent",
            json!({
                "session": "s",
                "from": a.to_string(), "to": b.to_string(),
                "question": "status?",
            }),
        )
        .expect("ask");
        assert_eq!(asked["state"], json!("working"));
    }

    #[test]
    fn null_and_absent_params_are_accepted() {
        let mut daemon = SessionDaemon::new();
        let result = call(&mut daemon, "list-verbs", Value::Null).expect("null params");
        assert!(result["verbs"].is_array(), "null params mean no params");
        let result = call(&mut daemon, "list-verbs", json!({})).expect("empty object");
        assert!(result["verbs"].is_array(), "empty object means no params");
        let pong = call(&mut daemon, "ping", json!({})).expect("ping with empty object");
        assert_eq!(pong["ok"], json!(true));
    }

    // --- adversarial ---

    #[test]
    fn unknown_verb_suggests_a_near_miss() {
        let mut daemon = SessionDaemon::new();
        let err = call(&mut daemon, "frobnicate", Value::Null).expect_err("unknown verb");
        assert_eq!(err.code, ErrorCode::UnknownVerb);
        let response = err.into_response(Option::None);
        let encoded = String::from_utf8(response.encode()).expect("utf8");
        assert!(encoded.contains("unknown_verb"), "{encoded}");
        assert!(encoded.contains("list-verbs"), "{encoded}");
    }

    #[test]
    fn near_miss_verb_names_the_suggestion() {
        let mut daemon = SessionDaemon::new();
        let err = call(&mut daemon, "list-verbz", Value::Null).expect_err("near miss");
        let response = err.into_response(Option::None);
        let encoded = String::from_utf8(response.encode()).expect("utf8");
        assert!(
            encoded.contains(r#""did_you_mean":"list-verbs""#),
            "{encoded}"
        );
    }

    #[test]
    fn unknown_param_is_refused() {
        let mut daemon = SessionDaemon::new();
        let err = call(
            &mut daemon,
            "list-windows",
            json!({"session": "s", "bogus": 1}),
        )
        .expect_err("unknown param");
        assert_eq!(err.code, ErrorCode::InvalidParams);
    }

    #[test]
    fn non_object_params_are_rejected() {
        let mut daemon = SessionDaemon::new();
        for params in [json!("oops"), json!(["oops"]), json!(42), json!(true)] {
            let err = call(&mut daemon, "list-verbs", params).expect_err("non-object params");
            assert_eq!(
                err.code,
                ErrorCode::InvalidParams,
                "non-object params refused"
            );
        }
    }

    #[test]
    fn zero_param_verbs_reject_unknown_names() {
        let mut daemon = SessionDaemon::new();
        for verb in ["ping", "list-verbs"] {
            let err = call(&mut daemon, verb, json!({"bogus": 1})).expect_err("unknown name");
            assert_eq!(
                err.code,
                ErrorCode::InvalidParams,
                "{verb} refuses named params"
            );
        }
    }

    #[test]
    fn duplicate_new_session_is_session_exists() {
        let mut daemon = SessionDaemon::new();
        new_session(&mut daemon, "s");
        let err = call(&mut daemon, "new-session", json!({"name": "s"})).expect_err("duplicate");
        assert_eq!(err.code, ErrorCode::SessionExists);
    }

    #[test]
    fn missing_session_is_session_not_found() {
        let mut daemon = SessionDaemon::new();
        let err = call(&mut daemon, "list-windows", json!({"session": "nope"}))
            .expect_err("missing session");
        assert_eq!(err.code, ErrorCode::SessionNotFound);
    }

    #[test]
    fn missing_window_is_window_not_found() {
        let mut daemon = SessionDaemon::new();
        new_session(&mut daemon, "s");
        let err = call(
            &mut daemon,
            "close-window",
            json!({"session": "s", "window": "ghost"}),
        )
        .expect_err("missing window");
        assert_eq!(err.code, ErrorCode::WindowNotFound);
    }

    #[test]
    fn message_to_missing_window_is_window_not_found() {
        let mut daemon = SessionDaemon::new();
        new_session(&mut daemon, "s");
        let a = new_window(&mut daemon, "s", "a");
        let err = call(
            &mut daemon,
            "send-agent-message",
            json!({
                "session": "s",
                "from": a.to_string(), "to": "999", "text": "hi",
            }),
        )
        .expect_err("missing target");
        assert_eq!(err.code, ErrorCode::WindowNotFound);
    }

    #[test]
    fn bad_agent_state_is_invalid_params() {
        let mut daemon = SessionDaemon::new();
        new_session(&mut daemon, "s");
        new_window(&mut daemon, "s", "a");
        let err = call(
            &mut daemon,
            "set-agent-state",
            json!({"session": "s", "window": "a", "state": "frobnicating"}),
        )
        .expect_err("bad state");
        assert_eq!(err.code, ErrorCode::InvalidParams);
    }

    #[test]
    fn overlong_mail_text_is_invalid_params() {
        let mut daemon = SessionDaemon::new();
        new_session(&mut daemon, "s");
        let a = new_window(&mut daemon, "s", "a");
        let err = call(
            &mut daemon,
            "send-mail",
            json!({
                "session": "s",
                "from": a.to_string(),
                "text": "x".repeat(crate::mailbox::MESSAGE_TEXT_BYTES_MAX + 1),
            }),
        )
        .expect_err("overlong text");
        assert_eq!(err.code, ErrorCode::InvalidParams);
    }

    #[test]
    fn claimed_human_from_is_forbidden() {
        let mut daemon = SessionDaemon::new();
        new_session(&mut daemon, "s");
        let a = new_window(&mut daemon, "s", "a");
        let err = call(
            &mut daemon,
            "send-agent-message",
            json!({
                "session": "s",
                "from": "human", "to": a.to_string(), "text": "trust me",
            }),
        )
        .expect_err("human spoof");
        assert_eq!(err.code, ErrorCode::Forbidden);
    }

    #[test]
    fn sender_rate_limit_eventually_bites() {
        let mut daemon = SessionDaemon::new();
        new_session(&mut daemon, "s");
        let a = new_window(&mut daemon, "s", "a");
        let b = new_window(&mut daemon, "s", "b");
        let mut limited = false;
        for _ in 0..(crate::mailbox::SEND_BURST + 5) {
            let result = call(
                &mut daemon,
                "send-agent-message",
                json!({
                    "session": "s",
                    "from": a.to_string(), "to": b.to_string(), "text": "spam",
                }),
            );
            if let Err(err) = result {
                assert_eq!(err.code, ErrorCode::RateLimited);
                limited = true;
                break;
            }
        }
        assert!(limited, "burst + refill must eventually rate-limit");
    }

    #[test]
    fn ask_self_is_loop_refused() {
        let mut daemon = SessionDaemon::new();
        new_session(&mut daemon, "s");
        let a = new_window(&mut daemon, "s", "a");
        let err = call(
            &mut daemon,
            "ask-agent",
            json!({
                "session": "s",
                "from": a.to_string(), "to": a.to_string(),
                "question": "me?",
            }),
        )
        .expect_err("self ask");
        assert_eq!(err.code, ErrorCode::LoopRefused);
    }

    #[test]
    fn client_side_hook_is_refused() {
        let mut daemon = SessionDaemon::new();
        let err = call(
            &mut daemon,
            "set-hook",
            json!({"event": "after-attach", "argv": ["/bin/true"]}),
        )
        .expect_err("client-side hook");
        assert_eq!(err.code, ErrorCode::InvalidParams);
    }

    #[test]
    fn overlong_limit_is_invalid_params() {
        let mut daemon = SessionDaemon::new();
        new_session(&mut daemon, "s");
        let err = call(
            &mut daemon,
            "mail-list",
            json!({"session": "s", "limit": READ_LIMIT_MAX as u64 + 1}),
        )
        .expect_err("overlong limit");
        assert_eq!(err.code, ErrorCode::InvalidParams);
    }

    #[test]
    fn window_id_must_be_numeric_for_message_ends() {
        let mut daemon = SessionDaemon::new();
        new_session(&mut daemon, "s");
        new_window(&mut daemon, "s", "a");
        let err = call(
            &mut daemon,
            "send-agent-message",
            json!({"session": "s", "from": "a", "to": "a", "text": "hi"}),
        )
        .expect_err("named ends");
        assert_eq!(err.code, ErrorCode::InvalidParams);
    }
}
