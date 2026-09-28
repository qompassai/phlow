//! Session tapes: small declarative scripts that build a session, adapted
//! from tuios's tape concept (`internal/tape`).
//!
//! tuios tapes script terminal demos — keystrokes, sleeps, window management
//! for playback. phlow tapes script *agent sessions*: the header declares the
//! session to build, and the commands drive windows, messages, and agent
//! states. The shape (leading header block, line commands, `Require` guards)
//! follows tuios; the command set does not, because typing keystrokes at a
//! terminal is not what a phlow session needs.
//!
//! Example:
//!
//! ```text
//! # Build the nightly session.
//! Session "nightly"
//! Require "cargo"
//!
//! NewWindow "builder"
//! SendMessage "ci" "builder" "build" "cargo build --workspace"
//! SetAgentState "builder" "working"
//! WaitForState "builder" "done" 600
//! ```
//!
//! Parsing never executes anything: a tape can be inspected (or refused)
//! before a line of it runs.

use std::time::Duration;

use crate::state::AgentState;

/// Bound: whole tape, in bytes.
pub const TAPE_BYTES_MAX: usize = 256 * 1024;
/// Bound: lines in one tape.
pub const TAPE_LINES_MAX: usize = 4096;
/// Bound: one line, in bytes.
pub const TAPE_LINE_BYTES_MAX: usize = 8 * 1024;
/// Bound: commands in one tape.
pub const TAPE_COMMANDS_MAX: usize = 1024;
/// Bound: one argument, in bytes.
pub const TAPE_ARG_BYTES_MAX: usize = 8 * 1024;
/// Bound: `Require` binary names per tape.
pub const TAPE_REQUIRES_MAX: usize = 64;
/// Bound: `WaitForState` timeout, in seconds.
pub const WAIT_TIMEOUT_SECS_MAX: u64 = 3600;
/// Bound: `Sleep` duration, in seconds.
pub const SLEEP_SECS_MAX: u64 = 3600;

/// The declarative header: leading directives before the first command.
/// Everything below it is action commands, parsed unchanged.
#[derive(Debug, Clone, Default)]
pub struct TapeHeader {
    /// Target session name.
    pub session: Option<String>,
    /// Workspace index to build in.
    pub workspace: Option<u32>,
    /// Binaries that must exist on PATH, else the tape is skipped.
    pub requires: Vec<String>,
}

/// One parsed tape command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TapeCommand {
    NewWindow {
        name: String,
    },
    CloseWindow {
        name: String,
    },
    FocusWindow {
        name: String,
    },
    SendMessage {
        from: String,
        to: String,
        subject: String,
        text: String,
    },
    SetAgentState {
        window: String,
        state: AgentState,
    },
    WaitForState {
        window: String,
        state: AgentState,
        timeout_secs: u64,
    },
    Sleep {
        secs: u64,
    },
}

/// A parsed tape: header plus commands.
#[derive(Debug, Clone)]
pub struct Tape {
    pub header: TapeHeader,
    pub commands: Vec<TapeCommand>,
    /// Source line number per command, so failures name the real line even
    /// when comments, blanks, and the header shift commands downward.
    command_lines: Vec<usize>,
}

impl Tape {
    /// The 1-based source line of `commands[index]`.
    #[must_use]
    pub fn command_line(&self, index: usize) -> usize {
        self.command_lines.get(index).copied().unwrap_or(0)
    }
}

/// A tape failure: the 1-based line, the command, and what went wrong.
#[derive(Debug, Clone)]
pub struct TapeError {
    pub line: usize,
    pub command: String,
    pub message: String,
}

impl TapeError {
    fn new(line: usize, command: &str, message: impl Into<String>) -> Self {
        Self {
            line,
            command: command.to_owned(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for TapeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "tape line {} ({}): {}",
            self.line, self.command, self.message
        )
    }
}

impl std::error::Error for TapeError {}

/// Parse a tape. The header is the run of leading lines that are blank, a
/// comment, or a recognized directive (`Session`, `Workspace`, `require`);
/// the body is everything from the first action command onward. An
/// unrecognized or malformed directive ends the header and starts the body,
/// where it fails as an unknown command — never silently.
///
/// # Errors
///
/// `TapeError` naming the line on hostile or malformed input: over-bound
/// size, unclosed quotes, unknown commands, or wrong arity.
pub fn parse(content: &str) -> Result<Tape, TapeError> {
    if content.len() > TAPE_BYTES_MAX {
        return Err(TapeError::new(0, "", "tape is larger than the size cap"));
    }
    let mut header = TapeHeader::default();
    let mut commands = Vec::new();
    let mut command_lines = Vec::new();
    let mut in_header = true;
    for (index, raw_line) in content.lines().enumerate() {
        let line_no = index + 1;
        if index >= TAPE_LINES_MAX {
            return Err(TapeError::new(
                line_no,
                "",
                "tape has more lines than the cap",
            ));
        }
        if raw_line.len() > TAPE_LINE_BYTES_MAX {
            return Err(TapeError::new(
                line_no,
                "",
                "tape line is longer than the cap",
            ));
        }
        let tokens = lex_line(raw_line).map_err(|message| TapeError::new(line_no, "", message))?;
        if tokens.is_empty() {
            continue;
        }
        if in_header && is_header_directive(&tokens[0]) {
            apply_header_directive(&mut header, &tokens)
                .map_err(|message| TapeError::new(line_no, &tokens[0], message))?;
            continue;
        }
        in_header = false;
        if commands.len() >= TAPE_COMMANDS_MAX {
            return Err(TapeError::new(
                line_no,
                "",
                "tape has more commands than the cap",
            ));
        }
        commands.push(
            parse_command(&tokens)
                .map_err(|message| TapeError::new(line_no, &tokens[0], message))?,
        );
        command_lines.push(line_no);
    }
    Ok(Tape {
        header,
        commands,
        command_lines,
    })
}

fn is_header_directive(first: &str) -> bool {
    matches!(
        first.to_ascii_lowercase().as_str(),
        "session" | "workspace" | "require"
    )
}

fn apply_header_directive(header: &mut TapeHeader, tokens: &[String]) -> Result<(), String> {
    match tokens[0].to_ascii_lowercase().as_str() {
        "session" => {
            let [_, name] = tokens else {
                return Err("Session needs exactly one argument".to_owned());
            };
            header.session = Some(name.to_owned());
            Ok(())
        }
        "workspace" => {
            let [_, index] = tokens else {
                return Err("Workspace needs exactly one argument".to_owned());
            };
            let parsed: u32 = index
                .parse()
                .map_err(|_| "Workspace needs a numeric index".to_owned())?;
            header.workspace = Some(parsed);
            Ok(())
        }
        "require" => {
            let [_, binary] = tokens else {
                return Err("Require needs exactly one argument".to_owned());
            };
            if header.requires.len() >= TAPE_REQUIRES_MAX {
                return Err("too many Require directives".to_owned());
            }
            header.requires.push(binary.to_owned());
            Ok(())
        }
        _ => unreachable!("checked by is_header_directive"),
    }
}

/// Split one line into tokens. `#` or `//` starts a comment outside a quoted
/// string; `"..."` quotes with `\"` and `\\` escapes; anything else is a bare
/// word. Every token is bounded by [`TAPE_ARG_BYTES_MAX`].
fn lex_line(line: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_token = false;
    let mut chars = line.chars().peekable();
    // Leading whitespace is not significant.
    while chars.peek().is_some_and(|c| c.is_whitespace()) {
        chars.next();
    }
    // A comment line contributes no tokens.
    if chars.peek().is_some_and(|&c| c == '#')
        || (chars.peek() == Some(&'/') && chars.clone().nth(1) == Some('/'))
    {
        return Ok(tokens);
    }
    while let Some(c) = chars.next() {
        if c == '#' && !in_token {
            break;
        }
        if c == '"' {
            in_token = true;
            let mut closed = false;
            while let Some(q) = chars.next() {
                match q {
                    '\\' => {
                        let escaped = chars.next().ok_or("unclosed quote")?;
                        match escaped {
                            '"' | '\\' => current.push(escaped),
                            'n' => current.push('\n'),
                            't' => current.push('\t'),
                            _ => {
                                return Err(format!("bad escape \\{escaped}"));
                            }
                        }
                    }
                    '"' => {
                        closed = true;
                        break;
                    }
                    _ => current.push(q),
                }
                if current.len() > TAPE_ARG_BYTES_MAX {
                    return Err("argument is longer than the cap".to_owned());
                }
            }
            if !closed {
                return Err("unclosed quote".to_owned());
            }
        } else if c.is_whitespace() {
            if in_token {
                push_token(&mut tokens, &mut current)?;
                in_token = false;
            }
        } else {
            in_token = true;
            current.push(c);
            if current.len() > TAPE_ARG_BYTES_MAX {
                return Err("argument is longer than the cap".to_owned());
            }
        }
    }
    if in_token {
        push_token(&mut tokens, &mut current)?;
    }
    Ok(tokens)
}

fn push_token(tokens: &mut Vec<String>, current: &mut String) -> Result<(), String> {
    if current.len() > TAPE_ARG_BYTES_MAX {
        return Err("argument is longer than the cap".to_owned());
    }
    tokens.push(std::mem::take(current));
    Ok(())
}

/// Parse the `waitforstate <window> <state> <timeout-secs>` tape command.
fn parse_wait_for_state(args: &[String]) -> Result<TapeCommand, String> {
    if args.len() != 3 {
        return Err(format!(
            "waitforstate needs 3 arguments, got {}",
            args.len()
        ));
    }
    let state =
        AgentState::parse(&args[1]).ok_or_else(|| format!("unknown agent state {:?}", args[1]))?;
    let timeout_secs: u64 = args[2]
        .parse()
        .map_err(|_| "WaitForState needs a numeric timeout in seconds".to_owned())?;
    if timeout_secs > WAIT_TIMEOUT_SECS_MAX {
        return Err("WaitForState timeout is past the cap".to_owned());
    }
    Ok(TapeCommand::WaitForState {
        window: args[0].clone(),
        state,
        timeout_secs,
    })
}

fn parse_command(tokens: &[String]) -> Result<TapeCommand, String> {
    let name = tokens[0].to_ascii_lowercase();
    let args = &tokens[1..];
    let arity = |want: usize| {
        if args.len() == want {
            Ok(())
        } else {
            Err(format!(
                "{} needs {want} arguments, got {}",
                tokens[0],
                args.len()
            ))
        }
    };
    match name.as_str() {
        "newwindow" => {
            arity(1)?;
            Ok(TapeCommand::NewWindow {
                name: args[0].clone(),
            })
        }
        "closewindow" => {
            arity(1)?;
            Ok(TapeCommand::CloseWindow {
                name: args[0].clone(),
            })
        }
        "focuswindow" => {
            arity(1)?;
            Ok(TapeCommand::FocusWindow {
                name: args[0].clone(),
            })
        }
        "sendmessage" => {
            arity(4)?;
            Ok(TapeCommand::SendMessage {
                from: args[0].clone(),
                to: args[1].clone(),
                subject: args[2].clone(),
                text: args[3].clone(),
            })
        }
        "setagentstate" => {
            arity(2)?;
            let state = AgentState::parse(&args[1])
                .ok_or_else(|| format!("unknown agent state {:?}", args[1]))?;
            Ok(TapeCommand::SetAgentState {
                window: args[0].clone(),
                state,
            })
        }
        "waitforstate" => parse_wait_for_state(args),
        "sleep" => {
            arity(1)?;
            let secs: u64 = args[0]
                .parse()
                .map_err(|_| "Sleep needs a numeric duration in seconds".to_owned())?;
            if secs > SLEEP_SECS_MAX {
                return Err("Sleep duration is past the cap".to_owned());
            }
            Ok(TapeCommand::Sleep { secs })
        }
        _ => Err(format!("unknown tape command {:?}", tokens[0])),
    }
}

/// The session a tape executes against. Implemented by the test fake and by
/// [`DaemonTape`], which drives a live [`crate::daemon::SessionDaemon`].
pub trait SessionControl {
    type Error: std::fmt::Display;
    fn new_window(&mut self, name: &str) -> Result<(), Self::Error>;
    fn close_window(&mut self, name: &str) -> Result<(), Self::Error>;
    fn focus_window(&mut self, name: &str) -> Result<(), Self::Error>;
    fn send_message(
        &mut self,
        from: &str,
        to: &str,
        subject: &str,
        text: &str,
    ) -> Result<(), Self::Error>;
    fn set_agent_state(&mut self, window: &str, state: AgentState) -> Result<(), Self::Error>;
    fn agent_state(&mut self, window: &str) -> Result<AgentState, Self::Error>;
    fn sleep(&mut self, duration: Duration) -> Result<(), Self::Error>;
    fn binary_present(&self, name: &str) -> bool;
}

/// Execute a parsed tape against `control`.
///
/// `Require` is checked before the first command runs: a missing binary
/// refuses the whole tape and nothing executes. Commands run in order; the
/// first failure stops the tape and reports its line.
///
/// # Errors
///
/// `TapeError` on a missing required binary or a failing command.
pub fn execute<C: SessionControl>(tape: &Tape, control: &mut C) -> Result<(), TapeError> {
    for binary in &tape.header.requires {
        if !control.binary_present(binary) {
            return Err(TapeError::new(
                0,
                "Require",
                format!("required binary {binary:?} is not on PATH; tape skipped"),
            ));
        }
    }
    for (index, command) in tape.commands.iter().enumerate() {
        let line = tape.command_line(index);
        let name = command_name(command);
        run_command(command, control).map_err(|message| TapeError::new(line, name, message))?;
    }
    Ok(())
}

fn command_name(command: &TapeCommand) -> &'static str {
    match command {
        TapeCommand::NewWindow { .. } => "NewWindow",
        TapeCommand::CloseWindow { .. } => "CloseWindow",
        TapeCommand::FocusWindow { .. } => "FocusWindow",
        TapeCommand::SendMessage { .. } => "SendMessage",
        TapeCommand::SetAgentState { .. } => "SetAgentState",
        TapeCommand::WaitForState { .. } => "WaitForState",
        TapeCommand::Sleep { .. } => "Sleep",
    }
}

fn run_command<C: SessionControl>(command: &TapeCommand, control: &mut C) -> Result<(), String> {
    match command {
        TapeCommand::NewWindow { name } => control.new_window(name).map_err(display),
        TapeCommand::CloseWindow { name } => control.close_window(name).map_err(display),
        TapeCommand::FocusWindow { name } => control.focus_window(name).map_err(display),
        TapeCommand::SendMessage {
            from,
            to,
            subject,
            text,
        } => control
            .send_message(from, to, subject, text)
            .map_err(display),
        TapeCommand::SetAgentState { window, state } => {
            control.set_agent_state(window, *state).map_err(display)
        }
        TapeCommand::WaitForState {
            window,
            state,
            timeout_secs,
        } => wait_for_state(control, window, *state, Duration::from_secs(*timeout_secs)),
        TapeCommand::Sleep { secs } => control.sleep(Duration::from_secs(*secs)).map_err(display),
    }
}

fn display(error: impl std::fmt::Display) -> String {
    error.to_string()
}

/// Poll `agent_state` until it equals `want` or `timeout` elapses. Polls at
/// 50 ms; a tape that waits is a tape that yields.
fn wait_for_state<C: SessionControl>(
    control: &mut C,
    window: &str,
    want: AgentState,
    timeout: Duration,
) -> Result<(), String> {
    const POLL_INTERVAL: Duration = Duration::from_millis(50);
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let state = control.agent_state(window).map_err(display)?;
        if state == want {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting for {window:?} to reach {}",
                want.name()
            ));
        }
        control.sleep(POLL_INTERVAL).map_err(display)?;
    }
}

/// PATH lookup for `Require`, without a shell: each `PATH` entry is joined
/// with the binary name and checked for executability.
#[must_use]
pub fn binary_on_path(name: &str) -> bool {
    if name.is_empty() || name.contains('/') || name.contains('\\') {
        return false;
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    for entry in std::env::split_paths(&path) {
        let candidate = entry.join(name);
        if let Ok(metadata) = std::fs::metadata(&candidate)
            && metadata.is_file()
        {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if metadata.permissions().mode() & 0o111 != 0 {
                    return true;
                }
            }
            #[cfg(not(unix))]
            {
                return true;
            }
        }
    }
    false
}

/// A [`SessionControl`] that drives a live [`crate::daemon::SessionDaemon`].
/// The tape's header session is created on first use.
pub struct DaemonTape<'a> {
    daemon: &'a mut crate::daemon::SessionDaemon,
    session: String,
    created: bool,
}

impl<'a> DaemonTape<'a> {
    #[must_use]
    pub fn new(daemon: &'a mut crate::daemon::SessionDaemon, session: &str) -> Self {
        Self {
            daemon,
            session: session.to_owned(),
            created: false,
        }
    }

    fn ensure_session(&mut self) -> Result<(), String> {
        if self.created {
            return Ok(());
        }
        // A session that already exists is fine: tapes compose with state.
        let _ = self.daemon.dispatch(
            "new-session",
            &serde_json::json!({"name": self.session.clone()}),
        );
        self.created = true;
        Ok(())
    }

    fn call(&mut self, verb: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
        self.ensure_session()?;
        let mut params = params;
        if let Some(object) = params.as_object_mut() {
            object
                .entry("session")
                .or_insert(serde_json::json!(self.session.clone()));
        }
        self.daemon
            .dispatch(verb, &params)
            .map_err(|err| format!("{verb}: {}: {}", err.code.as_str(), err.message))
    }
}

impl SessionControl for DaemonTape<'_> {
    type Error = String;

    fn new_window(&mut self, name: &str) -> Result<(), String> {
        self.call("new-window", serde_json::json!({"name": name}))
            .map(|_| ())
    }

    fn close_window(&mut self, name: &str) -> Result<(), String> {
        self.call("close-window", serde_json::json!({"window": name}))
            .map(|_| ())
    }

    fn focus_window(&mut self, name: &str) -> Result<(), String> {
        self.call("focus-window", serde_json::json!({"window": name}))
            .map(|_| ())
    }

    fn send_message(
        &mut self,
        from: &str,
        to: &str,
        subject: &str,
        text: &str,
    ) -> Result<(), String> {
        self.call(
            "send-agent-message",
            serde_json::json!({"from": from, "to": to, "subject": subject, "text": text}),
        )
        .map(|_| ())
    }

    fn set_agent_state(&mut self, window: &str, state: AgentState) -> Result<(), String> {
        self.call(
            "set-agent-state",
            serde_json::json!({"window": window, "state": state.name()}),
        )
        .map(|_| ())
    }

    fn agent_state(&mut self, window: &str) -> Result<AgentState, String> {
        let result = self.call("get-agent-state", serde_json::json!({"window": window}))?;
        let name = result["state"]
            .as_str()
            .ok_or("get-agent-state returned no state")?;
        AgentState::parse(name).ok_or_else(|| format!("daemon returned unknown state {name:?}"))
    }

    fn sleep(&mut self, duration: Duration) -> Result<(), String> {
        std::thread::sleep(duration);
        Ok(())
    }

    fn binary_present(&self, name: &str) -> bool {
        binary_on_path(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fake control recording every call, for executor tests.
    #[derive(Default)]
    struct Fake {
        calls: Vec<String>,
        states: std::collections::HashMap<String, AgentState>,
        binaries: Vec<String>,
        fail_on: Option<String>,
    }

    impl SessionControl for Fake {
        type Error = String;

        fn new_window(&mut self, name: &str) -> Result<(), String> {
            self.calls.push(format!("new_window {name}"));
            self.fail_on
                .as_ref()
                .filter(|f| *f == name)
                .map_or(Ok(()), |_| Err("boom".to_owned()))
        }
        fn close_window(&mut self, name: &str) -> Result<(), String> {
            self.calls.push(format!("close_window {name}"));
            Ok(())
        }
        fn focus_window(&mut self, name: &str) -> Result<(), String> {
            self.calls.push(format!("focus_window {name}"));
            Ok(())
        }
        fn send_message(
            &mut self,
            from: &str,
            to: &str,
            subject: &str,
            text: &str,
        ) -> Result<(), String> {
            self.calls
                .push(format!("send_message {from} {to} {subject} {text}"));
            Ok(())
        }
        fn set_agent_state(&mut self, window: &str, state: AgentState) -> Result<(), String> {
            self.calls
                .push(format!("set_agent_state {window} {}", state.name()));
            self.states.insert(window.to_owned(), state);
            Ok(())
        }
        fn agent_state(&mut self, window: &str) -> Result<AgentState, String> {
            Ok(self.states.get(window).copied().unwrap_or(AgentState::None))
        }
        fn sleep(&mut self, duration: Duration) -> Result<(), String> {
            self.calls.push(format!("sleep {}", duration.as_secs()));
            Ok(())
        }
        fn binary_present(&self, name: &str) -> bool {
            self.binaries.iter().any(|b| b == name)
        }
    }

    const SCRIPT: &str = r#"
# Build the nightly session.
Session "nightly"
Workspace 2
Require "cargo"

NewWindow "builder"
SendMessage "ci" "builder" "build" "cargo build"
SetAgentState "builder" "working"
WaitForState "builder" "working" 30
Sleep 1
FocusWindow "builder"
"#;

    // --- validation ---

    #[test]
    fn header_and_commands_parse() {
        let tape = parse(SCRIPT).expect("parse");
        assert_eq!(tape.header.session.as_deref(), Some("nightly"));
        assert_eq!(tape.header.workspace, Some(2));
        assert_eq!(tape.header.requires, ["cargo"]);
        assert_eq!(tape.commands.len(), 6);
        assert_eq!(
            tape.commands[0],
            TapeCommand::NewWindow {
                name: "builder".to_owned()
            }
        );
        assert!(matches!(
            tape.commands[3],
            TapeCommand::WaitForState {
                timeout_secs: 30,
                ..
            }
        ));
    }

    #[test]
    fn comments_and_blank_lines_are_ignored() {
        let tape = parse("# only a comment\n\n// another\n").expect("parse");
        assert!(tape.commands.is_empty());
        assert!(tape.header.session.is_none());
    }

    #[test]
    fn executor_drives_control_in_order() {
        let tape = parse(SCRIPT).expect("parse");
        let mut fake = Fake {
            binaries: vec!["cargo".to_owned()],
            states: [("builder".to_owned(), AgentState::Done)].into(),
            ..Default::default()
        };
        execute(&tape, &mut fake).expect("execute");
        // WaitForState is already met, so it records no call: the tape's six
        // commands produce five calls, in order.
        assert_eq!(
            fake.calls,
            [
                "new_window builder",
                "send_message ci builder build cargo build",
                "set_agent_state builder working",
                "sleep 1",
                "focus_window builder",
            ]
        );
    }

    #[test]
    fn daemon_tape_runs_against_a_live_daemon() {
        let mut daemon = crate::daemon::SessionDaemon::new();
        let tape = parse(
            "Session \"t\"\nNewWindow \"w\"\nSetAgentState \"w\" \"working\"\nWaitForState \"w\" \"working\" 5\n",
        )
        .expect("parse");
        let mut control = DaemonTape::new(&mut daemon, "t");
        execute(&tape, &mut control).expect("execute");
        let state = control.agent_state("w").expect("state");
        assert_eq!(state, AgentState::Working);
    }

    #[test]
    fn quoted_escapes_parse() {
        let tape =
            parse("SendMessage \"a\" \"b\" \"s\" \"say \\\"hi\\\"\\\\bye\"\n").expect("parse");
        assert_eq!(
            tape.commands[0],
            TapeCommand::SendMessage {
                from: "a".to_owned(),
                to: "b".to_owned(),
                subject: "s".to_owned(),
                text: "say \"hi\"\\bye".to_owned(),
            }
        );
    }

    #[test]
    fn require_is_repeatable() {
        let tape = parse("Require \"sh\"\nRequire \"true\"\nSleep 0\n").expect("parse");
        assert_eq!(tape.header.requires, ["sh", "true"]);
        assert_eq!(tape.commands.len(), 1);
    }

    #[test]
    fn header_directives_are_case_insensitive() {
        let tape = parse("session \"s\"\nworkspace 3\nSleep 0\n").expect("parse");
        assert_eq!(tape.header.session.as_deref(), Some("s"));
        assert_eq!(tape.header.workspace, Some(3));
    }

    #[test]
    fn sleep_zero_executes() {
        let tape = parse("Sleep 0\n").expect("parse");
        let mut fake = Fake::default();
        execute(&tape, &mut fake).expect("execute");
        assert_eq!(fake.calls, ["sleep 0"]);
    }

    #[test]
    fn wait_for_state_already_met_never_sleeps() {
        let tape = parse("WaitForState \"w\" \"done\" 30\n").expect("parse");
        let mut fake = Fake {
            states: [("w".to_owned(), AgentState::Done)].into(),
            ..Default::default()
        };
        execute(&tape, &mut fake).expect("execute");
        assert!(
            fake.calls.is_empty(),
            "no polling needed: {calls:?}",
            calls = fake.calls
        );
    }

    // --- adversarial ---

    #[test]
    fn unclosed_quote_names_the_line() {
        let err = parse("NewWindow \"oops\n").expect_err("unclosed quote");
        assert_eq!(err.line, 1);
    }

    #[test]
    fn unknown_command_is_refused() {
        let err = parse("Frobnicate \"x\"\n").expect_err("unknown command");
        assert_eq!(err.line, 1);
        assert!(err.message.contains("unknown tape command"));
    }

    #[test]
    fn wrong_arity_is_refused() {
        let err = parse("NewWindow\n").expect_err("arity");
        assert!(err.message.contains("needs 1 arguments"));
    }

    #[test]
    fn oversize_tape_is_refused() {
        let big = "x".repeat(TAPE_BYTES_MAX + 1);
        let err = parse(&big).expect_err("oversize");
        assert_eq!(err.line, 0);
    }

    #[test]
    fn overlong_line_is_refused() {
        let line = format!("Sleep {}\n", "1".repeat(TAPE_LINE_BYTES_MAX));
        let err = parse(&line).expect_err("overlong line");
        assert_eq!(err.line, 1);
    }

    #[test]
    fn missing_required_binary_runs_nothing() {
        let tape = parse("Require \"definitely-not-a-real-binary-xyz\"\nNewWindow \"w\"\n")
            .expect("parse");
        let mut fake = Fake::default();
        let err = execute(&tape, &mut fake).expect_err("missing binary");
        assert!(err.message.contains("not on PATH"));
        assert!(
            fake.calls.is_empty(),
            "nothing may run before Require passes"
        );
    }

    #[test]
    fn wait_timeout_reports_the_window_and_state() {
        let tape = parse("WaitForState \"w\" \"done\" 0\n").expect("parse");
        let mut fake = Fake::default();
        let err = execute(&tape, &mut fake).expect_err("timeout");
        assert!(err.message.contains("\"w\""), "{err}");
        assert!(err.message.contains("done"), "{err}");
    }

    #[test]
    fn command_failure_stops_the_tape_at_its_line() {
        let tape = parse("NewWindow \"a\"\nNewWindow \"bad\"\nNewWindow \"c\"\n").expect("parse");
        let mut fake = Fake {
            fail_on: Some("bad".to_owned()),
            ..Default::default()
        };
        let err = execute(&tape, &mut fake).expect_err("failure");
        assert_eq!(err.line, 2);
        assert_eq!(err.command, "NewWindow");
        assert!(
            !fake.calls.iter().any(|c| c == "new_window c"),
            "tape stops"
        );
    }

    #[test]
    fn binary_on_path_rejects_separators() {
        assert!(!binary_on_path(""));
        assert!(!binary_on_path("../bin/sh"));
        assert!(!binary_on_path("a\\b"));
    }

    #[test]
    fn too_many_commands_are_refused() {
        let script = "Sleep 0\n".repeat(TAPE_COMMANDS_MAX + 1);
        let err = parse(&script).expect_err("too many commands");
        assert!(err.message.contains("more commands than the cap"));
    }
}
