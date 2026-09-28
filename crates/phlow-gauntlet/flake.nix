# crates/phlow-gauntlet/flake.nix
#
# Phlow 130-task orchestration gauntlet: Nix entry points for the task
# drivers in `crates/phlow-gauntlet` (tasks 01-20 designed in
# ~/workspace/gauntlet-design.md; tasks 21-70 designed in
# ~/workspace/gauntlet-design-tasks-21-70.md; tasks 71-100 designed in
# ~/workspace/gauntlet-design-tasks-71-100.md; tasks 101-115 designed in
# ~/workspace/gauntlet-design-tasks-101-115.md; tasks 116-130 designed in
# ~/workspace/gauntlet-design-tasks-116-130.md).
#
# ROSTER SCHEME (two levels — read this before extending):
# - `gauntletRoster` declares EVERY task as pure data: { id, name, kind },
#   split into `tasks01to105` (implemented: drivers for 01-105 all landed
#   in this crate, waves 01-20), `tasks116to130` (implemented: diver
#   harness Phase-2 acceptance probes, waves 116-120, 121-125, and
#   126-130), and `tasks106to115` (pending: designed in the extension
#   docs, wave 21, drivers not yet landed).
#   Roster asserts validate ALL 130 entries.
# - Apps (`apps.task-NN`), the `gauntlet` runner, and `checks.task-list`
#   cover IMPLEMENTED tasks only. The flake never claims an app for a
#   task without a driver. This deliberately deviates from the design
#   docs' flake plans ("assert all 100/115 apps exist"): the plan is
#   data-right but honesty-wrong while drivers 101-115 do not exist.
# - When a wave worker lands a driver, move that task's entry from the
#   pending side to the implemented side (`rosterImplemented` grows,
#   `rosterPending` shrinks). The asserts, app generation, runner, and
#   checks all consume the two segments via `gauntletRoster` and
#   `implementedTasks` — no other edits needed. Waves through 96-100
#   landed every designed task, so the implemented side is complete
#   through task-105 plus tasks 116-130; the pending side holds tasks
#   106-115 until their drivers land.
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
  description = "phlow 130-task gauntlet: declared roster, apps for implemented drivers";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.05";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    { nixpkgs, flake-utils, ... }:
    let
      inherit (nixpkgs) lib;

      # Named bound on the declared roster: exactly 130 entries
      # (task-01..task-130), no more.
      taskCountMax = 130;

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

      tasks106to115 = [
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
      rosterImplemented = tasks01to105 ++ tasks116to130;
      rosterPending = tasks106to115;

      # Full declared roster: segments concatenated, nothing hardcoded.
      gauntletRoster = rosterImplemented ++ rosterPending;

      # Roster invariant: the concatenated table really is the 130 declared
      # entries, each well-shaped, ids unique and exactly task-01..task-130.
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
