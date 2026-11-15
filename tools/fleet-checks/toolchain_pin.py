#!/usr/bin/env python3
"""Fleet check: pinned-toolchain gate.

Contract: the workspace's recorded Rust toolchain pin resolves, on
this machine, to exactly the pinned toolchain.

The pin is read from `rust-toolchain.toml` (`[toolchain] channel`) or
the legacy `rust-toolchain` file. The check proves three things:
1. the pinned channel names an installed rustup toolchain;
2. the pinned toolchain's own rustc (via `rustup which`) runs;
3. the workspace-resolved `rustc --version` — through the cargo-home
   rustup proxy, which honours the pin file — is byte-identical to
   the pinned toolchain's version string: the pin is what actually
   resolves in the workspace.

All rustc/rustup invocations use the cargo-home binaries, never
PATH's: on primo, PATH's rustc is a system shim that ignores
toolchain selection entirely (measured 2026-11-14: it answered
`rustc +nightly-2026-10-30 --version` with the system version and
exit 0), so a PATH-first check can be fooled into passing on the
wrong toolchain. Automatic installs are disabled throughout
(RUSTUP_AUTO_INSTALL=0): the pin must already be installed, never
fetched as a side effect of a check.

Exit codes: 0 = pin resolves as recorded, 1 = pin does not resolve
(uninstalled channel, or the workspace resolves elsewhere),
2 = no readable pin / rustup missing / usage error.
State changes: none.
"""

import argparse
import os
import shutil
import subprocess
import sys
import tomllib

RUN_TIMEOUT_S = 30.0
OUTPUT_BYTES_MAX = 64 * 1024
PIN_NAME_MAX = 128


class CheckError(Exception):
    """No readable pin or no rustup: the run proves nothing."""


def find_tool(name):
    """Resolve a cargo-home tool by absolute path.

    `$CARGO_HOME/bin` (default `~/.cargo/bin`) wins over PATH: PATH on
    primo carries a system rustc shim that ignores toolchain
    selection, while the cargo-home entry is the rustup proxy whose
    resolution semantics this gate exists to verify. PATH is the
    fallback for machines without a cargo home. Raises CheckError
    when the tool exists in neither place.
    """
    cargo_home = os.environ.get("CARGO_HOME") or os.path.join(
        os.path.expanduser("~"), ".cargo")
    candidate = os.path.join(cargo_home, "bin", name)
    if os.path.isfile(candidate) and os.access(candidate, os.X_OK):
        return candidate
    on_path = shutil.which(name)
    if on_path is not None:
        return on_path
    raise CheckError(f"{name} not found in {cargo_home}/bin or on PATH")


def run_bounded(argv, cwd, env_extra):
    """Run one command with argv form, a deadline, and bounded output.

    Returns (returncode, stdout text). Raises CheckError on launch
    failure or timeout; the child is killed and reaped on timeout by
    subprocess.run's own cleanup.
    """
    env = dict(os.environ)
    env.update(env_extra)
    try:
        done = subprocess.run(
            argv, cwd=cwd, env=env, timeout=RUN_TIMEOUT_S,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False,
        )
    except OSError as error:
        raise CheckError(f"cannot run {argv[0]}: {error}") from error
    except subprocess.TimeoutExpired as error:
        raise CheckError(f"{argv[0]} exceeded {RUN_TIMEOUT_S}s") from error
    stdout = done.stdout[:OUTPUT_BYTES_MAX].decode("utf-8", "replace")
    return done.returncode, stdout


def read_pin(workspace):
    """The recorded channel string from the workspace's pin file."""
    toml_path = os.path.join(workspace, "rust-toolchain.toml")
    legacy_path = os.path.join(workspace, "rust-toolchain")
    if os.path.isfile(toml_path):
        try:
            with open(toml_path, "rb") as handle:
                parsed = tomllib.load(handle)
        except (OSError, tomllib.TOMLDecodeError) as error:
            raise CheckError(f"unreadable {toml_path}: {error}") from error
        channel = parsed.get("toolchain", {}).get("channel")
        if not isinstance(channel, str) or not channel:
            raise CheckError(f"{toml_path} records no [toolchain] channel")
        return channel
    if os.path.isfile(legacy_path):
        try:
            with open(legacy_path, encoding="utf-8") as handle:
                lines = [line.strip() for line in handle if line.strip()]
        except OSError as error:
            raise CheckError(f"unreadable {legacy_path}: {error}") from error
        if not lines:
            raise CheckError(f"{legacy_path} is empty")
        return lines[0]
    raise CheckError(f"no toolchain pin in {workspace}")


def pinned_rustc(rustup, channel, env_extra):
    """The pinned toolchain's own rustc path, per `rustup which`.

    Returns None when the channel names no installed toolchain.
    Asking rustup itself — instead of parsing `rustup toolchain list`
    output, whose display names carry variable target triples — is
    the semantics the gate means.
    """
    rc, stdout = run_bounded(
        [rustup, "which", "--toolchain", channel, "rustc"], None, env_extra)
    if rc != 0:
        return None
    lines = [line.strip() for line in stdout.splitlines() if line.strip()]
    return lines[0] if lines else None


def run(argv):
    """Execute the check; returns the process exit code."""
    parser = argparse.ArgumentParser(prog="toolchain_pin")
    parser.add_argument("--workspace", required=True)
    args = parser.parse_args(argv)
    workspace = os.path.realpath(args.workspace)
    if not os.path.isdir(workspace):
        raise CheckError(f"workspace is not a directory: {args.workspace}")
    channel = read_pin(workspace)
    if len(channel) > PIN_NAME_MAX:
        raise CheckError("recorded channel name is implausibly long")
    no_install = {"RUSTUP_AUTO_INSTALL": "0"}
    rustup = find_tool("rustup")
    pinned_binary = pinned_rustc(rustup, channel, no_install)
    if pinned_binary is None:
        print(f"toolchain_pin: FAIL channel={channel} is not installed")
        return 1
    rc, pinned = run_bounded([pinned_binary, "--version"], workspace,
                             no_install)
    if rc != 0:
        print(f"toolchain_pin: FAIL the pinned toolchain's rustc "
              f"does not run: {pinned_binary}")
        return 1
    rustc = find_tool("rustc")
    rc, resolved = run_bounded([rustc, "--version"], workspace, no_install)
    if rc != 0:
        print("toolchain_pin: FAIL rustc does not resolve in the workspace")
        return 1
    if not resolved.strip() or resolved.strip() != pinned.strip():
        print(f"toolchain_pin: FAIL workspace resolves to "
              f"{resolved.strip()!r}, pin is {pinned.strip()!r}")
        return 1
    print(f"toolchain_pin: PASS channel={channel} "
          f"version={resolved.strip()!r}")
    return 0


def main():
    try:
        return run(sys.argv[1:])
    except CheckError as error:
        print(f"toolchain_pin: UNPROVEN {error}")
        return 2


if __name__ == "__main__":
    sys.exit(main())
