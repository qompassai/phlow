# phlow-tuios design decisions

Lane `lane/tuios`, 2026-09-28. Adaptation of tuios concepts (MIT, Go/Bubble
Tea terminal multiplexer) into Tiger Style Rust. Concepts are re-expressed,
never ported: no Go code was translated.

## 1. Crate placement: `crates/phlow-tuios`

A new language-neutral crate, not an addition to an existing one:

- `phlow-tui` is the terminal UI. Agent state, the mailbox, and the session
  daemon are broader than one frontend; the TUI will be a *client* of this
  crate, not its owner.
- `phlow-mcp` is the Model Context Protocol surface. The Unix-socket
  JSON-line control protocol is not MCP and must not live there.
- `phlow-runtime` is the authoritative safe execution runtime. Making it
  the session/protocol dumping ground would dilute that authority.
- A dedicated crate keeps the socket protocol language-agnostic: `phlow-tui`,
  the CLI, or future non-Rust clients can all depend on it.

## 2. Hook system: daemon/client split kept

tuios splits hooks into daemon-side and client-side (`internal/hooks/hooks.go:23-39`;
daemon-side firing in `internal/session/daemon_hooks.go:191,268`). The split
is kept verbatim, including the event names:

- daemon-side: `after-new-window`, `after-close-window`, `after-focus-change`,
  `after-workspace-switch`, `after-agent-state`, `after-command-finished`
- client-side: `after-attach`, `after-detach`, `after-resize`,
  `after-layout-change` — the daemon lists them (`set-hook` refuses them with
  `invalid_params`) so a client knows what the embedding UI runs locally.

Deliberate difference: tuios runs hook commands through the shell;
phlow-tuios registers argv arrays and spawns them with no implicit shell
(`hooks.rs`). An operator who wants shell semantics registers
`["sh", "-c", "..."]` explicitly. `fire` never blocks: commands run on
spawned threads behind a real counting semaphore capped at
`HOOK_CONCURRENT_MAX` (8); further firings queue on the semaphore.

`after-agent-state` is the alert sink: it fires on every `set-agent-state`
transition with `agent_state`, `prev_agent_state`, `agent_harness`, and
`agent_message` in the hook environment.

## 3. Session daemon shape

tuios multiplexes terminal panes; the daemon here multiplexes *agent* panes:

- `SessionDaemon` owns sessions (`SESSIONS_MAX` = 256).
- `PhlowSession` owns windows (64 max), the focused window, the per-session
  `Mailbox` ring, and the `AskGraph`.
- `AgentPane` is a name, a stable numeric id, and the daemon-owned
  `AgentPaneState` (the agent's own report: state + block note + harness).

The mailbox lives on the session, not the pane, because tuios's ring is
per session (`internal/session/verb_mailbox.go:66-69`) and a pane's inbox is
a view over it (`inbox` verb: directed mail to the window id plus session
notices; ask records are history, never inbox mail).

Verbs (18): `ping`, `list-verbs`, `new-session`, `close-session`,
`list-sessions`, `new-window`, `close-window`, `focus-window`,
`list-windows`, `set-agent-state`, `get-agent-state`, `send-mail`,
`mail-list`, `send-agent-message`, `inbox`, `ask-agent`, `set-hook`,
`list-hooks`. `ping` is an adaptation, not an upstream verb: the real
daemon probed 2026-09-28 returns `unknown_verb` for it. Unknown parameter
names are refused with `invalid_params`
(never silently ignored), matching the probed daemon.

### `ask-agent` semantics (deliberate difference)

tuios's `ask-agent` delivers the question to the pane's live agent and
blocks until it answers. This daemon has no agent processes — panes are
state records — so `ask-agent` is *delivery, not a blocking wait*: the
question lands in the target's inbox as a directed message, an ask record
(`MessageKind::Ask`, `settled_by: "delivered"`) joins the session history,
and the call returns `{"ask_id", "delivered": true, "state"}`. The ask-graph
edge opens for the delivery and closes with it, so the cycle check guards
the only window where a cycle can form; self-ask is `loop_refused`.
`timeout_secs` is accepted for wire compatibility and validated, but nothing
waits. A live-agent binding can later hold the edge open across the wait.

### Human origin over the socket (deliberate restriction)

tuios distinguishes verified vs claimed human senders
(`internal/session/verb_mailbox.go:188-194`). This socket has no caller
identity, so it never vouches: `from: "human"` (or an ask end of `"human"`)
is refused with `forbidden`. The person's client must use a path that does
vouch — that path is undecided (see open questions).

## 4. Tape shape

tuios tapes script terminal demos (`internal/tape/`: `header.go`,
`lexer.go`, `parser.go`, `executor.go`). The shape is kept — a leading
header block (`Session`, `Workspace`, `Require`; `internal/tape/header.go:21-30`),
`#`/`//` comments, one command per line — but the commands script *agent
sessions*, not keystrokes: `NewWindow`, `CloseWindow`, `FocusWindow`,
`SendMessage`, `SetAgentState`, `WaitForState`, `Sleep`.

Parsing never executes: a tape is fully parsed (and `Require` binaries are
PATH-checked with no shell, rejecting path separators) before the first
command runs; a missing binary refuses the whole tape and nothing executes.
`DaemonTape` drives a live `SessionDaemon` through the same `dispatch`
every socket client uses, so tapes and clients cannot diverge.

## 5. Empirical probe of real tuios (2026-09-28, primo)

Built the real daemon from `/tmp/tuios` (Go 1.27.1, 36,618,992-byte
binary) and ran `go test ./internal/session/ -run
'TestVerb|TestProtocol|TestMailbox|TestAgentBus|TestAgentState' -count=1`:
pass, 0.235s. Hand-written Unix-socket probes observed:

- id echo: numeric `1`, string `"abc-123"`, absent, and explicit `null`
  all round-trip verbatim; malformed JSON gets `invalid_request` with no id.
- unknown verb `frobnicate` → `unknown_verb`, message `unknown verb
  frobnicate`, hint with `verb`, `command`, `available`, `detail`;
  near-miss `list-verbz` additionally gets `did_you_mean: "list-verbs"`.
- missing `verb` → `invalid_request` + hint; string-valued params and
  unknown param names → `invalid_params`.
- a line starting with non-`{` bytes or a JSON array hits tuios's first-byte
  sniff and is routed to the binary path: connection closes, no JSON
  response. (phlow-tuios is JSON-only; there is no binary peer to sniff for.)
- a request past 16 MiB resets/drops the connection with no response.
- socket dir `drwx------`, socket `srwx------`.
- 97 verbs listed; `new-session` ok, duplicate → `session_exists`.
- message text past 8 KiB → `invalid_params` ("text is longer than the
  message cap"); bad agent state `frobnicating` → `invalid_params`;
  `set-agent-state` to `working` ok.
- rate limiting was not reached empirically (sends failed first with
  `window_not_found` on invented pane names); the burst-10 / 30-per-minute
  policy is taken from source (`verb_mailbox.go:73-77`).
- `after-new-window` hook configured to append to a log file fired twice
  for `new-session` + `new-window`, proving hook firing end to end.
  Generic firing is proven; an explicit `after-agent-state` firing against
  the real daemon was not exercised (the Rust side fires and tests it).
- second live probe (same day, `/tmp/tuios-run/tuios/tuios.sock`):
  `ping` → `unknown_verb`, so `ping` is confirmed absent upstream and is
  an adaptation in phlow-tuios; zero-parameter `hello` accepts absent or
  explicit-null params but rejects an unknown named parameter with
  `invalid_params`; `list-verbs` likewise rejects unknown named parameters
  and array-shaped params with `invalid_params`.

## 6. Upstream citations (exact)

All paths under `/tmp/tuios`:

- `internal/session/agent_state.go:25-45` — the seven states and wire
  spellings (`""`, `working`, `needs_input`, `idle`, `done`, `errored`,
  `unknown`); `:97-99` — `NeedsYou` true for `needs_input`/`errored`.
- `internal/session/verb_mailbox.go:59-79` — caps: 8 KiB text (`:62`),
  120-char subject (`:63`), 8 attachments (`:64`), 256-message ring
  (`:66`), 512 KiB text (`:69`), burst 10 (`:74`), 30 sends/minute
  (`:77`); `:49-53` — reserved `human` inbox id; `:188-194` —
  verified vs claimed human.
- `internal/session/verb_protocol.go:47` — `unknown_verb` code;
  `:1897-1900` — 16 MiB scanner cap on a request line;
  `:1961-1964` — unknown-verb hint with `DidYouMean`.
- `internal/hooks/hooks.go:23-39` — the ten event names and the
  daemon/client split; `:45-47` — the full event list.
- `internal/session/daemon_hooks.go:57-68` — session-side hook events;
  `:268` — `fireAgentStateHook`.
- `internal/session/manager_unix.go:64,78` — runtime dir at 0700;
  `internal/session/daemon.go:919,1286` — socket chmod 0700.
- `internal/tape/header.go:21-30` — header directives (`Session`,
  `Scope`, `Workspace`, `Require`).

## 7. Open design questions

1. **Human vouching.** The socket refuses `from: "human"`. What vouches
   for the person — a separate privileged socket, a client credential, an
   explicit flag on a local-only path? Undecided; the restriction is the
   safe default until then.
2. **`ask-agent` waiting.** Fire-and-record is honest without agent
   processes, but tuios clients may expect the blocking wait. If a wait is
   added later, `timeout_secs` is already in the wire surface.
3. **`after-command-finished`.** The daemon runs no commands, so nothing
   fires it yet. Either a future command-running verb fires it, or the
   event stays client-defined. Not wired to anything today.
4. **phlow-tui integration.** The crate is standalone and green first; the
   TUI then gets a client API or re-export. No invasive refactor before
   that.
5. **Probe gap.** `after-agent-state` hook firing was proven generically
   (after-new-window) on the real daemon but not for that specific event.
   The Rust side fires it and asserts the log line; worth one real-daemon
   run if the event's exact environment contract ever matters.
