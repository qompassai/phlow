# Phlow — reproducible Rust builds.
#
# `nix build` compiles the Cargo workspace with the exact pinned nightly
# from rust-toolchain.toml (via fenix) inside the Nix sandbox: no network
# at build time, all inputs content-addressed. The resulting binaries are
# bit-reproducible, so their hashes can serve as the expected pins for
# phlow-checks' SHA-256 binary pinning (crates/phlow-checks/src/pin.rs).
#
# - `nix build`          -> full workspace (release)
# - `nix build .#phlow`  -> just the `phlow` CLI binary
# - `nix run`            -> run the `phlow` CLI
# - `nix flake check`    -> fmt + clippy gates
# - `nix develop`        -> dev shell with the pinned toolchain
#                         (also refreshes $PWD/.repomap.txt, the
#                         always-fresh codebase map for agents)
#
# Cargo remains the fast dev loop (`cargo build` on the host). This flake
# is for reproducible release builds and build provenance, not iteration.
{
  description = "Phlow — reproducible Rust builds (crane + pinned nightly)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    crane.url = "github:ipetkov/crane";
    # Pinned Rust toolchain matching rust-toolchain.toml (nightly-2026-09-25).
    # Serves the crane builds, the devShell, and the flake apps below so
    # `nix run .#<app>` is hermetic; pinned in flake.lock like all inputs.
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    # Always-fresh codebase map for agents, from the repomap subflake in
    # qompassai/nix (only the repomap/ dir is evaluated, not the NixOS config).
    repomap.url = "github:qompassai/nix?dir=repomap";
  };

  outputs = { self, nixpkgs, flake-utils, crane, fenix, repomap }:
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

        # Exact toolchain from rust-toolchain.toml. The sha256 is the hash of
        # the nightly channel manifest for the pinned date (not of the
        # toolchain file itself). If rust-toolchain.toml bumps the nightly
        # date, run `nix build` once with a dummy hash and paste the
        # "got:" hash Nix reports.
        toolchain = fenix.packages.${system}.fromToolchainFile {
          file = ./rust-toolchain.toml;
          sha256 = "sha256-3ok3kaNc8lhDSnZ9bZAJwmHgUrdB9CC5m+ZJBwFPWnA=";
        };

        # The flake apps below were written against this name for the same
        # pinned toolchain.
        rustToolchain = toolchain;

        craneLib = (crane.mkLib pkgs).overrideToolchain toolchain;

        # Linker matching the dev environment (clang + lld).
        rustLinkFlags = "-C linker=clang -C link-arg=-fuse-ld=lld";

        # cleanCargoSource strips everything but Cargo inputs, but several
        # crates vendor assets via include_str!/include_bytes!:
        #   crates/phlow-codegen/profiles/*.toml (8 codegen profiles)
        #   crates/phlow-runtime/skills/*.md (vendored system prompt)
        #   CUDA *.ptx, test fixtures (*.json/*.jsonl), *.desc
        # Keep those asset types alongside the Cargo sources.
        crateSrc = pkgs.lib.cleanSourceWith {
          src = craneLib.path ./.;
          filter = path: type:
            (craneLib.filterCargoSources path type)
            || (type == "regular" && pkgs.lib.any
              (ext: pkgs.lib.hasSuffix ext (builtins.toString path))
              [ ".toml" ".md" ".json" ".jsonl" ".ptx" ".desc" ".lua" ]);
        };

        commonArgs = {
          src = crateSrc;
          # The sandbox cannot run the integration tests: no /bin/sh,
          # no nvim, no diver checkout. Tests run on the host via cargo;
          # `nix build` compiles only.
          doCheck = false;
          strictDeps = true;
          nativeBuildInputs = with pkgs; [
            clang
            lld
            perl # ring
            pkg-config
          ];
          buildInputs = with pkgs; [
            openssl
            sqlite
          ];
          RUSTFLAGS = rustLinkFlags;
        };

        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        phlow-workspace = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;
        });

        phlow-bin = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;
          pname = "phlow";
          cargoExtraArgs = "-p phlow-cli";
        });

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
          default = phlow-workspace;
          phlow = phlow-bin;
        };

        apps.default = {
          type = "app";
          program = "${phlow-bin}/bin/phlow";
        };

        checks = {
          clippy = craneLib.cargoClippy (commonArgs // {
            inherit cargoArtifacts;
            cargoClippyExtraArgs = "--all-targets -- --deny warnings";
          });
          fmt = craneLib.cargoFmt {
            src = craneLib.cleanCargoSource ./.;
          };
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
          inputsFrom = [ phlow-workspace ];
          buildInputs = [
            toolchain
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

        # Nix formatter matching the qompassai/nix convention (alejandra).
        formatter = pkgs.alejandra;
      });
}
