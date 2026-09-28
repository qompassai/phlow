//! Hook events, adapted from tuios's `internal/hooks/hooks.go`.
//!
//! Hooks fire asynchronously when specific events occur and run
//! operator-registered commands with the event context in the environment.
//! The event set and the daemon-side / client-side split follow tuios:
//!
//! - daemon-side: after-new-window, after-close-window, after-focus-change,
//!   after-workspace-switch, after-agent-state, after-command-finished. The
//!   daemon fires these; `after-agent-state` is the alert sink and fires only
//!   for states the policy names.
//! - client-side: after-attach, after-detach, after-resize, after-layout-change.
//!   The daemon never fires these — it lists them so a client knows what the
//!   embedding UI (e.g. phlow-tui) runs locally.
//!
//! Difference from tuios, documented once here: tuios runs hook commands
//! through the shell. phlow-tuios registers argv arrays and spawns them
//! without an implicit shell; an operator who wants shell semantics registers
//! `["sh", "-c", "..."]` explicitly.

use std::collections::HashMap;
use std::process::Command;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

/// A hook event. Wire spellings match tuios's event names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HookEvent {
    AfterNewWindow,
    AfterCloseWindow,
    AfterFocusChange,
    AfterWorkspaceSwitch,
    AfterAgentState,
    AfterCommandFinished,
    AfterAttach,
    AfterDetach,
    AfterResize,
    AfterLayoutChange,
}

/// Which side of the architecture fires the event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookSide {
    Daemon,
    Client,
}

impl HookEvent {
    /// The wire spelling, stable across versions.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::AfterNewWindow => "after-new-window",
            Self::AfterCloseWindow => "after-close-window",
            Self::AfterFocusChange => "after-focus-change",
            Self::AfterWorkspaceSwitch => "after-workspace-switch",
            Self::AfterAgentState => "after-agent-state",
            Self::AfterCommandFinished => "after-command-finished",
            Self::AfterAttach => "after-attach",
            Self::AfterDetach => "after-detach",
            Self::AfterResize => "after-resize",
            Self::AfterLayoutChange => "after-layout-change",
        }
    }

    /// Parse a wire spelling.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "after-new-window" => Some(Self::AfterNewWindow),
            "after-close-window" => Some(Self::AfterCloseWindow),
            "after-focus-change" => Some(Self::AfterFocusChange),
            "after-workspace-switch" => Some(Self::AfterWorkspaceSwitch),
            "after-agent-state" => Some(Self::AfterAgentState),
            "after-command-finished" => Some(Self::AfterCommandFinished),
            "after-attach" => Some(Self::AfterAttach),
            "after-detach" => Some(Self::AfterDetach),
            "after-resize" => Some(Self::AfterResize),
            "after-layout-change" => Some(Self::AfterLayoutChange),
            _ => Option::None,
        }
    }

    /// Which side fires the event. Mirrors tuios's `sessionSideHooks`: the
    /// daemon drops client-side registrations rather than holding commands it
    /// can never run.
    #[must_use]
    pub fn side(self) -> HookSide {
        match self {
            Self::AfterNewWindow
            | Self::AfterCloseWindow
            | Self::AfterFocusChange
            | Self::AfterWorkspaceSwitch
            | Self::AfterAgentState
            | Self::AfterCommandFinished => HookSide::Daemon,
            Self::AfterAttach | Self::AfterDetach | Self::AfterResize | Self::AfterLayoutChange => {
                HookSide::Client
            }
        }
    }

    /// Every event in wire order.
    #[must_use]
    pub fn all() -> &'static [HookEvent] {
        &[
            Self::AfterNewWindow,
            Self::AfterCloseWindow,
            Self::AfterFocusChange,
            Self::AfterWorkspaceSwitch,
            Self::AfterAgentState,
            Self::AfterCommandFinished,
            Self::AfterAttach,
            Self::AfterDetach,
            Self::AfterResize,
            Self::AfterLayoutChange,
        ]
    }
}

/// Context for one hook firing. Fields that do not describe the event stay at
/// their zero value, so a hook command can read the environment
/// unconditionally. Only non-sensitive operational context is exposed here:
/// no pane tokens, no credentials.
#[derive(Debug, Clone, Default)]
pub struct HookContext {
    /// Window the event describes, when there is one.
    pub window_id: String,
    pub window_name: String,
    pub workspace: u32,
    pub session_id: String,
    pub event: Option<HookEvent>,
    /// Workspace active before an after-workspace-switch. Zero otherwise.
    pub previous_workspace: u32,
    /// Tiling layout after an after-layout-change. Empty otherwise.
    pub layout: String,
    /// New size in cells after an after-resize. Zero otherwise.
    pub width: u32,
    pub height: u32,
    /// State the pane moved into on an after-agent-state, and the one it came
    /// from, in wire spelling. Empty for every other event.
    pub agent_state: String,
    pub prev_agent_state: String,
    /// Harness id the reporting source named, and its free text.
    pub agent_harness: String,
    pub agent_message: String,
    /// For after-command-finished: the command line, its exit status, and how
    /// long it ran in milliseconds.
    pub command: String,
    pub exit_code: String,
    pub duration_ms: String,
}

/// Bound: registered commands per event.
pub const HOOKS_PER_EVENT_MAX: usize = 32;
/// Bound: hook commands running concurrently; further firings queue behind it.
pub const HOOK_CONCURRENT_MAX: usize = 8;
/// Bound: hook commands queued-or-running in total. Past this `fire` sheds
/// load instead of spawning another thread: the command is recorded as
/// dropped, never run silently.
pub const HOOK_QUEUED_MAX: usize = 64;
/// Bound: wall-clock time one hook command may run before it is killed.
pub const HOOK_TIMEOUT: Duration = Duration::from_secs(30);
/// Bound: length of one environment value, in bytes.
pub const HOOK_ENV_VALUE_BYTES_MAX: usize = 8 * 1024;

/// Outcome of one hook command run, kept for `hook-status` style inspection.
#[derive(Debug, Clone)]
pub struct HookOutcome {
    pub event: HookEvent,
    pub argv: Vec<String>,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    /// True when the command never ran: the queue was full and `fire` shed
    /// the load. Recorded, never silent.
    pub dropped: bool,
}

/// Runs registered hook commands. `fire` never blocks: commands run on
/// spawned threads behind a concurrency semaphore, and a failing or slow
/// command never stops the others.
#[derive(Debug, Clone)]
pub struct HookManager {
    inner: Arc<HookManagerInner>,
}

#[derive(Debug)]
struct HookManagerInner {
    hooks: Mutex<HashMap<HookEvent, Vec<Vec<String>>>>,
    outcomes: Mutex<Vec<HookOutcome>>,
    /// Bound on outcomes kept; the newest win.
    outcomes_cap: usize,
    /// Bounds concurrent hook command runs; further firings queue behind it.
    semaphore: Semaphore,
    /// Bounds commands queued-or-running in total; further firings are shed.
    queue: Arc<Semaphore>,
}

/// A counting semaphore: at most `permits` holders run at once. A fired hook
/// takes a permit for the whole run, so `fire` never blocks while hook
/// commands stay within [`HOOK_CONCURRENT_MAX`] concurrent runs.
///
/// Shared with the socket server, which bounds concurrent connections the
/// same way: one primitive, one bound idiom, no duplicates.
#[derive(Debug)]
pub(crate) struct Semaphore {
    permits: Mutex<usize>,
    cvar: Condvar,
}

impl Semaphore {
    pub(crate) fn new(permits: usize) -> Self {
        Self {
            permits: Mutex::new(permits),
            cvar: Condvar::new(),
        }
    }

    fn acquire(&self) -> SemaphoreGuard<'_> {
        let mut permits = self.permits.lock().expect("semaphore lock");
        while *permits == 0 {
            permits = self.cvar.wait(permits).expect("semaphore wait");
        }
        *permits -= 1;
        SemaphoreGuard { semaphore: self }
    }

    /// Take a permit that can outlive the caller's stack: the guard owns an
    /// `Arc` to the semaphore instead of borrowing it, so it moves into a
    /// spawned thread. The accept loop uses this to bound connections.
    pub(crate) fn acquire_owned(self: &Arc<Self>) -> OwnedSemaphoreGuard {
        let mut permits = self.permits.lock().expect("semaphore lock");
        while *permits == 0 {
            permits = self.cvar.wait(permits).expect("semaphore wait");
        }
        *permits -= 1;
        OwnedSemaphoreGuard {
            semaphore: Arc::clone(self),
        }
    }

    /// Try an owned permit without blocking: `None` when the pool is empty.
    /// Load-shedding uses this so `fire` never blocks on a full queue.
    pub(crate) fn try_acquire_owned(self: &Arc<Self>) -> Option<OwnedSemaphoreGuard> {
        let mut permits = self.permits.lock().expect("semaphore lock");
        if *permits == 0 {
            return Option::None;
        }
        *permits -= 1;
        Some(OwnedSemaphoreGuard {
            semaphore: Arc::clone(self),
        })
    }
}

/// Returns its permit on drop, even when the hook command panics.
#[derive(Debug)]
struct SemaphoreGuard<'a> {
    semaphore: &'a Semaphore,
}

impl Drop for SemaphoreGuard<'_> {
    fn drop(&mut self) {
        let mut permits = self.semaphore.permits.lock().expect("semaphore lock");
        *permits += 1;
        self.semaphore.cvar.notify_one();
    }
}

/// An owned semaphore permit: returns itself on drop like [`SemaphoreGuard`],
/// but carries the semaphore by `Arc` so the permit moves into threads.
#[derive(Debug)]
pub(crate) struct OwnedSemaphoreGuard {
    semaphore: Arc<Semaphore>,
}

impl Drop for OwnedSemaphoreGuard {
    fn drop(&mut self) {
        let mut permits = self.semaphore.permits.lock().expect("semaphore lock");
        *permits += 1;
        self.semaphore.cvar.notify_one();
    }
}

impl HookManager {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(HookManagerInner {
                hooks: Mutex::new(HashMap::new()),
                outcomes: Mutex::new(Vec::new()),
                outcomes_cap: 128,
                semaphore: Semaphore::new(HOOK_CONCURRENT_MAX),
                queue: Arc::new(Semaphore::new(HOOK_QUEUED_MAX)),
            }),
        }
    }

    /// Register an argv command for an event. Client-side events are refused:
    /// the daemon would hold commands it can never run, which reads as broken.
    ///
    /// # Errors
    ///
    /// `invalid_params` on a client-side event, an empty argv, or more than
    /// [`HOOKS_PER_EVENT_MAX`] commands on the event.
    pub fn register(
        &self,
        event: HookEvent,
        argv: Vec<String>,
    ) -> Result<(), crate::error::TuiosError> {
        use crate::error::{ErrorCode, TuiosError};
        if event.side() != HookSide::Daemon {
            return Err(TuiosError::protocol(
                ErrorCode::InvalidParams,
                format!("{} is client-side; the daemon cannot run it", event.name()),
            ));
        }
        if argv.is_empty() {
            return Err(TuiosError::protocol(
                ErrorCode::InvalidParams,
                "hook command needs an argv",
            ));
        }
        let mut hooks = self.inner.hooks.lock().expect("hook table lock");
        let list = hooks.entry(event).or_default();
        if list.len() >= HOOKS_PER_EVENT_MAX {
            return Err(TuiosError::protocol(
                ErrorCode::InvalidParams,
                "too many hook commands on one event",
            ));
        }
        list.push(argv);
        Ok(())
    }

    /// Fire an event: run every registered command asynchronously with the
    /// context in `PHLOW_HOOK_*` environment variables. Client-side events
    /// are ignored, never run. `fire` never blocks: at most
    /// [`HOOK_QUEUED_MAX`] commands queue-or-run at once, and a firing past
    /// that is recorded as dropped rather than spawning another thread.
    pub fn fire(&self, event: HookEvent, ctx: &HookContext) {
        if event.side() != HookSide::Daemon {
            return;
        }
        let inner = Arc::clone(&self.inner);
        let commands: Vec<Vec<String>> = {
            let hooks = inner.hooks.lock().expect("hook table lock");
            hooks.get(&event).cloned().unwrap_or_default()
        };
        for argv in commands {
            // Load-shedding: bound the threads waiting on the run
            // semaphore. The slot is held for the whole run, so queued +
            // running never exceeds HOOK_QUEUED_MAX.
            let Some(slot) = inner.queue.try_acquire_owned() else {
                self.record_dropped(event, &argv);
                continue;
            };
            let manager = self.clone();
            let ctx = ctx.clone();
            thread::spawn(move || {
                let _slot = slot;
                // The permit bounds concurrent runs; the thread queues here
                // when the bound is reached, and `fire` itself never blocks.
                let _permit = manager.inner.semaphore.acquire();
                manager.run_one(event, &argv, &ctx);
            });
        }
    }

    /// The outcomes of finished hook runs, newest last, capped.
    #[must_use]
    pub fn outcomes(&self) -> Vec<HookOutcome> {
        self.inner.outcomes.lock().expect("outcome lock").clone()
    }

    /// The registered commands per event, for introspection (`list-hooks`).
    #[must_use]
    pub fn registered(&self) -> HashMap<HookEvent, Vec<Vec<String>>> {
        self.inner.hooks.lock().expect("hook table lock").clone()
    }

    fn run_one(&self, event: HookEvent, argv: &[String], ctx: &HookContext) {
        let outcome = run_argv(argv, ctx);
        let mut outcomes = self.inner.outcomes.lock().expect("outcome lock");
        outcomes.push(HookOutcome {
            event,
            argv: argv.to_vec(),
            exit_code: outcome.0,
            timed_out: outcome.1,
            dropped: false,
        });
        while outcomes.len() > self.inner.outcomes_cap {
            outcomes.remove(0);
        }
    }

    /// Record a firing that never ran: the queue was full and the load was
    /// shed. Visible in outcomes, never silent.
    fn record_dropped(&self, event: HookEvent, argv: &[String]) {
        let mut outcomes = self.inner.outcomes.lock().expect("outcome lock");
        outcomes.push(HookOutcome {
            event,
            argv: argv.to_vec(),
            exit_code: Option::None,
            timed_out: false,
            dropped: true,
        });
        while outcomes.len() > self.inner.outcomes_cap {
            outcomes.remove(0);
        }
    }
}

impl Default for HookManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Run one hook command with the context as environment. Returns
/// (exit_code, timed_out). No shell is involved: `argv[0]` is the program.
fn run_argv(argv: &[String], ctx: &HookContext) -> (Option<i32>, bool) {
    let mut command = Command::new(&argv[0]);
    if argv.len() > 1 {
        command.args(&argv[1..]);
    }
    for (key, value) in hook_env(ctx) {
        command.env(key, value);
    }
    // The child is reaped on every path: wait with a timeout, then kill.
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(_) => return (Option::None, false),
    };
    let deadline = std::time::Instant::now() + HOOK_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return (status.code(), false),
            Ok(Option::None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return (Option::None, true);
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(_) => return (Option::None, false),
        }
    }
}

/// The environment for a hook command. Every value is truncated to
/// [`HOOK_ENV_VALUE_BYTES_MAX`] bytes; names are fixed.
fn hook_env(ctx: &HookContext) -> Vec<(String, String)> {
    let mut env = Vec::new();
    let mut put = |key: &str, value: &str| {
        let v = crate::truncate_bytes(value, HOOK_ENV_VALUE_BYTES_MAX);
        env.push((format!("PHLOW_HOOK_{key}"), v));
    };
    put("EVENT", ctx.event.map_or("", HookEvent::name));
    put("WINDOW_ID", &ctx.window_id);
    put("WINDOW_NAME", &ctx.window_name);
    put("WORKSPACE", &ctx.workspace.to_string());
    put("SESSION_ID", &ctx.session_id);
    put("PREVIOUS_WORKSPACE", &ctx.previous_workspace.to_string());
    put("LAYOUT", &ctx.layout);
    put("WIDTH", &ctx.width.to_string());
    put("HEIGHT", &ctx.height.to_string());
    put("AGENT_STATE", &ctx.agent_state);
    put("PREV_AGENT_STATE", &ctx.prev_agent_state);
    put("AGENT_HARNESS", &ctx.agent_harness);
    put("AGENT_MESSAGE", &ctx.agent_message);
    put("COMMAND", &ctx.command);
    put("EXIT_CODE", &ctx.exit_code);
    put("DURATION_MS", &ctx.duration_ms);
    env
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration as StdDuration;

    fn ctx_for(event: HookEvent) -> HookContext {
        HookContext {
            event: Some(event),
            window_id: "w1".to_owned(),
            session_id: "s".to_owned(),
            ..Default::default()
        }
    }

    fn wait_for_outcomes(manager: &HookManager, want: usize) {
        let deadline = std::time::Instant::now() + StdDuration::from_secs(10);
        while manager.outcomes().len() < want {
            assert!(
                std::time::Instant::now() < deadline,
                "hook did not run in time"
            );
            thread::sleep(StdDuration::from_millis(20));
        }
    }

    // --- validation ---

    #[test]
    fn daemon_side_event_fires_with_context_env() {
        let manager = HookManager::new();
        manager
            .register(
                HookEvent::AfterNewWindow,
                vec![
                    "sh".to_owned(),
                    "-c".to_owned(),
                    "echo $PHLOW_HOOK_WINDOW_ID > /tmp/phlow-tuios-hook-test-env".to_owned(),
                ],
            )
            .expect("register");
        manager.fire(
            HookEvent::AfterNewWindow,
            &ctx_for(HookEvent::AfterNewWindow),
        );
        wait_for_outcomes(&manager, 1);
        let seen = std::fs::read_to_string("/tmp/phlow-tuios-hook-test-env")
            .expect("hook wrote its env file");
        assert_eq!(seen.trim(), "w1");
        let _ = std::fs::remove_file("/tmp/phlow-tuios-hook-test-env");
    }

    #[test]
    fn multiple_commands_on_one_event_all_run() {
        let manager = HookManager::new();
        for i in 0..3 {
            manager
                .register(
                    HookEvent::AfterCloseWindow,
                    vec!["sh".to_owned(), "-c".to_owned(), format!("exit {i}")],
                )
                .expect("register");
        }
        manager.fire(
            HookEvent::AfterCloseWindow,
            &ctx_for(HookEvent::AfterCloseWindow),
        );
        wait_for_outcomes(&manager, 3);
        let mut codes: Vec<Option<i32>> = manager.outcomes().iter().map(|o| o.exit_code).collect();
        codes.sort();
        assert_eq!(codes, [Some(0), Some(1), Some(2)]);
    }

    #[test]
    fn client_side_event_is_never_fired_by_the_daemon() {
        let manager = HookManager::new();
        // Registration itself is refused...
        let err = manager
            .register(HookEvent::AfterAttach, vec!["true".to_owned()])
            .expect_err("client-side register must fail");
        assert_eq!(err.code(), crate::error::ErrorCode::InvalidParams);
        // ...and fire is a no-op even if a command were present.
        manager.fire(HookEvent::AfterAttach, &ctx_for(HookEvent::AfterAttach));
        thread::sleep(StdDuration::from_millis(100));
        assert!(manager.outcomes().is_empty());
    }

    // --- adversarial ---

    #[test]
    fn event_registration_is_capped() {
        let manager = HookManager::new();
        for _ in 0..HOOKS_PER_EVENT_MAX {
            manager
                .register(HookEvent::AfterNewWindow, vec!["true".to_owned()])
                .expect("register within cap");
        }
        let err = manager
            .register(HookEvent::AfterNewWindow, vec!["true".to_owned()])
            .expect_err("registration past the cap must fail");
        assert_eq!(err.code(), crate::error::ErrorCode::InvalidParams);
    }

    #[test]
    fn failing_hook_does_not_stop_the_others() {
        let manager = HookManager::new();
        manager
            .register(HookEvent::AfterFocusChange, vec!["false".to_owned()])
            .expect("register");
        manager
            .register(HookEvent::AfterFocusChange, vec!["true".to_owned()])
            .expect("register");
        manager.fire(
            HookEvent::AfterFocusChange,
            &ctx_for(HookEvent::AfterFocusChange),
        );
        wait_for_outcomes(&manager, 2);
        assert_eq!(manager.outcomes().len(), 2);
    }

    #[test]
    fn missing_binary_reports_no_exit_code() {
        let manager = HookManager::new();
        manager
            .register(
                HookEvent::AfterFocusChange,
                vec!["/nonexistent/phlow-hook-binary".to_owned()],
            )
            .expect("register");
        manager.fire(
            HookEvent::AfterFocusChange,
            &ctx_for(HookEvent::AfterFocusChange),
        );
        wait_for_outcomes(&manager, 1);
        let outcomes = manager.outcomes();
        assert_eq!(outcomes[0].exit_code, Option::None);
        assert!(!outcomes[0].timed_out);
    }

    #[test]
    fn fire_sheds_load_when_queue_is_full() {
        let manager = HookManager::new();
        manager
            .register(HookEvent::AfterNewWindow, vec!["true".to_owned()])
            .expect("register");
        // Occupy every queue slot without running anything, so the firing
        // has nowhere to wait.
        let mut held = Vec::new();
        for _ in 0..HOOK_QUEUED_MAX {
            held.push(manager.inner.queue.try_acquire_owned().expect("slot"));
        }
        manager.fire(
            HookEvent::AfterNewWindow,
            &ctx_for(HookEvent::AfterNewWindow),
        );
        wait_for_outcomes(&manager, 1);
        let outcomes = manager.outcomes();
        assert_eq!(
            outcomes.len(),
            1,
            "the shed firing is recorded exactly once"
        );
        assert!(
            outcomes[0].dropped,
            "a full queue records a dropped outcome"
        );
        assert_eq!(outcomes[0].exit_code, Option::None);
        drop(held);
    }

    // --- validation ---

    #[test]
    fn registration_cap_is_per_event() {
        let manager = HookManager::new();
        for _ in 0..HOOKS_PER_EVENT_MAX {
            manager
                .register(HookEvent::AfterNewWindow, vec!["true".to_owned()])
                .expect("register within cap");
        }
        // A full cap on one event does not affect a different event.
        manager
            .register(HookEvent::AfterCloseWindow, vec!["true".to_owned()])
            .expect("other event still accepts registrations");
        assert_eq!(
            manager
                .registered()
                .get(&HookEvent::AfterNewWindow)
                .map(Vec::len),
            Some(HOOKS_PER_EVENT_MAX)
        );
    }

    #[test]
    fn fire_runs_and_releases_queue_slots() {
        let manager = HookManager::new();
        manager
            .register(HookEvent::AfterNewWindow, vec!["true".to_owned()])
            .expect("register");
        manager.fire(
            HookEvent::AfterNewWindow,
            &ctx_for(HookEvent::AfterNewWindow),
        );
        wait_for_outcomes(&manager, 1);
        let outcomes = manager.outcomes();
        assert!(!outcomes[0].dropped, "a normal firing is not shed");
        assert_eq!(outcomes[0].exit_code, Some(0));
        // Every permit came back: the queue is whole again.
        let mut held = Vec::new();
        for _ in 0..HOOK_QUEUED_MAX {
            held.push(
                manager
                    .inner
                    .queue
                    .try_acquire_owned()
                    .expect("permit returned"),
            );
        }
        drop(held);
    }
}
