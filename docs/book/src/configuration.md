# User Guide: Configuration

Think of the config file as the workshop's permission slip. It names the
workshop (the workspace directory), the phone line to the model
(Ollama), how long anyone may work (the agent budgets), and which
machines the foreman is allowed to run (the checks). Everything in it is
validated *before* anything runs: out-of-range values, empty model
names, non-loopback Ollama URLs without explicit opt-in, and missing
workspaces are all rejected, and a rejected config leaves nothing
behind — there is no partial config to observe.

Every option below comes from `crates/phlow-config` (`model.rs` for
the schema and validated ranges, `load.rs` for loading rules). The
shipped example is the repo-root `config.toml`.

## How the config is found

- `--config /path/to/file.toml` names it explicitly. Passing the file
  **approves the file's contents**; it grants no execution trust.
- Without `--config`, the XDG `phlow/` location wins. The legacy
  `flow/` location is a migration fallback that emits a deprecation
  warning.
- The file may not exceed 1 MiB (`CONFIG_FILE_BYTES_MAX = 1_048_576`
  bytes); anything larger is refused before parsing.
- Unknown keys are rejected at the root and inside every section. The
  legacy `shell`/`plugins` keys are rejected as unknown options.
- `trusted` is **CLI/API only** (`--trusted`). A `trusted` key in
  TOML is rejected as an unknown option — trust can never be smuggled
  in through the config file.

## Root

| Key | Default | Rules |
|---|---|---|
| `workspace_dir` | `.` | Must be an existing directory. Workspace values must sit at the root, before any TOML table header. `--workspace` on the CLI wins over this. |

## [ollama]

| Key | Default | Valid range / rules |
|---|---|---|
| `base_url` | `http://127.0.0.1:11434` | HTTP(S) origin of the Ollama daemon. Loopback by default; a non-loopback URL is rejected unless `allow_remote = true`. |
| `model` | `qwen2.5-coder:7b` | Must be a nonempty string. Roles with empty names inherit this. |
| `temperature` | `0.2` | 0–2. |
| `context_length` | `16384` | 1024–131072 tokens. |
| `timeout` | `120` | 0.1–600 **seconds**. Note: this is seconds in `[ollama]`, unlike the millisecond timeouts in `[checks]`. |
| `allow_remote` | `false` | Whether a non-loopback `base_url` is explicitly permitted. |

## [models]

Per-role model overrides. Each is a string; an **empty string inherits
`ollama.model`**. Any non-empty name is accepted — there is no
allowlist, so a typo is just a model that Ollama will not have.

| Key | Default |
|---|---|
| `planner` | `""` (inherits) |
| `coder` | `""` (inherits) |
| `reviewer` | `""` (inherits) |

The TUI's `/model <name>` assigns `ollama.model` and clears all three
overrides, so every role follows the switch.

## [agent]

The budgets. These are the physics: every run is bounded by them, and
no prompt or plugin can widen them.

| Key | Default | Valid range | Means |
|---|---|---|---|
| `max_iterations` | `6` | 1–32 | Model turns per role, per cycle. |
| `max_cycles` | `2` | 1–5 | planner → coder → reviewer cycles. |
| `max_tool_calls` | `64` | 1–256 | Global tool-call budget across all roles and cycles. |
| `max_context_chars` | `100000` | 4096–1_000_000 | Context window cap, in characters. |
| `max_task_chars` | `16000` | 1–100_000 | Task description cap, in characters. |

## [checks.NAME]

Named, operator-approved checks. Each `[checks.<name>]` table defines
one check the runtime may run:

| Key | Rules |
|---|---|
| `cmd` | **Required.** The exact argv array, executed **without a shell**. Example: `[".venv/bin/python", "-m", "pytest", "-q"]`. |
| `timeout` | Timeout in **milliseconds** (e.g. `120000`). Unlike `[ollama]`'s seconds, checks measure in ms, matching Rose's convention. |
| `required` | Boolean. When `true`, verification fails if this check is missing or failing. |
| `kind` | One of `check`, `test`, `lint`, `typecheck`, `diagnostics`, `build` (from `CheckKind`). Descriptive. |
| `filetypes` | List of strings (e.g. `["python"]`). Descriptive — a required check is never silently skipped because of filetypes. |

Two things the operator owns, and phlow does not do for you:

1. The executables in `cmd` **must already exist in the target
   project's environment**. phlow runs them; it does not install them.
2. On Linux, check children run under a seccomp egress filter
   (`phlow-seccomp`): `socket(AF_INET/AF_INET6)` and
   `io_uring_setup` fail with `EPERM`. A check cannot phone home.

## A complete example

The repo's own `config.toml`, annotated:

```toml
workspace_dir = "."

[ollama]
base_url = "http://127.0.0.1:11434"
model = "qwen2.5-coder:7b"
temperature = 0.2
context_length = 16384
timeout = 120 # seconds
allow_remote = false

[models]
# Empty strings inherit ollama.model. Set any role to a separately
# installed local model.
planner = ""
coder = ""
reviewer = ""

[agent]
max_iterations = 6
max_cycles = 2
max_tool_calls = 64
max_context_chars = 100000
max_task_chars = 16000

[checks.tests]
cmd = [".venv/bin/python", "-m", "pytest", "-q"]
timeout = 120000 # milliseconds
required = true
kind = "test"
filetypes = ["python"]

[checks.lint]
cmd = [".venv/bin/ruff", "check", "flow", "tests"]
timeout = 30000
required = true
kind = "lint"
filetypes = ["python"]
```

With the slip signed, the controls are yours: [TUI
Controls](tui-controls.md).
