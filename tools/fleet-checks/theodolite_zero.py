#!/usr/bin/env python3
"""Fleet check: theodolite-zero.

Contract: for each target file, the theodolite bridge's diagnostics
owner for the file's language reports zero error-severity diagnostics.

Usage:
    theodolite_zero.py --workspace DIR [--deadline-s SECONDS] FILE...

FILE paths are workspace-relative (absolute paths must stay inside the
workspace). Routes (extension -> bridge server id) mirror
~/.local/share/theodolite/bridge.toml: rs -> bacon-ls, toml -> taplo,
lua -> lua-language-server, py -> jedi-language-server, ron -> ron-ls.

Verdict discipline (estate theodolite doctrine):
- A zero is only believed from a proven session. For every non-Rust
  route in the run, the script first plants a control file with a
  known syntax error inside the workspace and requires the bridge to
  report it; a session that cannot see the control proves nothing and
  the check exits 2 rather than passing on an unproven empty.
- The Rust route's diagnostics come from the producer's
  `.bacon-locations` (bacon-ls with the Bacon backend). The script
  never starts a producer; it fails closed (exit 2) unless the file
  exists and is at least as fresh as every Rust target.

Exit codes: 0 = every target clean, 1 = error diagnostics found,
2 = unproven / precondition unmet / usage error. State changes: the
control directory `.phlow-fleet-control/` is created inside the
workspace and always removed again; nothing else is written.
"""

import argparse
import json
import os
import select
import shutil
import subprocess
import sys
import time

DEFAULT_BIN = "/home/phaedrus/.local/bin/theodolite"
DEFAULT_CONFIG = "/home/phaedrus/.local/share/theodolite/bridge.toml"
PROTOCOL_VERSION = "2026-07-28"
CONTROL_DIR_NAME = ".phlow-fleet-control"
LOCATIONS_NAME = ".bacon-locations"

DEADLINE_S_DEFAULT = 280.0
DEADLINE_S_MAX = 3600.0
RESPONSE_BYTES_MAX = 16 * 1024 * 1024
TARGETS_MAX = 64
DIAGNOSTICS_PRINTED_MAX = 20
ERROR_SEVERITY = 1

# extension -> (bridge server id, control file name, control body).
# A control body of None marks the producer-precondition route (Rust):
# no in-band control exists for it, by design (see module docstring).
ROUTES = {
    "rs": ("bacon-ls", None, None),
    "toml": ("taplo", "control.toml", "[phlow_control]\nmembers = [\n"),
    "lua": ("lua-language-server", "control.lua", "if true then\n"),
    "py": ("jedi-language-server", "control.py", "def phlow_control(:\n"),
    "ron": ("ron-ls", "control.ron", "(\n"),
}


class CheckError(Exception):
    """Precondition, protocol, or usage failure: the run proves nothing."""


def parse_args(argv):
    """Parse and validate argv; raises CheckError on bad input."""
    parser = argparse.ArgumentParser(prog="theodolite_zero")
    parser.add_argument("--workspace", required=True)
    parser.add_argument("--deadline-s", type=float, default=DEADLINE_S_DEFAULT)
    parser.add_argument("files", nargs="+")
    args = parser.parse_args(argv)
    if not 0.0 < args.deadline_s <= DEADLINE_S_MAX:
        raise CheckError(f"--deadline-s out of range: {args.deadline_s}")
    if len(args.files) > TARGETS_MAX:
        raise CheckError(f"too many targets: {len(args.files)} > {TARGETS_MAX}")
    return args


def resolve_targets(workspace, files):
    """Map file arguments to (absolute path, server id); all must exist,
    stay inside the workspace, and have a known route."""
    targets = []
    for name in files:
        path = name if os.path.isabs(name) else os.path.join(workspace, name)
        path = os.path.realpath(path)
        if not path.startswith(workspace + os.sep):
            raise CheckError(f"target escapes the workspace: {name}")
        if not os.path.isfile(path):
            raise CheckError(f"target is not a file: {name}")
        extension = os.path.splitext(path)[1].lstrip(".").lower()
        route = ROUTES.get(extension)
        if route is None:
            raise CheckError(f"no theodolite route for extension: {name}")
        targets.append((path, route[0]))
    return targets


def require_rust_precondition(workspace, targets):
    """The Rust route reads the producer's locations file; it must exist
    and be at least as fresh as every Rust target (producer-first rule)."""
    rust_targets = [path for path, server in targets if server == "bacon-ls"]
    if not rust_targets:
        return
    locations = os.path.join(workspace, LOCATIONS_NAME)
    if not os.path.isfile(locations):
        raise CheckError(
            f"producer precondition unmet: {locations} is missing; "
            "run the bacon-ls producer before this check"
        )
    locations_mtime = os.path.getmtime(locations)
    for path in rust_targets:
        if locations_mtime < os.path.getmtime(path):
            raise CheckError(
                f"producer precondition unmet: {locations} is older than "
                f"{path}; refresh the producer before this check"
            )


class Bridge:
    """One theodolite stdio session, owned end to end by this process.

    Every read is bounded by the run's shared deadline and a byte cap;
    the process is terminated and reaped on close, on every path.
    """

    def __init__(self, binary, config, deadline_monotonic):
        self._deadline = deadline_monotonic
        self._buffer = b""
        try:
            self._proc = subprocess.Popen(
                [binary, "--config", config],
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.DEVNULL,
            )
        except OSError as error:
            raise CheckError(f"cannot start theodolite: {error}") from error

    def close(self):
        proc = getattr(self, "_proc", None)
        if proc is None:
            return
        self._proc = None
        try:
            proc.kill()
        except OSError:
            pass
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            pass

    def __enter__(self):
        return self

    def __exit__(self, exc_type, exc, tb):
        self.close()
        return False

    def _read_line(self):
        while b"\n" not in self._buffer:
            remaining = self._deadline - time.monotonic()
            if remaining <= 0:
                raise CheckError("deadline exceeded waiting for the bridge")
            ready, _, _ = select.select([self._proc.stdout], [], [], remaining)
            if not ready:
                raise CheckError("deadline exceeded waiting for the bridge")
            chunk = os.read(self._proc.stdout.fileno(), 65536)
            if not chunk:
                raise CheckError("bridge closed stdout")
            self._buffer += chunk
            if len(self._buffer) > RESPONSE_BYTES_MAX:
                raise CheckError("bridge response exceeds the byte bound")
        line, self._buffer = self._buffer.split(b"\n", 1)
        return line

    def request(self, request_id, method, params):
        """Send one JSON-RPC request and return its decoded response."""
        message = {"jsonrpc": "2.0", "id": request_id, "method": method,
                   "params": params}
        try:
            self._proc.stdin.write((json.dumps(message) + "\n").encode())
            self._proc.stdin.flush()
        except OSError as error:
            raise CheckError(f"cannot write to the bridge: {error}") from error
        try:
            response = json.loads(self._read_line())
        except json.JSONDecodeError as error:
            raise CheckError(f"bridge returned non-JSON: {error}") from error
        if "error" in response:
            raise CheckError(f"bridge error for {method}: {response['error']}")
        return response

    def notify(self, method):
        message = {"jsonrpc": "2.0", "method": method}
        try:
            self._proc.stdin.write((json.dumps(message) + "\n").encode())
            self._proc.stdin.flush()
        except OSError as error:
            raise CheckError(f"cannot write to the bridge: {error}") from error

    def diagnostics(self, request_id, workspace, server, path):
        """One diagnostics call, decoded to the bridge's payload object."""
        response = self.request(request_id, "tools/call", {
            "name": "diagnostics",
            "arguments": {"workspace": workspace, "server": server,
                          "path": path},
        })
        try:
            text = response["result"]["content"][0]["text"]
            payload = json.loads(text)
            diagnostics = payload["diagnostics"]
        except (KeyError, IndexError, TypeError, json.JSONDecodeError) as error:
            raise CheckError(
                f"malformed diagnostics payload for {path}: {error}"
            ) from error
        if not isinstance(diagnostics, list):
            raise CheckError(f"malformed diagnostics payload for {path}")
        return payload


def error_diagnostics(payload):
    """The payload's error-severity diagnostics (LSP severity 1)."""
    return [d for d in payload["diagnostics"]
            if isinstance(d, dict) and d.get("severity") == ERROR_SEVERITY]


def run_controls(bridge, workspace, targets, request_id):
    """Plant one syntax-error control per non-Rust route in use and
    require the bridge to see it. Returns the next request id. The
    control directory is removed by the caller, on every path."""
    routes = []
    for _path, server in targets:
        route = next(r for r in ROUTES.values() if r[0] == server)
        if route[1] is not None and server not in routes:
            routes.append(server)
    if not routes:
        return request_id
    control_dir = os.path.join(workspace, CONTROL_DIR_NAME)
    os.makedirs(control_dir, exist_ok=True)
    for server in routes:
        _sid, filename, body = next(r for r in ROUTES.values() if r[0] == server)
        control_path = os.path.join(control_dir, filename)
        with open(control_path, "w", encoding="utf-8") as handle:
            handle.write(body)
        payload = bridge.diagnostics(request_id, workspace, server, control_path)
        request_id += 1
        if not error_diagnostics(payload):
            raise CheckError(
                f"session unproven: the {server} control was not reported"
            )
        print(f"theodolite_zero: control ok for {server}")
    return request_id


def run(argv):
    """Execute the check; returns the process exit code."""
    args = parse_args(argv)
    workspace = os.path.realpath(args.workspace)
    if not os.path.isdir(workspace):
        raise CheckError(f"workspace is not a directory: {args.workspace}")
    targets = resolve_targets(workspace, args.files)
    require_rust_precondition(workspace, targets)
    binary = os.environ.get("FLEET_THEODOLITE_BIN", DEFAULT_BIN)
    config = os.environ.get("FLEET_THEODOLITE_CONFIG", DEFAULT_CONFIG)
    if not os.path.isfile(binary) or not os.path.isfile(config):
        raise CheckError("theodolite binary or bridge config is missing")
    deadline = time.monotonic() + args.deadline_s
    control_dir = os.path.join(workspace, CONTROL_DIR_NAME)
    failures = 0
    try:
        with Bridge(binary, config, deadline) as bridge:
            bridge.request(1, "initialize", {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": "phlow-fleet-check", "version": "1"},
            })
            bridge.notify("notifications/initialized")
            request_id = run_controls(bridge, workspace, targets, 2)
            for path, server in targets:
                payload = bridge.diagnostics(request_id, workspace, server, path)
                request_id += 1
                errors = error_diagnostics(payload)
                relative = os.path.relpath(path, workspace)
                if errors:
                    failures += 1
                    print(f"theodolite_zero: FAIL {relative} "
                          f"errors={len(errors)} server={server}")
                    for item in errors[:DIAGNOSTICS_PRINTED_MAX]:
                        print(f"  {item.get('message', '')}")
                else:
                    print(f"theodolite_zero: PASS {relative} server={server}")
    finally:
        shutil.rmtree(control_dir, ignore_errors=True)
    return 1 if failures else 0


def main():
    try:
        return run(sys.argv[1:])
    except CheckError as error:
        print(f"theodolite_zero: UNPROVEN {error}")
        return 2


if __name__ == "__main__":
    sys.exit(main())
