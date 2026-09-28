# task-93: improvement sandbox validation

**Kind:** rust · **Status:** fail (open) · **Wave:** 91–95 · **Commits:** pending (wave 91-95)

## ELI5

Imagine a science lab with a sealed testing chamber — nothing gets in or out except through monitored airlocks, and anything the experiment tries to do outside its dish is stopped and reported as "the chamber was breached," not as "the experiment failed." Phlow's evaluator *names* such a chamber — there's a stage called "Prepare the isolated workspace and snapshots" and a state called `Isolated` ("Candidate workspace created in isolation") — but when you look inside, the chamber is a painted backdrop. The `prepare` step is one line: move to the next stage. No workspace is created, no network is cut, no writes are contained. A proposal's tests would run with full network access and full filesystem access, and there is no mechanism to tell "the candidate misbehaved" apart from "the candidate's tests failed."

## What this task attempts

- **Goal:** verify candidate evaluation runs in a genuine sandbox — isolated workspace, no network, write-contained, with policy violations reported as violations rather than test failures — or document the absence with source evidence.
- **Mechanism:** `src/tasks/task_93.rs` runs four bounded source recons: `prepare_is_skeleton` (the `prepare` method's doc comment declares "Skeleton: order only"; its body is `self.advance(EvalStage::Prepare)` with no workspace/snapshot logic); `no_network_isolation` (zero hits for `unshare`/`network_isolation`/`no_network`/`netns`/`sandbox_network`); `no_write_containment` (zero hits for `chroot`/`write_containment`/`readonly_mount`/`pivot_root`/`sandbox_dir`); `isolation_advertised_but_unimplemented` (`Lifecycle::Isolated` is documented as "Candidate workspace created in isolation" but no `isolated_workspace` implementation exists — the state name and stage docs promise a sandbox the code does not build).
- **Success criterion:** sandbox verified, or the absence documented with source evidence.
- **Non-goals:** building a sandbox on gauntlet authority (product decision — banked, never implemented here).

## What happened

Honest FAIL at `where = "seam"`, first attempt — the seam is ABSENT as designed:

- **V1:** `prepare` is the documented skeleton — `self.advance(EvalStage::Prepare)`, order enforcement only. The doc comment above the signature says "Prepares the isolated workspace and snapshots. Skeleton: order only." — the honesty is in the source.
- **V2:** no network isolation — zero hits across every product crate. A network-dependent proposal's tests would run with full network access.
- **A1:** no write containment — zero hits. An escaping proposal's writes are not denied by any sandbox layer, so the design's "policy violation, not test failure" distinction has no mechanism behind it.
- **A2:** advertised but unimplemented — `Lifecycle::Isolated` and the `Prepare` stage docs promise isolation; the transition into `Isolated` performs none. Names and docs promise a sandbox; the code does not build one.

## Full technical depth

The recon uses bounded exact-token scans (word-boundary, case-sensitive) over `crates/*/src/**/*.rs`, excluding the gauntlet crate itself per the harness-probe principle (established in wave 81-85: harness probes use design vocabulary; only product crates count). The `prepare_is_skeleton` case is the most delicate: it separates the doc comment (which must contain "Skeleton: order only") from the method body (which must contain no workspace/snapshot logic) — an earlier draft checked the body for the skeleton marker and would have falsely failed, since the marker lives in the doc comment above the signature.

The `Lifecycle::Isolated` state is real vocabulary in `promotion.rs`, and the `Evaluator` stage machine is real order enforcement — what is absent is any isolation primitive behind them. This is not a half-built sandbox; it is a named-but-unbuilt one, honestly labeled as a skeleton in the source.

Product decision banked for Matt: whether to build a genuine network-disabled, write-contained validation sandbox with typed policy-violation reporting. Not implemented on gauntlet authority.

## Sources

- `crates/phlow-experiment/src/evaluator.rs` — `Evaluator::prepare` (the documented skeleton) and the `EvalStage` order machine
- `crates/phlow-experiment/src/promotion.rs` — `Lifecycle::Isolated` ("Candidate workspace created in isolation")
- `~/workspace/gauntlet-design-tasks-71-100.md` — task-93 design (Wave 91–95)
