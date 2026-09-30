# User Guide: Workflows

A day with phlow has a rhythm: point it at a project, prove the gates
work, then run tasks — one-shot from the shell, interactively in the
TUI, or continuously from your editor over MCP. Every step below is
grounded in the CLI contract ([The CLI](cli.md)) and the config schema
([Configuration](configuration.md)).

## 1. Point it at a project

Every phlow invocation needs a workspace: the single directory the
runtime is allowed to touch. Set it at the config root as
`workspace_dir`, or pass `--workspace /absolute/path/to/project`
(which wins over the TOML). Use absolute paths — relative ones resolve
against wherever you happen to be standing, which is how operators
surprise themselves.

## 2. Check the workshop is alive

```sh
phlow status --workspace /absolute/path/to/project
```

`status` reports local capabilities and makes **no model call**, so it
works with Ollama down. If this fails, stop: nothing else will work
either. Exit codes are part of the contract — `0` success, `1` the
command ran but the report status is not `"ok"`, `2` usage or
config error (the message starts `Phlow: `), `130` interrupted.

## 3. Approve the checks, then run them

Write an operator-reviewed config (see [Configuration](configuration.md))
whose `[checks.*]` tables name real executables in the *project's*
environment — phlow does not install them for you. Then:

```sh
phlow check --config /absolute/path/to/phlow-operator.toml --trusted
```

Two trust facts to keep straight:

- Passing `--config` **approves the file's contents**; it grants no
  execution trust.
- `--trusted` grants execution trust, and it is CLI-only — it can
  never come from the TOML file.

`phlow check [NAME]` runs all named checks, or one by name
(`--name` wins over the positional). A `required = true` check that
is missing or failing fails verification. On Linux the check children
run under a seccomp egress filter: no network, no new privilege. If a
check needs the network, it was never a valid check.

## 4. Run one task, get one report

```sh
phlow run "add a regression test for the timeout handling" --config cfg.toml --trusted
```

`run` sends the task string through the bounded loop — planner,
coder, host verification, reviewer — and prints **one ASCII-escaped JSON
line**, byte-compatible with Python's
`json.dumps(result, ensure_ascii=True)`. That single line *is* the
report; pipe it into `jq`, log it, or feed it to the next tool. The
budgets from `[agent]` bound the whole thing: six model turns per role
per cycle, two cycles, sixty-four tool calls by default.

## 5. Stay a while: the interactive loop

```sh
phlow tui
```

Or just `phlow` with no command — it drops into the TUI exactly like
the Python one did. Type tasks in plain language; use the slash commands
for the machinery: `/model qwen2.5-coder:14b` to switch every role at
once, `/check tests` to re-run the gates after the coder finishes,
`/status` to see where you stand, `/quit` or Ctrl-D to leave. All
the controls are in [TUI Controls](tui-controls.md); the one to
internalize is Ctrl-C, which *terminates* the process in the Rust TUI
instead of continuing the loop.

## 6. Plug in your editor

```sh
phlow serve
```

`serve` speaks newline-delimited JSON-RPC (MCP) over stdin/stdout —
the contract is in [The MCP stdio contract](mcp.md). This is how
rose.nvim drives phlow: Rose calls phlow over MCP stdio, and phlow calls
back into Rose's native editor tools over the private Neovim socket
(see [The private Neovim socket](editor.md); `--nvim` points at it,
`--editor-timeout` bounds the round-trip at 0.1–660 s).

The editor loop is the same workshop, just with the customer handing
work orders through the window continuously instead of one at a time.

## The shape of a good session

1. `status` — the workshop is alive.
2. `check --trusted` — the gates are green *before* the model writes
   anything.
3. `run` or the TUI — bounded work, one report per task.
4. `check` again — the foreman re-verifies after the coder; a
   required check that fails means the change does not ship.

That is the whole discipline: trust is explicit, budgets are walls, and
verification is a gate, not a suggestion.
