#!/usr/bin/env python3
"""Regression tests for run_corpus.py's git hardening (CVE-2026-129128).

A hostile fixture repo points diff.external and core.fsmonitor at
marker-writing scripts. An unhardened control invocation proves the
fixture is live; the same operations through run_corpus.git() must
create no markers, and must still report real data.

Run: python3 -m unittest test_git_hardening -v   (from this directory)
"""

import os
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import run_corpus  # noqa: E402


def raw_git(root, *args):
    return subprocess.run(
        ["git", "-C", root] + list(args),
        capture_output=True, text=True, timeout=60,
    )


class GitHardeningTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="corpus-git-hardening-")
        self.root = self.tmp.name
        self.ext_marker = os.path.join(self.root, "extdiff-fired")
        self.fsm_marker = os.path.join(self.root, "fsmonitor-fired")
        for name, marker in (("ext.sh", self.ext_marker), ("fsm.sh", self.fsm_marker)):
            script = os.path.join(self.root, name)
            with open(script, "w", encoding="utf-8") as handle:
                handle.write(f"#!/bin/sh\ntouch '{marker}'\n")
            os.chmod(script, 0o755)
        self.assertEqual(raw_git(self.root, "init", "-q").returncode, 0)
        self.assertEqual(
            raw_git(self.root, "config", "diff.external",
                    os.path.join(self.root, "ext.sh")).returncode, 0)
        self.assertEqual(
            raw_git(self.root, "config", "core.fsmonitor",
                    os.path.join(self.root, "fsm.sh")).returncode, 0)
        with open(os.path.join(self.root, "tracked.txt"), "w", encoding="utf-8") as handle:
            handle.write("one\n")
        self.assertEqual(raw_git(self.root, "add", "tracked.txt").returncode, 0)
        self.assertEqual(
            raw_git(self.root, "-c", "user.email=test@example.invalid",
                    "-c", "user.name=test", "commit", "-qm", "init").returncode, 0)
        with open(os.path.join(self.root, "tracked.txt"), "w", encoding="utf-8") as handle:
            handle.write("one\ntwo\n")

    def tearDown(self):
        self.tmp.cleanup()

    def test_control_fixture_fires_under_raw_git(self):
        raw_git(self.root, "status", "--porcelain")
        raw_git(self.root, "diff")
        self.assertTrue(os.path.exists(self.fsm_marker),
                        "control: fsmonitor script must fire under raw git")
        self.assertTrue(os.path.exists(self.ext_marker),
                        "control: external diff must fire under raw git")

    def test_hardened_git_never_fires_and_still_reports(self):
        # setUp's own raw git add/commit may legitimately have fired the
        # fixture scripts (config was already weaponised); start clean so
        # only the hardened calls below are measured.
        for marker in (self.ext_marker, self.fsm_marker):
            if os.path.exists(marker):
                os.remove(marker)
        status = run_corpus.git(["status", "--porcelain"], self.root)
        self.assertIn("tracked.txt", status)
        stat = run_corpus.git(["diff", "--stat"], self.root)
        self.assertIn("tracked.txt", stat)
        run_corpus.git(["checkout", "--", "."], self.root)
        self.assertFalse(os.path.exists(self.fsm_marker),
                         "fsmonitor script fired through hardened git()")
        self.assertFalse(os.path.exists(self.ext_marker),
                         "external diff fired through hardened git()")


if __name__ == "__main__":
    unittest.main()
