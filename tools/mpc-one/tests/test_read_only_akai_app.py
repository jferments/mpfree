#!/usr/bin/env python3
"""Tests for read-only-akai-app.sh: it runs against a stand-in "MPC" process (a copy
of sleep) with the shell it will meet on the MPC, finds it, prints every section,
and stops cleanly when the app is not running. It also checks the script never
reads a process's environment or writes anything.

Runs with busybox sh when MPFREE_REQUIRE_BUSYBOX=1 (CI) or busybox is installed,
and with /bin/sh otherwise.
"""
import os
import re
import shutil
import subprocess
import tempfile
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPT = os.path.join(os.path.dirname(HERE), "read-only-akai-app.sh")
BUSYBOX = shutil.which("busybox")
if not BUSYBOX and os.environ.get("MPFREE_REQUIRE_BUSYBOX") == "1":
    raise SystemExit("busybox is required (MPFREE_REQUIRE_BUSYBOX=1)")
SHELL = [BUSYBOX, "sh"] if BUSYBOX else ["/bin/sh"]


class ReadOnlyAkaiAppTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.fake = os.path.join(self.tmp, "MPC")
        shutil.copy(shutil.which("sleep"), self.fake)
        self.proc = subprocess.Popen([self.fake, "60"])
        time.sleep(0.2)

    def tearDown(self):
        self.proc.kill()
        self.proc.wait()
        shutil.rmtree(self.tmp, ignore_errors=True)

    def run_script(self, **env):
        before = set(os.listdir(self.tmp))
        res = subprocess.run(SHELL + [SCRIPT], capture_output=True, text=True, timeout=60, cwd=self.tmp,
                             env=dict(os.environ, **env))
        self.assertEqual(set(os.listdir(self.tmp)), before, "the script wrote a file")
        return res

    def test_finds_the_app_and_prints_every_section(self):
        res = self.run_script(MPFREE_APP_EXE=self.fake)
        self.assertEqual(res.returncode, 0, res.stdout + res.stderr)
        out = res.stdout
        self.assertIn(f"pid {self.proc.pid}, executable {self.fake}", out)
        for section in ("=== memory ===", "=== limits and cgroup ===", "=== CPU ===",
                        "=== files it has mapped", "=== files and devices it has open",
                        "=== sound, MIDI and input devices ===", "=== kernel features for guest mode ===",
                        "DONE: read-only"):
            self.assertIn(section, out)
        self.assertRegex(out, r"VmRSS:\s+\d+ kB")
        self.assertRegex(out, r"Max locked memory")
        self.assertRegex(out, r"app CPU so far: user \d+ ticks")
        self.assertRegex(out, r"\n  MPC \| \d+ \| \d+ \| \d+\n")
        self.assertIn(self.fake + "\n", out)  # the executable is among the mapped files

    def test_finds_the_app_by_its_name(self):
        res = self.run_script(MPFREE_APP_EXE="/nonexistent/MPC")
        self.assertEqual(res.returncode, 0, res.stdout + res.stderr)
        self.assertIn(f"pid {self.proc.pid}", res.stdout)

    def test_stops_when_the_app_is_not_running(self):
        res = self.run_script(MPFREE_APP_EXE="/nonexistent/MPC", MPFREE_APP_COMM="no-such-app")
        self.assertEqual(res.returncode, 1)
        self.assertIn("STOP: Akai's app (/nonexistent/MPC) is not running", res.stdout)

    def test_never_reads_environments_or_writes(self):
        with open(SCRIPT) as fh:
            body = "\n".join(line for line in fh if not line.lstrip().startswith("#"))
        self.assertNotIn("environ", body)
        self.assertNotRegex(body, r"(^|[^2])>\s*[^&/]")  # no redirection into a file (2>/dev/null is fine)
        for word in ("kill", "systemctl", "rm ", "mount", "echo 1 >", "tee "):
            self.assertNotIn(word, body)
        self.assertIsNone(re.search(r"\bdd\b", body))


if __name__ == "__main__":
    unittest.main()
