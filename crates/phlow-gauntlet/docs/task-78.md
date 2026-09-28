# task-78: model download and cache budgets

**Kind:** rust (adversarial) · **Status:** fail (open) · **Wave:** 76–80 · **Commits:** pending (wave 76-80)

## ELI5

Downloading a model is "fetching a multi-gigabyte file from the internet and keeping it on disk." The design wants that done carefully: cap the bytes actually observed (never trust the server's advertised size — "2GB" metadata serving 40GB of data must be cut off), verify resumed downloads by hash, evict old models with an explicit policy (LRU, never the active one), and never mistake a half-downloaded file for a complete model. Phlow does none of this — because phlow never downloads models at all. There is no download client, no HF hub client, no pull path, no blob cache, no resume logic, no revision pinning. The only `downloads` hit in the workspace is the safe-runtime policy DENYING downloads. Models are served by Ollama over HTTP; model bytes never enter phlow's address space. The byte caps that DO exist guard other seams: `RESPONSE_BYTES_MAX` (2 MiB, Ollama response bodies) and `FILE_BYTES_MAX` (256 KiB, workspace file reads).

## What this task attempts

- **Goal:** verify the model-manager seam — observed-byte caps independent of advertised sizes, hash-verified resume, explicit eviction with pinned exemption, incomplete-never-loadable — or document the absence with file evidence (the design explicitly allows the documented absence as the finding).
- **Mechanism:** `src/tasks/task_78.rs` runs exact-token source scans (task_48 pattern): no_download_client (`download`/`downloads`/`huggingface`/`hf` — every hit classified: the prompt.rs denial or gauntlet harness vocabulary), no_snapshot_revision_cache (`snapshot`/`blob`/`revision` — every hit classified into unrelated senses, anything unclassified fails the case as "finding refuted"), no_observed_byte_cap (the existing caps cited as file evidence for other seams), incomplete_never_marked (no partials, no markers — vacuous, not enforced).
- **Success criterion:** a model download/cache manager meeting the pass criteria, or the absence documented with file evidence (and banked for Matt as a product decision).
- **Non-goals:** inventing an HF model manager on gauntlet authority (it is a product decision, not a bug fix).

## What happened

Honest FAIL at `where = "seam"`, first attempt — the absence IS the finding:

- **V1:** no download client — `download` 0 hits; `downloads` 1 hit, the safe-runtime policy in `crates/phlow-runtime/src/prompt.rs` denying downloads; `huggingface`/`hf` 0 product hits.
- **V2:** no snapshot/blob-cache/revision machinery — every hit classifies into unrelated senses (workspace/editor/run/context/experiment snapshots; report/manifest/workflow revisions); the case fails loudly on anything unclassified.
- **A1:** no observed-byte cap — a lying Content-Length has no phlow code to bound it; the existing caps are cited as file evidence that they guard other seams.
- **A2:** incomplete downloads are never marked — there are no partial files and no resume logic; the design's criteria are unrepresentable without a downloader.

## Full technical depth

The probe scans every `crates/*/src/**/*.rs` with exact-token (case-insensitive) matching, bounded like task_48, excluding only the gauntlet crate itself (the harness's own probes use the design vocabulary). The classification sets are explicit in the driver: the V1 denial is pinned to `crates/phlow-runtime/src/prompt.rs` ("No arbitrary commands, cwd overrides, downloads, plugins or outside-workspace access"), and the V2 classified crates are phlow-workspace, phlow-editor, phlow-runtime, phlow-agent, phlow-experiment, phlow-council, phlow-checks — each verified as an unrelated sense. Any hit outside the classification fails the case with "finding refuted — inspect before proceeding", so the finding stays falsifiable as the codebase evolves. The eviction-policy criterion is vacuous, not enforced: with no cache there is no LRU, no pinned set, and no risk of evicting the active model — the design's criteria need a manager, and there is none.

Banked for Matt (product decision, NOT auto-implemented on gauntlet authority): whether phlow should gain an HF model manager — download with observed-byte caps, hash-verified resume, blob cache with explicit eviction, incomplete markers. Ollama owns model fetching today; that is a product decision, not a bug fix. The gauntlet documents the gap and stops.

## Sources

- `~/workspace/repos/phlow/crates/phlow-runtime/src/prompt.rs` — safe-runtime policy denying downloads
- `~/workspace/repos/phlow/crates/phlow-llm/src/transport.rs` — `RESPONSE_BYTES_MAX` (Ollama response bodies)
- `~/workspace/repos/phlow/crates/phlow-workspace/src/workspace.rs` — `FILE_BYTES_MAX` (workspace reads)
- `~/workspace/gauntlet-design-tasks-71-100.md` — task-78 design (Wave 76–80)
