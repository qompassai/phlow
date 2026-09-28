# crates/phlow-gauntlet/flake.nix
#
# Phlow 280-task orchestration gauntlet: Nix entry points for the task
# drivers in `crates/phlow-gauntlet` (tasks 01-20 designed in
# ~/workspace/gauntlet-design.md; tasks 21-70 designed in
# ~/workspace/gauntlet-design-tasks-21-70.md; tasks 71-100 designed in
# ~/workspace/gauntlet-design-tasks-71-100.md; tasks 101-115 designed in
# ~/workspace/gauntlet-design-tasks-101-115.md; tasks 116-130 designed in
# ~/workspace/gauntlet-design-tasks-116-130.md; tasks 131-150 designed in
# ~/workspace/gauntlet-design-tasks-131-150.md; tasks 151-200 designed in
# ~/workspace/gauntlet-design-tasks-151-200.md; tasks 201-250 designed in
# ~/workspace/gauntlet-design-tasks-201-250.md; tasks 251-280 designed in
# ~/workspace/gauntlet-design-tasks-251-280.md).
#
# ROSTER SCHEME (two levels — read this before extending):
# - `gauntletRoster` declares EVERY task as pure data: { id, name, kind },
#   split into `tasks01to105` (implemented: drivers for 01-105 all landed
#   in this crate, waves 01-20), `tasks116to130` (implemented: diver
#   harness Phase-2 acceptance probes, waves 116-120, 121-125, and
#   126-130), `tasks106to110` (implemented: SkillOpt wave 21, drivers
#   landed), `tasks111to115` (implemented: SkillOpt wave 22, drivers
#   landed), `tasks131to135` (implemented: bug-bounty wave 23, drivers
#   landed), `tasks136to140` (implemented: bug-bounty wave 24, drivers
#   landed), `tasks141to145` (implemented: bug-bounty wave 25, drivers
#   landed), `tasks146to150` (implemented: bug-bounty wave 26, drivers
#   landed), `tasks151to158` (implemented: Ghostex adaptation wave 25,
#   drivers landed),
#   `tasks151to200` (pending: Ghostex adaptation waves 26-32,
#   designed in the extension docs, drivers not yet landed),
#   `tasks201to250` (pending: zeroclaw adaptation waves 33-36,
#   designed in the extension docs, drivers not yet landed), and
#   `tasks251to280` (pending: verification-against-adaptation waves
#   37-40, designed in the extension docs, drivers not yet landed;
#   run after tasks201to250).
#   Roster asserts validate ALL 280 entries.
# - Apps (`apps.task-NN`), the `gauntlet` runner, and `checks.task-list`
#   cover IMPLEMENTED tasks only. The flake never claims an app for a
#   task without a driver. This deliberately deviates from the design
#   docs' flake plans ("assert all 100/115 apps exist"): the plan is
#   data-right but honesty-wrong while drivers 101-115 do not exist.
# - When a wave worker lands a driver, move that task's entry from the
#   pending side to the implemented side (`rosterImplemented` grows,
#   `rosterPending` shrinks). The asserts, app generation, runner, and
#   checks all consume the two segments via `gauntletRoster` and
#   `implementedTasks` — no other edits needed. The implemented side is
#   complete through task-130; the pending side holds tasks 131-200
#   until their drivers land.
# - `tasks01to60` was once referenced but never defined (dangling binding
#   in the 70-task flake); it is defined here so the implemented
#   concatenation evaluates. Nothing is hardcoded to 130 except the
#   named bound `taskCountMax`.
#
# INPUT PINS: `nixpkgs` tracks the `nixos-25.05` branch and `flake-utils`
# tracks its default branch. No `flake.lock` exists yet (no Nix evaluator in
# the authoring sandbox), so BEFORE this flake is done:
#   1. run `nix flake lock` on primo,
#   2. review the lock,
#   3. commit it.
# `nix flake check` and every `nix run` app below also execute on primo.
#
# HONEST BOUNDARIES (deliberate, documented):
# - Task apps are NOT sandboxed. Cargo needs network on first run for the
#   crates.io registry: the toolchain is pinned, the registry is not
#   vendored. The task source is the working tree, never a store copy.
# - Headless nvim runs with diver's `lua/` on the runtimepath so
#   `require("ai.harness")` loads Matt's actual config code; the full diver
#   `init.lua` is NOT bootstrapped (it would pull plugin management and
#   network). Harness modules under test are byte-identical to his config.
# - No secrets exist anywhere near this flake; nothing here places private
#   material in the store, a derivation, or a log.
#
# Evaluator: Nix 2.x with the `nix-command` and `flakes` experimental
# features enabled. Nixpkgs: nixos-25.05 (locked on primo, see above).
{
  description = "phlow 280-task gauntlet: declared roster, apps for implemented drivers";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.05";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    { nixpkgs, flake-utils, ... }:
    let
      inherit (nixpkgs) lib;

      # Named bound on the declared roster: exactly 280 entries
      # (task-01..task-280), no more.
      taskCountMax = 280;

      # Roster segment: tasks 01-50. Drivers landed in this crate
      # (`src/tasks/task_01.rs` .. `src/tasks/task_50.rs`).
      # Contract: each entry is { id, name, kind } with id "task-NN",
      # name from the gauntlet-design.md task table, and kind in
      # { "nvim-lua", "rust" }.
      tasks01to45 = [
        {
          id = "task-01";
          name = "fan-out/fan-in verdict aggregation";
          kind = "nvim-lua";
        }
        {
          id = "task-02";
          name = "budget exhaustion fails closed";
          kind = "nvim-lua";
        }
        {
          id = "task-03";
          name = "approval gate blocks unapproved tool use";
          kind = "nvim-lua";
        }
        {
          id = "task-04";
          name = "unknown adapter → invalid_adapter";
          kind = "nvim-lua";
        }
        {
          id = "task-05";
          name = "cancel/resume semantics";
          kind = "nvim-lua";
        }
        {
          id = "task-06";
          name = "ACP interop round-trip";
          kind = "nvim-lua";
        }
        {
          id = "task-07";
          name = "A2A task lifecycle";
          kind = "nvim-lua";
        }
        {
          id = "task-08";
          name = "MCP stdio tool bridging";
          kind = "nvim-lua";
        }
        {
          id = "task-09";
          name = "prompt injection via tool output";
          kind = "nvim-lua";
        }
        {
          id = "task-10";
          name = "malicious MCP tool description";
          kind = "nvim-lua";
        }
        {
          id = "task-11";
          name = "self-approval rejected";
          kind = "rust";
        }
        {
          id = "task-12";
          name = "poisoned context compaction";
          kind = "rust";
        }
        {
          id = "task-13";
          name = "scheduler overload";
          kind = "rust";
        }
        {
          id = "task-14";
          name = "evaluator budget exhaustion";
          kind = "rust";
        }
        {
          id = "task-15";
          name = "lifecycle illegal transitions";
          kind = "rust";
        }
        {
          id = "task-16";
          name = "manifest validation";
          kind = "rust";
        }
        {
          id = "task-17";
          name = "nvim edit→check→fix loop";
          kind = "nvim-lua";
        }
        {
          id = "task-18";
          name = "worker crash recovery";
          kind = "nvim-lua";
        }
        {
          id = "task-19";
          name = "council arbitration";
          kind = "rust";
        }
        {
          id = "task-20";
          name = "deadline expiry / at-most-once";
          kind = "rust";
        }
        {
          id = "task-21";
          name = "saga compensating transactions";
          kind = "nvim-lua";
        }
        {
          id = "task-22";
          name = "DAG diamond dependencies";
          kind = "rust";
        }
        {
          id = "task-23";
          name = "bounded dynamic fan-out";
          kind = "nvim-lua";
        }
        {
          id = "task-24";
          name = "priority preemption";
          kind = "rust";
        }
        {
          id = "task-25";
          name = "backpressure propagation";
          kind = "rust";
        }
        {
          id = "task-26";
          name = "circuit breaker";
          kind = "rust";
        }
        {
          id = "task-27";
          name = "retry backoff under storm";
          kind = "rust";
        }
        {
          id = "task-28";
          name = "idempotency keys";
          kind = "rust";
        }
        {
          id = "task-29";
          name = "lease fencing";
          kind = "rust";
        }
        {
          id = "task-30";
          name = "leader election under partition";
          kind = "nvim-lua";
        }
        {
          id = "task-31";
          name = "optimistic concurrency";
          kind = "rust";
        }
        {
          id = "task-32";
          name = "event-sourced replay";
          kind = "rust";
        }
        {
          id = "task-33";
          name = "checkpoint durability";
          kind = "nvim-lua";
        }
        {
          id = "task-34";
          name = "adversarial concurrent merge";
          kind = "rust";
        }
        {
          id = "task-35";
          name = "duplicate delivery dedup";
          kind = "rust";
        }
      ]
      ++ [

        {
          id = "task-36";
          name = "tool-output exfiltration";
          kind = "nvim-lua";
        }
        {
          id = "task-37";
          name = "confused deputy";
          kind = "nvim-lua";
        }
        {
          id = "task-38";
          name = "ReDoS guard";
          kind = "rust";
        }
        {
          id = "task-39";
          name = "recursive payload bomb";
          kind = "rust";
        }
        {
          id = "task-40";
          name = "approval TOCTOU";
          kind = "nvim-lua";
        }
        {
          id = "task-41";
          name = "authority attenuation";
          kind = "nvim-lua";
        }
        {
          id = "task-42";
          name = "audit log append-only";
          kind = "rust";
        }
        {
          id = "task-43";
          name = "secrets in error messages";
          kind = "rust";
        }
        {
          id = "task-44";
          name = "egress filtering";
          kind = "nvim-lua";
        }
        {
          id = "task-45";
          name = "plugin dependency confusion";
          kind = "nvim-lua";
        }
      ];

      # Roster segment: tasks 46-50. Drivers landed in this crate
      # (`src/tasks/task_46.rs` .. `src/tasks/task_50.rs`).
      # Contract: each entry is { id, name, kind } with id "task-NN",
      # name from the gauntlet-design.md task table, and kind in
      # { "nvim-lua", "rust" }.
      tasks46to50 = [
        {
          id = "task-46";
          name = "context pressure compaction";
          kind = "rust";
        }
        {
          id = "task-47";
          name = "file descriptor exhaustion";
          kind = "rust";
        }
        {
          id = "task-48";
          name = "disk quota enforcement";
          kind = "rust";
        }
        {
          id = "task-49";
          name = "CPU fairness";
          kind = "rust";
        }
        {
          id = "task-50";
          name = "bounded event buffers";
          kind = "rust";
        }
      ];

      # Roster segment: tasks 51-55. Drivers landed in this crate
      # (`src/tasks/task_51.rs` .. `src/tasks/task_55.rs`).
      # Contract: each entry is { id, name, kind } with id "task-NN",
      # name from the gauntlet-design.md task table, and kind in
      # { "nvim-lua", "rust" }.
      tasks51to55 = [
        {
          id = "task-51";
          name = "byzantine worker detection";
          kind = "nvim-lua";
        }
        {
          id = "task-52";
          name = "straggler mitigation";
          kind = "nvim-lua";
        }
        {
          id = "task-53";
          name = "quorum reads and writes";
          kind = "rust";
        }
        {
          id = "task-54";
          name = "gossip convergence";
          kind = "rust";
        }
        {
          id = "task-55";
          name = "dissent escalation";
          kind = "nvim-lua";
        }
      ];

      # Roster segment: tasks 56-60. Drivers landed in this crate
      # (`src/tasks/task_56.rs` .. `src/tasks/task_60.rs`). Same
      # { id, name, kind } contract as above.
      tasks56to60 = [
        {
          id = "task-56";
          name = "approval timeout defaults deny";
          kind = "nvim-lua";
        }
        {
          id = "task-57";
          name = "escalation chains";
          kind = "nvim-lua";
        }
        {
          id = "task-58";
          name = "dual control";
          kind = "rust";
        }
        {
          id = "task-59";
          name = "approval scope binding";
          kind = "nvim-lua";
        }
        {
          id = "task-60";
          name = "break-glass procedure";
          kind = "nvim-lua";
        }
      ];

      # Roster segment: tasks 61-65. Drivers landed in this crate
      # (wave 12, ~/workspace/gauntlet-design-tasks-21-70.md). Same
      # { id, name, kind } contract as above.
      tasks61to65 = [
        {
          id = "task-61";
          name = "plan schema validation";
          kind = "nvim-lua";
        }
        {
          id = "task-62";
          name = "replanning on partial failure";
          kind = "nvim-lua";
        }
        {
          id = "task-63";
          name = "decomposition depth bound";
          kind = "rust";
        }
        {
          id = "task-64";
          name = "deterministic tool selection";
          kind = "nvim-lua";
        }
        {
          id = "task-65";
          name = "plan cost estimation";
          kind = "rust";
        }
      ];

      # Roster segment: tasks 66-70. Designed in
      # ~/workspace/gauntlet-design-tasks-21-70.md; drivers landed in
      # wave 66-70. Same { id, name, kind } contract as above.
      tasks66to70 = [
        {
          id = "task-66";
          name = "trace propagation";
          kind = "nvim-lua";
        }
        {
          id = "task-67";
          name = "audit completeness";
          kind = "rust";
        }
        {
          id = "task-68";
          name = "metric cardinality bound";
          kind = "rust";
        }
        {
          id = "task-69";
          name = "hallucinated tool rejection";
          kind = "nvim-lua";
        }
        {
          id = "task-70";
          name = "retrieved-document poisoning";
          kind = "nvim-lua";
        }
      ];

      # Roster segment: tasks 71-75. Designed in
      # ~/workspace/gauntlet-design-tasks-71-100.md (wave 14);
      # drivers landed in this crate
      # (`src/tasks/task_71.rs` .. `src/tasks/task_75.rs`). Same
      # { id, name, kind } contract as above.
      tasks71to75 = [
        {
          id = "task-71";
          name = "delegation depth attribution";
          kind = "nvim-lua";
        }
        {
          id = "task-72";
          name = "context handoff fidelity";
          kind = "nvim-lua";
        }
        {
          id = "task-73";
          name = "subagent failure containment";
          kind = "nvim-lua";
        }
        {
          id = "task-74";
          name = "result aggregation under partial failure";
          kind = "rust";
        }
        {
          id = "task-75";
          name = "delegation cycle detection";
          kind = "nvim-lua";
        }
      ];

      # Roster segment: tasks 76-80. Designed in
      # ~/workspace/gauntlet-design-tasks-71-100.md (wave 15);
      # drivers landed in the wave 76-80 commit, so this segment is
      # IMPLEMENTED. Same { id, name, kind } contract as above.
      tasks76to80 = [
        {
          id = "task-76";
          name = "tokenizer boundary mismatch";
          kind = "rust";
        }
        {
          id = "task-77";
          name = "quantization behavior change";
          kind = "rust";
        }
        {
          id = "task-78";
          name = "model download and cache budgets";
          kind = "rust";
        }
        {
          id = "task-79";
          name = "offline fallback";
          kind = "rust";
        }
        {
          id = "task-80";
          name = "local model selection tradeoffs";
          kind = "nvim-lua";
        }
      ];

      # Roster segment: tasks 81-85. Designed in
      # ~/workspace/gauntlet-design-tasks-71-100.md (wave 16);
      # drivers landed in the wave 81-85 commit, so this segment is
      # IMPLEMENTED. Same { id, name, kind } contract as above.
      tasks81to85 = [
        {
          id = "task-81";
          name = "provider auth failure modes";
          kind = "rust";
        }
        {
          id = "task-82";
          name = "provider rate-limit protocol compliance";
          kind = "rust";
        }
        {
          id = "task-83";
          name = "streaming vs non-streaming parity";
          kind = "rust";
        }
        {
          id = "task-84";
          name = "provider outage failover";
          kind = "rust";
        }
        {
          id = "task-85";
          name = "provider usage accounting integrity";
          kind = "nvim-lua";
        }
      ];

      # Roster segment: tasks 86-90. Designed in
      # ~/workspace/gauntlet-design-tasks-71-100.md (wave 17);
      # drivers landed in the wave 86-90 commit, so this segment is
      # IMPLEMENTED. Same { id, name, kind } contract as above.
      tasks86to90 = [
        {
          id = "task-86";
          name = "MCP capability negotiation mismatch";
          kind = "nvim-lua";
        }
        {
          id = "task-87";
          name = "MCP session resumption";
          kind = "nvim-lua";
        }
        {
          id = "task-88";
          name = "A2A error-shape propagation";
          kind = "rust";
        }
        {
          id = "task-89";
          name = "protocol version skew";
          kind = "rust";
        }
        {
          id = "task-90";
          name = "namespaced cross-protocol dispatch";
          kind = "nvim-lua";
        }
      ];

      # Roster segment: tasks 91-95. Designed in
      # ~/workspace/gauntlet-design-tasks-71-100.md (wave 18);
      # drivers landed in the wave 91-95 commit, so this segment is
      # IMPLEMENTED. Same { id, name, kind } contract as above.
      tasks91to95 = [
        {
          id = "task-91";
          name = "propose→approve→apply pipeline";
          kind = "rust";
        }
        {
          id = "task-92";
          name = "proposal scope binding and drift detection";
          kind = "nvim-lua";
        }
        {
          id = "task-93";
          name = "improvement sandbox validation";
          kind = "rust";
        }
        {
          id = "task-94";
          name = "improvement rollback";
          kind = "rust";
        }
        {
          id = "task-95";
          name = "approval render integrity (WYSIWYG)";
          kind = "nvim-lua";
        }
      ];

      # Roster segment: tasks 96-100. Designed in
      # ~/workspace/gauntlet-design-tasks-71-100.md (wave 19);
      # drivers landed in the wave 96-100 commit, so this segment is
      # IMPLEMENTED. Same { id, name, kind } contract as above.
      tasks96to100 = [
        {
          id = "task-96";
          name = "cross-agent approval laundering";
          kind = "rust";
        }
        {
          id = "task-97";
          name = "gate self-modification attempt";
          kind = "rust";
        }
        {
          id = "task-98";
          name = "incremental composition attack";
          kind = "rust";
        }
        {
          id = "task-99";
          name = "approval fatigue and dark-pattern proposals";
          kind = "nvim-lua";
        }
        {
          id = "task-100";
          name = "artifact swap between pipeline stages";
          kind = "rust";
        }
      ];

      # Roster segment: tasks 101-115. Designed in
      # ~/workspace/gauntlet-design-tasks-101-115.md (waves 20-22,
      # SkillOpt extension); drivers have not landed, so this segment
      # is PENDING. Same { id, name, kind } contract as above.
      tasks101to105 = [
        {
          id = "task-101";
          name = "edit-budget bound ablation";
          kind = "rust";
        }
        {
          id = "task-102";
          name = "selection-gate ablation";
          kind = "rust";
        }
        {
          id = "task-103";
          name = "rejected-buffer ablation";
          kind = "rust";
        }
        {
          id = "task-104";
          name = "slow-meta ablation";
          kind = "rust";
        }
        {
          id = "task-105";
          name = "evidence-size robustness";
          kind = "rust";
        }
      ];
      tasks116to120 = [
        {
          id = "task-116";
          name = "goal-propagation";
          kind = "nvim-lua";
        }
        {
          id = "task-117";
          name = "a2a-completion-integrity";
          kind = "nvim-lua";
        }
        {
          id = "task-118";
          name = "policy-launch-enforcement";
          kind = "nvim-lua";
        }
        {
          id = "task-119";
          name = "resume-mutation-ordering";
          kind = "nvim-lua";
        }
        {
          id = "task-120";
          name = "run-failure-legality";
          kind = "nvim-lua";
        }
      ];

      tasks121to125 = [
        {
          id = "task-121";
          name = "deny-all default and explicit opt-in";
          kind = "nvim-lua";
        }
        {
          id = "task-122";
          name = "capability-based risk escalation";
          kind = "nvim-lua";
        }
        {
          id = "task-123";
          name = "HarnessRun argument parsing contract";
          kind = "nvim-lua";
        }
        {
          id = "task-124";
          name = "prompt fallback and abort atomicity";
          kind = "nvim-lua";
        }
        {
          id = "task-125";
          name = "run selection TOCTOU for cancel/resume";
          kind = "nvim-lua";
        }
      ];

      tasks126to130 = [
        {
          id = "task-126";
          name = "sink-append wakes supervision, no poll";
          kind = "nvim-lua";
        }
        {
          id = "task-127";
          name = "deadline one-shots and handle hygiene";
          kind = "nvim-lua";
        }
        {
          id = "task-128";
          name = "retry one-shot lifecycle";
          kind = "nvim-lua";
        }
        {
          id = "task-129";
          name = "approval-expiry one-shots";
          kind = "nvim-lua";
        }
        {
          id = "task-130";
          name = "idle-loop quietness";
          kind = "nvim-lua";
        }
      ];

      tasks131to135 = [
        {
          id = "task-131";
          name = "scheduled scope refresh";
          kind = "rust";
        }
        {
          id = "task-132";
          name = "scope diffing added removed";
          kind = "rust";
        }
        {
          id = "task-133";
          name = "scope revocation mid-cycle";
          kind = "rust";
        }
        {
          id = "task-134";
          name = "passive-active transition gating";
          kind = "rust";
        }
        {
          id = "task-135";
          name = "approval expiry";
          kind = "rust";
        }
      ];

      tasks136to140 = [
        {
          id = "task-136";
          name = "bounded concurrency queueing";
          kind = "rust";
        }
        {
          id = "task-137";
          name = "cancellation without zombies";
          kind = "rust";
        }
        {
          id = "task-138";
          name = "restart recovery without rescan";
          kind = "rust";
        }
        {
          id = "task-139";
          name = "finding dedup across cycles";
          kind = "rust";
        }
        {
          id = "task-140";
          name = "rate limits and testing windows";
          kind = "rust";
        }
      ];

      tasks141to145 = [
        {
          id = "task-141";
          name = "evidence preservation custody";
          kind = "rust";
        }
        {
          id = "task-142";
          name = "finding validation pipeline";
          kind = "rust";
        }
        {
          id = "task-143";
          name = "false-positive rejection";
          kind = "rust";
        }
        {
          id = "task-144";
          name = "report generation";
          kind = "rust";
        }
        {
          id = "task-145";
          name = "submission payload preview";
          kind = "rust";
        }
      ];

      tasks146to150 = [
        {
          id = "task-146";
          name = "operator approval boundary";
          kind = "rust";
        }
        {
          id = "task-147";
          name = "post-submission state tracking";
          kind = "rust";
        }
        {
          id = "task-148";
          name = "triager feedback ingestion";
          kind = "rust";
        }
        {
          id = "task-149";
          name = "secret non-disclosure";
          kind = "rust";
        }
        {
          id = "task-150";
          name = "hostile scope change refusal";
          kind = "rust";
        }
      ];

      # Implemented: Ghostex adaptation wave 25 (tasks 151-158); drivers landed.
      tasks151to158 = [
        {
          id = "task-151";
          name = "open-enum unknown variants";
          kind = "rust";
        }
        {
          id = "task-152";
          name = "unknown fields ignored";
          kind = "rust";
        }
        {
          id = "task-153";
          name = "lenient envelope parsing";
          kind = "rust";
        }
        {
          id = "task-154";
          name = "versioned envelope routing";
          kind = "rust";
        }
        {
          id = "task-155";
          name = "hostile frame rejection";
          kind = "rust";
        }
        {
          id = "task-156";
          name = "variant confusion refusal";
          kind = "rust";
        }
        {
          id = "task-157";
          name = "version skew monotonicity";
          kind = "rust";
        }
        {
          id = "task-158";
          name = "socket parity and license audit";
          kind = "rust";
        }
      ];

      # Pending: Ghostex adaptation waves 26-32 (tasks 159-200).
      # Designed in ~/workspace/gauntlet-design-tasks-151-200.md;
      # drivers not yet landed. Runs after tasks151to158.
      tasks151to200 = [
        {
          id = "task-159";
          name = "reconnect ladder";
          kind = "rust";
        }
        {
          id = "task-160";
          name = "subscribe on reconnect";
          kind = "rust";
        }
        {
          id = "task-161";
          name = "snapshot resync";
          kind = "rust";
        }
        {
          id = "task-162";
          name = "flapping daemon backoff";
          kind = "rust";
        }
        {
          id = "task-163";
          name = "hostile peer on daemon socket";
          kind = "rust";
        }
        {
          id = "task-164";
          name = "half-open connection timeout";
          kind = "rust";
        }
        {
          id = "task-165";
          name = "worker shutdown without zombies";
          kind = "rust";
        }
        {
          id = "task-166";
          name = "pure transition function";
          kind = "rust";
        }
        {
          id = "task-167";
          name = "effect interpreter separation";
          kind = "rust";
        }
        {
          id = "task-168";
          name = "hostile event injection";
          kind = "rust";
        }
        {
          id = "task-169";
          name = "effect ordering idempotency";
          kind = "rust";
        }
        {
          id = "task-170";
          name = "pairing code issuance";
          kind = "rust";
        }
        {
          id = "task-171";
          name = "pairing code TTL expiry";
          kind = "rust";
        }
        {
          id = "task-172";
          name = "secret hash comparison";
          kind = "rust";
        }
        {
          id = "task-173";
          name = "pairing brute-force rate limit";
          kind = "rust";
        }
        {
          id = "task-174";
          name = "pairing code replay refusal";
          kind = "rust";
        }
        {
          id = "task-175";
          name = "pairing code forgery refusal";
          kind = "rust";
        }
        {
          id = "task-176";
          name = "tailcat sidecar supervision";
          kind = "rust";
        }
        {
          id = "task-177";
          name = "pairing ceremony end to end";
          kind = "rust";
        }
        {
          id = "task-178";
          name = "ephemeral bridge launch";
          kind = "rust";
        }
        {
          id = "task-179";
          name = "bridge stdio framing";
          kind = "rust";
        }
        {
          id = "task-180";
          name = "devtools round trip";
          kind = "rust";
        }
        {
          id = "task-181";
          name = "hostile page payload bounds";
          kind = "rust";
        }
        {
          id = "task-182";
          name = "bridge navigation allowlist";
          kind = "rust";
        }
        {
          id = "task-183";
          name = "cancelled bridge reaping";
          kind = "rust";
        }
        {
          id = "task-184";
          name = "bridge cross-talk isolation";
          kind = "rust";
        }
        {
          id = "task-185";
          name = "no persistent mcp config";
          kind = "rust";
        }
        {
          id = "task-186";
          name = "scan-once sqlite index";
          kind = "rust";
        }
        {
          id = "task-187";
          name = "ranked fuzzy queries";
          kind = "rust";
        }
        {
          id = "task-188";
          name = "no per-agent parsers";
          kind = "rust";
        }
        {
          id = "task-189";
          name = "poisoned session index";
          kind = "rust";
        }
        {
          id = "task-190";
          name = "corrupt index rebuild";
          kind = "rust";
        }
        {
          id = "task-191";
          name = "query literal handling";
          kind = "rust";
        }
        {
          id = "task-192";
          name = "shared index concurrent readers";
          kind = "rust";
        }
        {
          id = "task-193";
          name = "scan plan apply sync";
          kind = "rust";
        }
        {
          id = "task-194";
          name = "idempotent skill sync";
          kind = "rust";
        }
        {
          id = "task-195";
          name = "skill supply-chain refusal";
          kind = "rust";
        }
        {
          id = "task-196";
          name = "concurrent modification safety";
          kind = "rust";
        }
        {
          id = "task-197";
          name = "help-first commands";
          kind = "rust";
        }
        {
          id = "task-198";
          name = "json output stability";
          kind = "rust";
        }
        {
          id = "task-199";
          name = "verify-after-act";
          kind = "rust";
        }
        {
          id = "task-200";
          name = "hostile output sanitization";
          kind = "rust";
        }
      ];

      # Pending: zeroclaw adaptation waves 33-36 (tasks 201-250).
      # Designed in ~/workspace/gauntlet-design-tasks-201-250.md;
      # drivers not yet landed. Runs after tasks151to200.
      tasks201to250 = [
        {
          id = "task-201";
          name = "jsonrpc frame validation";
          kind = "rust";
        }
        {
          id = "task-202";
          name = "invalid frame direction inference";
          kind = "rust";
        }
        {
          id = "task-203";
          name = "acp method-set spec diff";
          kind = "rust";
        }
        {
          id = "task-204";
          name = "wire field constants";
          kind = "rust";
        }
        {
          id = "task-205";
          name = "cancellation sentinel recognition";
          kind = "rust";
        }
        {
          id = "task-206";
          name = "failure sentinel locale independence";
          kind = "rust";
        }
        {
          id = "task-207";
          name = "session restore provenance";
          kind = "rust";
        }
        {
          id = "task-208";
          name = "localized-text cancellation rejection";
          kind = "rust";
        }
        {
          id = "task-209";
          name = "sentinel forgery refusal";
          kind = "rust";
        }
        {
          id = "task-210";
          name = "pairing-gated ws upgrade";
          kind = "rust";
        }
        {
          id = "task-211";
          name = "per-agent url alias";
          kind = "rust";
        }
        {
          id = "task-212";
          name = "stdio-ws bridge framing";
          kind = "rust";
        }
        {
          id = "task-213";
          name = "subprotocol version skew";
          kind = "rust";
        }
        {
          id = "task-214";
          name = "well-known index schema";
          kind = "rust";
        }
        {
          id = "task-215";
          name = "index size bound";
          kind = "rust";
        }
        {
          id = "task-216";
          name = "artifact digest verification";
          kind = "rust";
        }
        {
          id = "task-217";
          name = "dns dial pinning";
          kind = "rust";
        }
        {
          id = "task-218";
          name = "redirect chain cap";
          kind = "rust";
        }
        {
          id = "task-219";
          name = "traversal-safe unpack";
          kind = "rust";
        }
        {
          id = "task-220";
          name = "archive bounds";
          kind = "rust";
        }
        {
          id = "task-221";
          name = "hostile index policy refusal";
          kind = "rust";
        }
        {
          id = "task-222";
          name = "dns rebinding refusal";
          kind = "rust";
        }
        {
          id = "task-223";
          name = "artifact swap detection";
          kind = "rust";
        }
        {
          id = "task-224";
          name = "malicious archive refusal";
          kind = "rust";
        }
        {
          id = "task-225";
          name = "redirect loop refusal";
          kind = "rust";
        }
        {
          id = "task-226";
          name = "transport seam testability";
          kind = "rust";
        }
        {
          id = "task-227";
          name = "supervised unknown defaults ask";
          kind = "rust";
        }
        {
          id = "task-228";
          name = "auto-approve always-ask lists";
          kind = "rust";
        }
        {
          id = "task-229";
          name = "session allowlist semantics";
          kind = "rust";
        }
        {
          id = "task-230";
          name = "approval audit log";
          kind = "rust";
        }
        {
          id = "task-231";
          name = "autonomy level matrix";
          kind = "rust";
        }
        {
          id = "task-232";
          name = "non-interactive approval";
          kind = "rust";
        }
        {
          id = "task-233";
          name = "unknown tool prompt";
          kind = "rust";
        }
        {
          id = "task-234";
          name = "autonomy escalation refusal";
          kind = "rust";
        }
        {
          id = "task-235";
          name = "always-ask bypass refusal";
          kind = "rust";
        }
        {
          id = "task-236";
          name = "approval expiry race";
          kind = "rust";
        }
        {
          id = "task-237";
          name = "audit log append-only";
          kind = "rust";
        }
        {
          id = "task-238";
          name = "approval spoof resistance";
          kind = "rust";
        }
        {
          id = "task-239";
          name = "estop levels engage";
          kind = "rust";
        }
        {
          id = "task-240";
          name = "estop fail-closed";
          kind = "rust";
        }
        {
          id = "task-241";
          name = "estop selective resume";
          kind = "rust";
        }
        {
          id = "task-242";
          name = "otp validation";
          kind = "rust";
        }
        {
          id = "task-243";
          name = "webauthn verification";
          kind = "rust";
        }
        {
          id = "task-244";
          name = "prompt guard";
          kind = "rust";
        }
        {
          id = "task-245";
          name = "sandbox backend selection";
          kind = "rust";
        }
        {
          id = "task-246";
          name = "cross-init service dispatch";
          kind = "rust";
        }
        {
          id = "task-247";
          name = "estop under attack";
          kind = "rust";
        }
        {
          id = "task-248";
          name = "sandbox escape containment";
          kind = "rust";
        }
        {
          id = "task-249";
          name = "pairing guard bypass refusal";
          kind = "rust";
        }
        {
          id = "task-250";
          name = "secrets hygiene audit";
          kind = "rust";
        }
      ];

      # Pending: verification-against-adaptation waves 37-40 (tasks 251-280).
      # Designed in ~/workspace/gauntlet-design-tasks-251-280.md; drivers
      # not yet landed. Runs AFTER tasks201to250: these tasks test the
      # adaptations, so the adaptations must exist first.
      tasks251to280 = [
        {
          id = "task-251";
          name = "localized cancellation flood";
          kind = "rust";
        }
        {
          id = "task-252";
          name = "lenient vs permissive parsing";
          kind = "rust";
        }
        {
          id = "task-253";
          name = "out-of-order envelopes";
          kind = "rust";
        }
        {
          id = "task-254";
          name = "sentinel near-miss battery";
          kind = "rust";
        }
        {
          id = "task-255";
          name = "poisoned restore provenance";
          kind = "rust";
        }
        {
          id = "task-256";
          name = "ws pairing-gate bypass battery";
          kind = "rust";
        }
        {
          id = "task-257";
          name = "bridge secret hygiene";
          kind = "rust";
        }
        {
          id = "task-258";
          name = "acp conformance and license audit";
          kind = "rust";
        }
        {
          id = "task-259";
          name = "malicious index battery";
          kind = "rust";
        }
        {
          id = "task-260";
          name = "dns rebinding mid-fetch";
          kind = "rust";
        }
        {
          id = "task-261";
          name = "digest binding under attack";
          kind = "rust";
        }
        {
          id = "task-262";
          name = "archive hostile battery";
          kind = "rust";
        }
        {
          id = "task-263";
          name = "decompression bomb battery";
          kind = "rust";
        }
        {
          id = "task-264";
          name = "redirect attack chains";
          kind = "rust";
        }
        {
          id = "task-265";
          name = "index never sources policy";
          kind = "rust";
        }
        {
          id = "task-266";
          name = "installer bounds and license audit";
          kind = "rust";
        }
        {
          id = "task-267";
          name = "unknown-tool gauntlet";
          kind = "rust";
        }
        {
          id = "task-268";
          name = "escalation attempt battery";
          kind = "rust";
        }
        {
          id = "task-269";
          name = "always-ask laundering";
          kind = "rust";
        }
        {
          id = "task-270";
          name = "expiry race exploitation";
          kind = "rust";
        }
        {
          id = "task-271";
          name = "allowlist poisoning";
          kind = "rust";
        }
        {
          id = "task-272";
          name = "non-interactive denial semantics";
          kind = "rust";
        }
        {
          id = "task-273";
          name = "approval audit and license";
          kind = "rust";
        }
        {
          id = "task-274";
          name = "estop under fire";
          kind = "rust";
        }
        {
          id = "task-275";
          name = "sandbox escape battery";
          kind = "rust";
        }
        {
          id = "task-276";
          name = "pairing ceremony attacks";
          kind = "rust";
        }
        {
          id = "task-277";
          name = "otp attack battery";
          kind = "rust";
        }
        {
          id = "task-278";
          name = "prompt-guard evasion";
          kind = "rust";
        }
        {
          id = "task-279";
          name = "secret-zero audit";
          kind = "rust";
        }
        {
          id = "task-280";
          name = "security posture and license audit";
          kind = "rust";
        }
      ];

      tasks106to110 = [
        {
          id = "task-106";
          name = "loop convergence dynamics";
          kind = "rust";
        }
        {
          id = "task-107";
          name = "cross-family skill transfer";
          kind = "rust";
        }
        {
          id = "task-108";
          name = "cross-harness skill transfer";
          kind = "nvim-lua";
        }
        {
          id = "task-109";
          name = "adversarial edit catch rate";
          kind = "rust";
        }
        {
          id = "task-110";
          name = "poisoned rollout evidence";
          kind = "rust";
        }
      ];

      tasks111to115 = [
        {
          id = "task-111";
          name = "selection-split overfitting";
          kind = "rust";
        }
        {
          id = "task-112";
          name = "skill-document prompt injection";
          kind = "nvim-lua";
        }
        {
          id = "task-113";
          name = "edit-budget accounting evasion";
          kind = "rust";
        }
        {
          id = "task-114";
          name = "slow-update integrity";
          kind = "rust";
        }
        {
          id = "task-115";
          name = "skill-export approval render";
          kind = "nvim-lua";
        }
      ];

      # Segment membership IS the implemented/pending signal: a task with
      # a driver lives in `rosterImplemented`, a designed-but-driverless
      # task in `rosterPending`. `tasks01to60` was missing from the
      # 70-task flake (dangling reference); it is the concatenation of
      # the first four implemented segments.
      tasks01to60 = tasks01to45 ++ tasks46to50 ++ tasks51to55 ++ tasks56to60;
      tasks01to65 = tasks01to60 ++ tasks61to65;
      tasks01to70 = tasks01to65 ++ tasks66to70;
      tasks01to75 = tasks01to70 ++ tasks71to75;
      tasks01to80 = tasks01to75 ++ tasks76to80;
      tasks01to85 = tasks01to80 ++ tasks81to85;
      tasks01to90 = tasks01to85 ++ tasks86to90;
      tasks01to95 = tasks01to90 ++ tasks91to95;
      tasks01to100 = tasks01to95 ++ tasks96to100;
      tasks01to105 = tasks01to100 ++ tasks101to105;
      tasks116to125 = tasks116to120 ++ tasks121to125;
      tasks116to130 = tasks116to125 ++ tasks126to130;
      rosterImplemented =
        tasks01to105
        ++ tasks106to110
        ++ tasks111to115
        ++ tasks116to130
        ++ tasks131to135
        ++ tasks136to140
        ++ tasks141to145
        ++ tasks146to150
        ++ tasks151to158;
      rosterPending = tasks151to200 ++ tasks201to250 ++ tasks251to280;

      # Full declared roster: segments concatenated, nothing hardcoded.
      gauntletRoster = rosterImplemented ++ rosterPending;

      # Roster invariant: the concatenated table really is the 200 declared
      # entries, each well-shaped, ids unique and exactly task-01..task-200.
      # Laziness note: these asserts fire when a per-system output that
      # references `implementedTasks` is evaluated, not at flake load.
      # `checks.task-list` forces that evaluation under `nix flake check`.
      validatedRoster =
        let
          ids = map (t: t.id) gauntletRoster;
          sortedIds = builtins.sort builtins.lessThan ids;
          # `padId` pads single-digit ids only, so lexicographic sort order
          # (e.g. task-100 before task-11) is NOT numeric order once ids
          # pass two digits. Sort the generated expectation with the same
          # comparator as the roster ids: the assert then checks the id
          # SET (exactly task-01..task-<taskCountMax>), not the order.
          padId = n: "task-" + (if n < 9 then "0" else "") + toString (n + 1);
          expectedIds = builtins.sort builtins.lessThan (lib.genList padId taskCountMax);
        in
        assert builtins.length gauntletRoster == taskCountMax;
        assert builtins.length (lib.unique ids) == taskCountMax;
        assert sortedIds == expectedIds;
        assert lib.all (t: t ? id && t ? name && t ? kind) gauntletRoster;
        assert lib.all (
          t:
          builtins.elem t.kind [
            "nvim-lua"
            "rust"
          ]
        ) gauntletRoster;
        gauntletRoster;

      # Implemented tasks only: the validated roster filtered to entries
      # whose drivers have landed. Apps, the runner, and checks.task-list
      # consume this — never the full roster, never the pending segment.
      implementedTaskIds = map (t: t.id) rosterImplemented;
      implementedTasks = builtins.filter (t: builtins.elem t.id implementedTaskIds) validatedRoster;
    in
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs { inherit system; };
        inherit (pkgs) writeShellApplication;

        # Wrap a writeShellApplication derivation as a flake app.
        # Contract: drv exposes one bin/<name> entry point; rejected inputs:
        # anything that is not a derivation.
        toApp = drv: {
          type = "app";
          program = lib.getExe drv;
        };

        # Build ONE per-task app from its table entry.
        # Contract: task is a validated { id, name, kind } entry.
        # The app resolves the phlow workspace (PHLOW_WORKSPACE env, else
        # the enclosing git checkout, else a clear error), requires
        # DIVER_LUA_DIR, then execs the gauntlet CLI for that task id.
        # Prints one JSON verdict line to stdout; a non-"pass" outcome
        # means the task failed.
        mkTaskApp =
          task:
          writeShellApplication {
            name = "gauntlet-${task.id}";
            runtimeInputs = [
              pkgs.cargo
              pkgs.git
              pkgs.coreutils
            ];
            text = ''
              set -euo pipefail

              # Workspace: env override wins, else the enclosing git repo.
              if [ -n "''${PHLOW_WORKSPACE:-}" ]; then
                workspace="$PHLOW_WORKSPACE"
              elif command -v git >/dev/null 2>&1; then
                workspace="$(git rev-parse --show-toplevel)"
              else
                echo "gauntlet: cannot locate the phlow workspace;" \
                  "set PHLOW_WORKSPACE" >&2
                exit 1
              fi
              if [ ! -f "$workspace/Cargo.toml" ]; then
                echo "gauntlet: $workspace is not a phlow workspace" \
                  "(no Cargo.toml)" >&2
                exit 1
              fi

              # Diver lua/ dir: nvim-lua drivers load Matt's actual config code.
              if [ -z "''${DIVER_LUA_DIR:-}" ]; then
                echo "gauntlet: DIVER_LUA_DIR is not set;" \
                  "point it at diver's lua/ directory" >&2
                exit 1
              fi

              workDir="$(mktemp -d)"
              trap 'rm -rf "$workDir"' EXIT

              cd "$workspace"
              exec cargo run --locked -p phlow-gauntlet --bin gauntlet -- \
                ${
                  lib.escapeShellArgs [
                    "run"
                    task.id
                    "--nvim-bin"
                    "${pkgs.neovim}/bin/nvim"
                  ]
                } \
                --diver-lua "$DIVER_LUA_DIR" \
                --work-dir "$workDir"
            '';
          };

        # One per-task app per IMPLEMENTED task: ONE bounded map over the
        # implemented table. Pending tasks get no app until their driver
        # lands (see ROSTER SCHEME at the top of this file).
        taskApps = builtins.listToAttrs (
          map (task: {
            name = task.id;
            value = mkTaskApp task;
          }) implementedTasks
        );

        # One shell line per implemented task, in roster order:
        # runTask <id> <kind> <bin>.
        runAllLines = lib.concatMapStrings (
          task:
          "  runTask "
          + lib.escapeShellArgs [
            task.id
            task.kind
          ]
          + " "
          + lib.escapeShellArg "${taskApps.${task.id}}/bin/gauntlet-${task.id}"
          + "\n"
        ) implementedTasks;

        # Runs every implemented task app in roster order, collects their
        # JSON verdict lines, writes gauntlet-report.json into
        # $GAUNTLET_REPORT_DIR (default ./report), and prints a pass/fail
        # summary table.
        gauntletRunner = writeShellApplication {
          name = "gauntlet";
          runtimeInputs = [
            pkgs.jq
            pkgs.coreutils
          ];
          text = ''
            set -euo pipefail

            reportDir="''${GAUNTLET_REPORT_DIR:-./report}"
            mkdir -p "$reportDir"

            scratch="$(mktemp -d)"
            trap 'rm -rf "$scratch"' EXIT
            lines="$scratch/verdicts.jsonl"
            : > "$lines"

            passedCount=0
            failedCount=0

            printf '%-9s %-9s %s\n' "TASK" "KIND" "OUTCOME"

            # Run one task app; count pass/fail from its JSON verdict line.
            # A nonzero exit or unparseable output counts as failed.
            runTask() {
              local taskId="$1" taskKind="$2" taskBin="$3"
              local line outcome
              line="$("$taskBin")" || true
              if [ -n "$line" ]; then
                printf '%s\n' "$line" >> "$lines"
              fi
              outcome="$(printf '%s\n' "$line" \
                | jq -r '.outcome // "unknown"' 2>/dev/null)" || outcome="unknown"
              case "$outcome" in
                pass) passedCount=$((passedCount + 1)) ;;
                *) failedCount=$((failedCount + 1)) ;;
              esac
              printf '%-9s %-9s %s\n' "$taskId" "$taskKind" "$outcome"
            }

          ''
          + runAllLines
          + ''
            report="$reportDir/gauntlet-report.json"
            # Keep only parseable JSON lines; jq -s on empty input yields [].
            jq -R 'fromjson? // empty' "$lines" | jq -s '.' > "$report"
            printf '\n%d passed, %d failed\nreport: %s\n' \
              "$passedCount" "$failedCount" "$report"
            if [ "$failedCount" -ne 0 ]; then
              exit 1
            fi
          '';
        };

        # Pretty-prints the latest gauntlet-report.json with jq.
        reportApp = writeShellApplication {
          name = "gauntlet-report";
          runtimeInputs = [ pkgs.jq ];
          text = ''
            set -euo pipefail
            reportDir="''${GAUNTLET_REPORT_DIR:-./report}"
            report="$reportDir/gauntlet-report.json"
            if [ ! -f "$report" ]; then
              echo "gauntlet: no report at $report;" \
                "run the gauntlet app first" >&2
              exit 1
            fi
            exec jq '.' "$report"
          '';
        };
      in
      {
        apps = {
          gauntlet = toApp gauntletRunner;
          report = toApp reportApp;
        }
        // builtins.mapAttrs (_: toApp) taskApps;

        # Sterile dev env. Cargo still needs network on first run for the
        # crates.io registry (documented boundary, not a sandbox escape:
        # the toolchain is pinned, the registry is not vendored).
        devShells.default = pkgs.mkShell {
          packages = [
            pkgs.cargo
            pkgs.rustc
            pkgs.neovim
            pkgs.lua5_4
            pkgs.stylua
            pkgs.git
            pkgs.jq
            pkgs.deadnix
            pkgs.statix
            pkgs.nixfmt-rfc-style
          ];
          shellHook = ''
            echo "phlow gauntlet dev shell (nixpkgs nixos-25.05; flake.lock committed on primo)"
            cargo --version
            rustc --version
            nvim --version | head -n 1 || true
            lua -v
            stylua --version
            git --version
            jq --version
          '';
        };

        checks = {
          # Honest app-presence check, two invariants, one assert each:
          # (1) every implemented task has an app;
          # (2) no app exists for a task that is not implemented.
          # Pending tasks are validated as roster DATA by `validatedRoster`
          # (length, id uniqueness, exact task-01..task-115 sequence, entry
          # shape, kind set) — never as apps. The asserts fire when this
          # check is evaluated; the trivial derivation exists so
          # `nix flake check` exercises them.
          task-list =
            let
              presentTasks = builtins.filter (t: taskApps ? ${t.id}) implementedTasks;
            in
            assert builtins.length presentTasks == builtins.length implementedTasks;
            assert builtins.length (builtins.attrNames taskApps) == builtins.length implementedTasks;
            pkgs.runCommand "gauntlet-task-list" { } ''
              printf '%d of %d declared tasks have apps\n' \
                ${toString (builtins.length implementedTasks)} \
                ${toString taskCountMax} > "$out"
            '';

          # Lint this flake's own Nix sources with hermetic nixpkgs tools.
          # Source is filtered to .nix files only: the working tree is the
          # task source, and generated files must never enter the store.
          nix-lint = pkgs.stdenvNoCC.mkDerivation {
            name = "gauntlet-nix-lint";
            src = lib.fileset.toSource {
              root = ./.;
              fileset = ./flake.nix;
            };
            nativeBuildInputs = [
              pkgs.nixfmt-rfc-style
              pkgs.statix
              pkgs.deadnix
            ];
            buildPhase = ''
              nixfmt --check flake.nix
              statix check .
              deadnix --fail .
            '';
            installPhase = ''
              touch "$out"
            '';
          };
        };
      }
    );
}
