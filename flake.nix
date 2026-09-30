{
  description = "Phlow — Local AI Workflow System";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    # Always-fresh codebase map for agents, from the repomap subflake in
    # qompassai/nix (only the repomap/ dir is evaluated, not the NixOS config).
    repomap.url = "github:qompassai/nix?dir=repomap";
    # Pinned Rust toolchain matching rust-toolchain.toml (nightly-2026-09-25).
    # Used ONLY by the flake apps below so `nix run .#<app>` is hermetic;
    # the devShell is untouched. fenix follows the same nixpkgs as everything
    # else, and the input is pinned in flake.lock like all others.
    fenix.url = "github:nix-community/fenix";
    fenix.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = { self, nixpkgs, flake-utils, repomap, fenix }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        # python3.12: the pinned nixpkgs' sphinx 9.1.0 (pulled in via
        # httpx's test closure) no longer supports 3.11.
        python = pkgs.python312;

        pythonEnv = python.withPackages (ps: with ps; [
          httpx
          rich
          ps."prompt-toolkit"
          # Dev tools
          pytest
          ruff
        ]);

        flowPkg = python.pkgs.buildPythonPackage {
          pname = "phlow";
          version = "0.1.0";
          src = ./.;
          format = "pyproject";
          nativeBuildInputs = [ python.pkgs.hatchling ];
          propagatedBuildInputs = with python.pkgs; [
            httpx
            rich
            python.pkgs."prompt-toolkit"
          ];
        };

        # Pinned Rust toolchain matching rust-toolchain.toml
        # (nightly-2026-09-25), for the flake apps only. The devShell below
        # is untouched.
        rustToolchain = fenix.packages.${system}.fromToolchainFile {
          file = ./rust-toolchain.toml;
          # Hash of the date-pinned channel manifest
          # https://static.rust-lang.org/dist/2026-09-25/channel-rust-nightly.toml
          # (immutable: the date is part of the URL).
          sha256 = "sha256-3ok3kaNc8lhDSnZ9bZAJwmHgUrdB9CC5m+ZJBwFPWnA=";
        };

        appCommonLib = builtins.readFile ./nix/lib/common.sh;

        # Build one hermetic flake app. `name` is the binary name, `desc`
        # a one-line description, `script` the app body
        # (./nix/apps/<name>.sh); the common library is prepended and
        # PHLOW_APP_NAME baked in. writeShellApplication runs shellcheck
        # over the result, so the scripts under ./nix/apps must stay
        # shellcheck-clean.
        mkApp = name: desc: runtimeInputs: script:
          let
            prog = pkgs.writeShellApplication {
              inherit name runtimeInputs;
              text = ''
                # Baked in by flake.nix: identifies the app for scratch/reports.
                PHLOW_APP_NAME="${name}"
              '' + appCommonLib + builtins.readFile script;
            };
          in {
            type = "app";
            program = "${prog}/bin/${name}";
            meta.description = desc;
          };

      in {
        packages = {
          default = flowPkg;
          phlow = flowPkg;
        };

        apps.default = {
          type = "app";
          program = "${flowPkg}/bin/phlow";
        };

        # --- Deterministic validation / release apps -----------------------
        # `nix run .#<app>` runs repo workflows hermetically: every tool
        # comes from pinned flake inputs (see flake.lock) — no paru/curl
        # installed tools at runtime. Each app:
        #   * fails fast naming any missing dependency,
        #   * builds in an isolated cargo target dir inside a temp scratch
        #     dir, removed on EXIT whether the run passes or fails,
        #   * writes a markdown report to reports/<app>-<UTC ts>.md,
        #   * proves the tree is otherwise untouched (`git status` check).
        # Docs: docs/FLAKE.md.
        #
        # HUMAN-GATED BOUNDARY: scripts/publish/tbr-* (real crates.io
        # publish, manual release steps) and any keystore/key generation
        # are deliberately NOT flake apps. `nix run .#release` only ever
        # creates GitHub Releases (never crates.io, Play, or F-Droid), and
        # only when invoked explicitly with a version.
        apps.gates =
          mkApp "phlow-gates" "Publish gates: build, clippy (zero warnings), fmt, test — hermetic, with report."
            [ rustToolchain pkgs.git ]
            ./nix/apps/gates.sh;
        apps.security =
          mkApp "phlow-security" "Security audit: cargo audit + cargo geiger (cargo deny skipped: no deny.toml)."
            [ rustToolchain pkgs.cargo-deny pkgs.cargo-audit pkgs.cargo-geiger ]
            ./nix/apps/security.sh;
        apps.publish-dryrun =
          mkApp "phlow-publish-dryrun" "Version audit + topo order + cargo publish --dry-run per crate."
            [ rustToolchain pkgs.git pkgs.python3 ]
            ./nix/apps/publish-dryrun.sh;
        apps.version-audit =
          mkApp "phlow-version-audit" "Fast read-only workspace version-consistency check."
            [ pkgs.git pkgs.python3 ]
            ./nix/apps/version-audit.sh;
        apps.gauntlet =
          mkApp "phlow-gauntlet" "Build the gauntlet binary and drive it (default: list smoke test)."
            [ rustToolchain pkgs.git pkgs.neovim ]
            ./nix/apps/gauntlet.sh;
        apps.debug-smoke =
          mkApp "phlow-debug-smoke" "DAP debugger smoke: breakpoint on phlow_json::parse_limited must verify AND hit."
            # NOTE: no pkgs.lldb here on purpose. nixpkgs' lldb 21.1.8 ships a
            # broken lldb-dap: every launch fails with "invalid debugger"
            # (verified 2026-09-30, inside and outside the devShell). The
            # devShell itself does not pin lldb either, so the app uses the
            # ambient lldb-dap from the user's PATH (nix run preserves it) and
            # fails honestly if none is present.
            [ rustToolchain pkgs.python3 ]
            ./nix/apps/debug-smoke.sh;
        apps.release =
          mkApp "phlow-release" "Cut a GitHub release: build tarball, self-populating notes, gh release create. Needs <version>."
            [ rustToolchain pkgs.git pkgs.gh pkgs.git-cliff pkgs.python3 ]
            ./nix/apps/release.sh;

        devShells.default = pkgs.mkShell {
          name = "phlow-dev";
          buildInputs = [
            pythonEnv
            pkgs.ollama
            pkgs.ruff
            pkgs.shellcheck
            pkgs.shfmt
            pkgs.lua-language-server
            pkgs.stylua
            pkgs.rust-analyzer
            pkgs.cargo
            pkgs.go
            pkgs.gopls
            pkgs.nodejs
            pkgs.clang-tools
            pkgs.git
            repomap.packages.${system}.repomap
          ];

          # Keep $PWD/.repomap.txt fresh for agents: regenerates only when a
          # .rs file is newer than it, so entering the shell stays cheap.
          # The map is a derived artifact (gitignored).
          shellHook = ''
            echo "Phlow dev environment loaded"
            echo "Python: $(python --version)"
            echo "Run: uv venv .venv && uv pip install --python .venv/bin/python -e '.[editor,dev]'"
          '' + repomap.lib.refreshHook {
            pkg = repomap.packages.${system}.repomap;
          };
        };
      }
    );
}
