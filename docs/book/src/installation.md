# User Guide: Installation

Imagine phlow is a workshop you assemble yourself: the pieces arrive as
source code, and you bolt them into one binary called `phlow`. There is
no installer wizard, no global package — just the Rust toolchain, an
Ollama server nearby, and one `cargo` command.

## What you need

- A POSIX operating system. The README names Linux and macOS as the
  primary targets, because the secure local file I/O needs POSIX.
- The pinned Rust toolchain. The repo root carries
  `rust-toolchain.toml`: channel `nightly-2026-09-25`, profile
  `minimal`, plus the `rustfmt` and `clippy` components. If you have
  `rustup` installed it picks this up automatically when you enter the
  checkout — nothing to select by hand.
- Ollama with an installed, tool-capable model. The default model is
  `qwen2.5-coder:7b` (this is the default value of `ollama.model` in
  `phlow-config`).

## Option A: the Nix dev shell

The repo ships a `flake.nix` whose default dev shell
(`phlow-dev`) includes cargo, rust-analyzer, the Ollama package, ruff,
and the usual dev tools:

```sh
nix develop
```

Note honestly: the flake's *packaged* app and its shell hook still
describe the old Python `flow` (the port is in progress); the shell
itself is the useful part for Rust work — cargo and rust-analyzer are
there.

## Option B: plain cargo

From the repo root, build the CLI crate. The binary `phlow` is the
crate `phlow-cli`'s single bin target (`src/bin/phlow.rs`):

```sh
cargo build --release -p phlow-cli
```

The binary lands at `target/release/phlow`. Put it on your `PATH` or
invoke it by path.

## Start Ollama

In a separate terminal, if it is not already running:

```sh
ollama serve
ollama pull qwen2.5-coder:7b
```

## Prove it works

This runs no model call and no command execution, so it works even with
Ollama down:

```sh
phlow status --workspace /absolute/path/to/project
```

`phlow status` reports local capabilities only. If that prints cleanly,
the workshop is open. Next: [Configuration](configuration.md) —
phlow does nothing useful until you give it a config that names a
workspace and (for real runs) a model backend.

## What is deliberately NOT here

The README's "Get started" section still shows `uv venv` /`uv pip install` instructions for the Python `flow`; that is the
pre-port quickstart. The authoritative checkout on `main` is the Rust
port built with cargo. Do not mix the two: phlow does not install
project dependencies for you, and neither should your shell.
