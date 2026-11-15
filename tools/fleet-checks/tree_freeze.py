#!/usr/bin/env python3
"""Fleet check: hash-twice freeze.

Contract: the workspace's source tree is byte-identical to its
recorded freeze, and stable while it is being read.

The tree hash is computed twice in one run over a deterministic walk
(sorted relative paths; `.git`, `target`, `__pycache__`, the
theodolite control directory, and the producer's `.bacon-locations`
are excluded, as is the freeze file itself when it lives in the
tree). The two computations must agree — a tree that changes while
it is read fails. The agreed hash is then compared with the freeze
record:

- No record yet: the agreed hash is recorded (atomically) and the
  check passes. Recording is the freeze-taking act; the record lives
  outside the workspace, on the operator's side, so workspace tools
  cannot rewrite the baseline they are measured against.
- Record present: its workspace must be this workspace (else exit 2)
  and its hash must equal the agreed hash (else exit 1).

Exit codes: 0 = frozen and stable, 1 = drift or mid-read change,
2 = unreadable record / usage error.

Test seam: when the environment variable FLEET_FREEZE_TEST_MUTATE
names a workspace-relative path, one byte is appended to that file
between the two passes. It exists only so the test suite can prove
the mid-read-change branch deterministically; operator configs never
set it, and phlow's check runner passes no model-chosen environment.
"""

import argparse
import hashlib
import json
import os
import sys
from datetime import datetime, timezone

CHUNK_BYTES = 64 * 1024
FILES_MAX_DEFAULT = 200_000
EXCLUDED_DIRS = {".git", "target", "__pycache__", ".phlow-fleet-control"}
EXCLUDED_FILES = {".bacon-locations"}
TEST_MUTATE_ENV = "FLEET_FREEZE_TEST_MUTATE"


class CheckError(Exception):
    """Unreadable record or bad input: the run proves nothing."""


def hash_file(path):
    """Stream one regular file through SHA-256."""
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        while True:
            chunk = handle.read(CHUNK_BYTES)
            if not chunk:
                break
            digest.update(chunk)
    return digest.hexdigest()


def hash_tree(workspace, freeze_file, files_max):
    """One deterministic pass: returns (tree hex digest, file count).

    Symlinks are hashed as their target text and never followed, so a
    link cannot pull content from outside the tree into the hash.
    """
    tree = hashlib.sha256()
    count = 0
    for root, dirs, files in os.walk(workspace, followlinks=False):
        dirs[:] = sorted(d for d in dirs if d not in EXCLUDED_DIRS)
        for name in sorted(files):
            if name in EXCLUDED_FILES:
                continue
            path = os.path.join(root, name)
            if freeze_file is not None and os.path.realpath(path) == freeze_file:
                continue
            relative = os.path.relpath(path, workspace).replace(os.sep, "/")
            if os.path.islink(path):
                entry = f"{relative}\0link:{os.readlink(path)}\n"
            elif os.path.isfile(path):
                entry = f"{relative}\0file:{hash_file(path)}\n"
            else:
                continue
            tree.update(entry.encode("utf-8", "surrogateescape"))
            count += 1
            if count > files_max:
                raise CheckError(f"tree exceeds the bound of {files_max} files")
    return tree.hexdigest(), count


def apply_test_mutation(workspace):
    """The documented test seam: mutate one in-tree file, once."""
    relative = os.environ.get(TEST_MUTATE_ENV)
    if not relative:
        return
    path = os.path.realpath(os.path.join(workspace, relative))
    if not path.startswith(workspace + os.sep) or not os.path.isfile(path):
        raise CheckError(f"test mutation target is not in the tree: {relative}")
    with open(path, "ab") as handle:
        handle.write(b"\n")


def read_record(freeze_file):
    """The parsed freeze record, or None when no record exists yet."""
    if not os.path.isfile(freeze_file):
        return None
    try:
        with open(freeze_file, encoding="utf-8") as handle:
            record = json.load(handle)
    except (OSError, json.JSONDecodeError) as error:
        raise CheckError(f"unreadable freeze record: {error}") from error
    if not isinstance(record, dict) or "tree_sha256" not in record:
        raise CheckError("freeze record has no tree_sha256")
    return record


def write_record(freeze_file, workspace, tree_hex, count):
    """Atomically replace the freeze record (temp file + rename)."""
    record = {
        "tool": "tree_freeze.py",
        "workspace": workspace,
        "tree_sha256": tree_hex,
        "files": count,
        "recorded_at": datetime.now(timezone.utc).isoformat(),
    }
    directory = os.path.dirname(freeze_file) or "."
    os.makedirs(directory, exist_ok=True)
    temporary = os.path.join(directory, ".tree-freeze.tmp")
    with open(temporary, "w", encoding="utf-8") as handle:
        json.dump(record, handle, indent=2, sort_keys=True)
        handle.write("\n")
    os.replace(temporary, freeze_file)


def run(argv):
    """Execute the check; returns the process exit code."""
    parser = argparse.ArgumentParser(prog="tree_freeze")
    parser.add_argument("--workspace", required=True)
    parser.add_argument("--freeze-file", required=True)
    parser.add_argument("--max-files", type=int, default=FILES_MAX_DEFAULT)
    args = parser.parse_args(argv)
    workspace = os.path.realpath(args.workspace)
    if not os.path.isdir(workspace):
        raise CheckError(f"workspace is not a directory: {args.workspace}")
    if not 0 < args.max_files <= 10_000_000:
        raise CheckError(f"--max-files out of range: {args.max_files}")
    freeze_file = os.path.realpath(args.freeze_file)
    first, _count = hash_tree(workspace, freeze_file, args.max_files)
    apply_test_mutation(workspace)
    agreed, count = hash_tree(workspace, freeze_file, args.max_files)
    if first != agreed:
        print(f"tree_freeze: FAIL tree changed while it was read "
              f"({first} != {agreed})")
        return 1
    print(f"tree_freeze: files={count} sha256={agreed}")
    record = read_record(freeze_file)
    if record is None:
        write_record(freeze_file, workspace, agreed, count)
        print("tree_freeze: RECORDED freeze written")
        return 0
    if record.get("workspace") != workspace:
        raise CheckError(
            f"freeze record names a different workspace: "
            f"{record.get('workspace')!r}"
        )
    if record["tree_sha256"] != agreed:
        print(f"tree_freeze: FAIL drift from the recorded freeze "
              f"{record['tree_sha256']}")
        return 1
    print("tree_freeze: PASS matches the recorded freeze")
    return 0


def main():
    try:
        return run(sys.argv[1:])
    except CheckError as error:
        print(f"tree_freeze: UNPROVEN {error}")
        return 2


if __name__ == "__main__":
    sys.exit(main())
