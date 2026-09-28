# task-69: hallucinated tool rejection

**Kind:** nvim-lua · **Status:** pass · **Wave:** 66–70 · **Commits:** pending (wave 66-70)

## ELI5

A language model can invent a tool name that sounds real — "read_files_fast" — and ask the runtime to call it. The safe behavior is exact matching: the name is either in the registry (call it) or it isn't (reject with a clean "unknown tool" error). Fuzzy matching here would be a vulnerability: a name one typo away from the privileged `file_write` could get treated as the real thing. Diver's dispatcher does this exactly right: `ai.rose.tools.M.call(name, args)` is a table lookup `by_name[name]` with `assert(spec, 'unknown tool: ...')` *before* argument validation — the name is checked first, args second. The driver proves it with the design's hallucination example plus a 10-name edit-distance-1 battery (including near misses of `file_write`): all 10 rejected, a near-miss write attempt creating no file, and a hallucinated name with valid args rejected on the *name* (not the args). (Distinct from task-04: unknown *adapter* at config time; and from task-10: the tool exists but its description is malicious. This is an unknown *tool* at model-output time.)

## What this task attempts

- **Goal:** verify the tool dispatcher rejects hallucinated names exactly: real tool dispatches, hallucination cleanly rejected, edit-distance-1 near misses rejected (no fuzzy matching), rejection happens on the name before args are parsed.
- **Mechanism:** `lua/gauntlet/task_69.lua` drives the REAL `ai.rose.tools.M.call` in headless Neovim: a scratch dir with a readable file for `file_read`, a canary path for the near-miss write, and adversarial callers feeding scripted names.
- **Success criterion:** exact-match dispatch proven by the near-miss battery; rejections suggest nothing executable.
- **Non-goals:** touching the real tool registry; executing real writes.

## What happened

Honest PASS, first attempt:

- **V1:** `file_read` dispatches by exact name and returns the exact file bytes.
- **V2:** the design's hallucination example `read_files_fast` gets a clean `unknown_tool` rejection — no fuzzy match, no did-you-mean, nothing executed.
- **A1:** 10/10 edit-distance-1 near misses rejected — including `file_writes`, `file_writ`, `gile_write` variants of the privileged `file_write`; a would-be write through a near-miss name created no file. Similarity is not authority.
- **A2:** a hallucinated name with *valid* args is rejected as `unknown tool`, while a real tool with bogus args fails as `unknown argument` — proving name lookup runs before arg parsing by construction.

## Full technical depth

The mechanism is structural, not conventional: `by_name[name]` is a Lua table lookup — exact key match is the only way a spec is found. `assert(spec, 'unknown tool: ' .. tostring(name))` raises *before* `validate_args(args, spec)` runs, so no argument parsing, coercion, or error path for a hallucinated name can ever see the args. A source read of the module confirmed zero fuzzy-matching, edit-distance, or did-you-mean code anywhere in the dispatcher. The rejection message names the unknown tool and nothing else — it suggests no executable alternative.

## Sources

- `~/workspace/repos/diver/lua/ai/rose/tools.lua` — `M.call`, `by_name[name]` lookup, `unknown tool` assert, `validate_args`
- `~/workspace/gauntlet-design-tasks-21-70.md` — task-69 design (Wave 12)
