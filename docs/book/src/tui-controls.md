# User Guide: TUI Controls

First, the honest headline: **there are no keybindings to list.** A
grep for `KeyCode` over every crate in the workspace finds zero
matches — the TUI has no key-event handler at all. phlow's TUI is not a
full-screen key-driven interface; it is a *line loop* over stdin/stdout
(the ratatui panels render output, but crossterm only drives the
backend — input stays line-based). You type a line, press Enter, and the
runtime answers. The "controls" are therefore: two control keys, the
prompt, and the slash commands.

This is verified against `crates/phlow-tui/src` (`app.rs`,
`commands.rs`, `panels.rs`). If a control is not in that source, it
is not documented here.

## The two keys that matter

| Key | What happens |
|---|---|
| **Ctrl-D** | End of input (EOF). The loop exits cleanly, exit code 0 — the banner says it plainly: `Type /help for commands. /quit or Ctrl-D to exit.` |
| **Ctrl-C** | Terminates the process. This is a *deliberate deviation* from the Python `flow`, which caught Ctrl-C and continued the loop. In the Rust TUI there is no signal handler; Ctrl-C ends the session. |

Everything else you type is just text: either a task (sent to the
runtime as a natural-language request) or a slash command. One hard
limit: an input line longer than **1,000,000 bytes**
(`INPUT_LINE_BYTES_MAX` in `app.rs`) is rejected instead of being
buffered — stdin can never make the TUI grow a buffer without bound.

## The prompt

The interactive prompt is literally `flow > ` (yes, still the old
name in the source):

```text
flow > Type /help for commands. /quit or Ctrl-D to exit.
```

If stdout is piped instead of a TTY, the same loop runs **headless**:
plain prompts, results as compact JSON, so `phlow < /dev/null` and CI
pipelines work.

## Slash commands

From the `COMMANDS` table in `commands.rs`. Any other line is passed
to the runtime unchanged as a task.

| Command | Does |
|---|---|
| `/status` | Runtime status, as JSON. |
| `/check [name]` | Run the configured named checks, or one named check. |
| `/tools` | List the tool surfaces the runtime exposes. |
| `/models` | List installed models from the Ollama backend. If Ollama is unreachable, the TUI warns on stderr and continues with an empty list — it never hangs waiting. |
| `/model <name>` | Switch the model for **all** roles: assigns `ollama.model` and clears the per-role overrides, so planner, coder, and reviewer all follow. Any non-empty name is accepted (no allowlist). |
| `/build <request>` | A build request handed to the runtime. |
| `/clear` | Prints the command list again (shares `/help`'s output). The note it carries explains why: *"Every task has fresh role contexts; /clear needs no persistent cleanup."* |
| `/quit`, `/exit` | Leave the loop, exit 0. |

Three legacy commands still exist but are permanently disabled:
`/plugins`, `/evolve`, `/memory`, `/feedback`. They return
`"status": "unavailable"` with the message: *"Legacy executable
plugins, automatic prompt mutation and cross-workspace memory are
disabled in the safe runtime."* An unknown `/command` returns an
error: `Unknown command; try /help`.

Now put it to work: [Workflows](workflows.md).
