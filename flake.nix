{
  description = "Phlow — Local AI Workflow System";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    # Always-fresh codebase map for agents, from the repomap subflake in
    # qompassai/nix (only the repomap/ dir is evaluated, not the NixOS config).
    repomap.url = "github:qompassai/nix?dir=repomap";
  };

  outputs = { self, nixpkgs, flake-utils, repomap }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        python = pkgs.python311;

        pythonEnv = python.withPackages (ps: with ps; [
          httpx
          rich
          prompt_toolkit
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
            prompt_toolkit
          ];
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
