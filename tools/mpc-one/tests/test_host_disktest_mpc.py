#!/usr/bin/env python3
"""Tests for host_disktest_mpc.py against a fake test build. No device.

As in test_app_mpc.py, a fake ssh runs the tool's real remote scripts under
BusyBox applets against a fake root folder (setting the scripts' R='' root
prefix to it), accepts only the test build's host key alias, and only while
the fake MPC is "on the test build". Running the disk test is the one other
change: a shell stand-in logs its arguments and prints what the real one
would (or refuses, with exit code 3, as the real one does off the stick).

Set MPFREE_REQUIRE_BUSYBOX=1 (CI does) to fail instead of skip without BusyBox.
"""
import hashlib
import importlib.util
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
TOOLS = os.path.dirname(HERE)
HELPER = os.path.join(TOOLS, "host_disktest_mpc.py")
_spec = importlib.util.spec_from_file_location("test_app_mpc", os.path.join(HERE, "test_app_mpc.py"))
th = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(th)

FAKE_SSH = th.FAKE_SSH.replace(
    "program = 'exec \"$R/run/mpfree-app/mpfree\"'\n"
    "if program in script:\n"
    "    script = script.replace(program, 'exec \"$FAKE_PROGRAM\"')",
    "program = 'exec \"$R/run/mpfree-disktest/mpfree-disktest\"'\n"
    "if program in script:\n"
    "    script = script.replace(program, 'exec \"$FAKE_PROGRAM\"')")
assert FAKE_SSH != th.FAKE_SSH, "the fake ssh's program line moved in test_app_mpc.py"

FAKE_DISKTEST = r'''#!/bin/sh
echo "disktest: [$*]" >> "$FAKE_LOG"
[ -x ./mpfree-disktest ] || { echo "STOP: not copied"; exit 1; }
if [ "$FAKE_DISKTEST_RC" = 3 ]; then echo "mpfree-disktest: refused: 40.0 MiB free" >&2; exit 3; fi
echo "free before: 900.0 MiB on /run/mpfree-data"
echo "  write: 1488 ops, p50 0.10 ms, p99 31.00 ms, max 412.00 ms; over 50/100/250/500 ms: 9/4/1/0"
echo "free after: 900.0 MiB (the folder is deleted)"
exit "$FAKE_DISKTEST_RC"
'''


class DiskTestTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.busybox = shutil.which("busybox")
        if not cls.busybox:
            if os.environ.get("MPFREE_REQUIRE_BUSYBOX") == "1":
                raise RuntimeError("BusyBox is required for these tests")
            raise unittest.SkipTest("BusyBox not installed")

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="mpfree-disktest-test-")
        t = self.tmp
        self.work, self.log, self.stage = os.path.join(t, "work"), os.path.join(t, "log"), os.path.join(t, "stage")
        self.root, self.report = os.path.join(t, "root310"), os.path.join(t, "report.txt")
        th.write(self.stage, "310\n")
        th.write(self.log, "")
        fakebin, remote = os.path.join(t, "fakebin"), os.path.join(t, "remote-bin")
        th.write(os.path.join(fakebin, "ssh"), FAKE_SSH, 0o755)
        os.makedirs(remote)
        for name in th.APPLETS:
            os.symlink(self.busybox, os.path.join(remote, name))
        th.write(os.path.join(remote, "systemctl"), th.FAKE_SYSTEMCTL, 0o755)
        screen, app = os.path.join(t, "screen-state"), os.path.join(t, "app-state")
        th.write(screen, "inactive\n")
        th.write(app, "active\n")
        standin = os.path.join(t, "disktest-standin")
        th.write(standin, FAKE_DISKTEST, 0o755)
        self.env = {"PATH": fakebin + os.pathsep + os.environ.get("PATH", "/usr/bin:/bin"), "HOME": t,
                    "MPFREE_APP_TEST": "1", "MPFREE_ACCEPT_DISKTEST": "YES", "FAKE_STAGE": self.stage,
                    "FAKE_LOG": self.log, "FAKE_ROOT": self.root, "FAKE_REMOTE_PATH": remote, "FAKE_SCREEN": screen,
                    "FAKE_APP": app, "FAKE_ACVS": "inactive", "FAKE_PROGRAM": standin, "FAKE_DISKTEST_RC": "0"}
        self.key = os.path.join(t, "key")
        th.write(self.key, "fake key\n", 0o600)
        self.program = os.path.join(t, "build", "mpfree-disktest")
        th.write(self.program, th.arm_elf(), 0o755)
        folder = os.path.join(self.work, "kexec-stick-20260101T000000Z-abc")
        th.write(os.path.join(folder, "stick.img"), b"\x00" * 4096)
        th.write(os.path.join(folder, "known_hosts"), "mpfree-kexec-test ssh-ed25519 AAAAfake\n")
        th.write(os.path.join(folder, "report.txt"), f"its kernel release: {th.NEW}\n")
        r = self.root
        th.write(f"{r}/proc/sys/kernel/osrelease", th.NEW + "\n")
        th.write(f"{r}/proc/sys/kernel/kexec_load_disabled", "1\n")
        os.makedirs(f"{r}/sys/block/sda")
        th.write(f"{r}/proc/mounts", "/dev/root / ext4 ro,relatime 0 0\ntmpfs /run tmpfs rw,nosuid,nodev,mode=755 0 0\n")
        os.makedirs(f"{r}/run/mpfree-data")

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def run_helper(self, answers="\n" * 2, **env):
        cmd = [sys.executable, "-I", HELPER, "--ip", "192.0.2.141", "--key", self.key, "--work", self.work,
               "--report", self.report, "--program", self.program]
        return subprocess.run(cmd, input=answers, env=dict(self.env, **env), text=True, stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, timeout=120)

    def lines(self, prefix):
        with open(self.log) as fh:
            return [ln[len(prefix):].strip() for ln in fh.read().splitlines() if ln.startswith(prefix)]

    def test_copies_the_program_and_runs_it_with_no_arguments(self):
        res = self.run_helper()
        self.assertEqual(res.returncode, 0, res.stdout)
        for text in ("static ARM EABI5 hard-float", "CHECKS-OK", "/run/mpfree-data",
                     "copied; its hash on the MPC matches", "WRITES test files in /run/mpfree-data/mpfree-disktest",
                     "over 50/100/250/500 ms: 9/4/1/0", "the folder is deleted", "RESULT: the disk test ran"):
            self.assertIn(text, res.stdout)
        self.assertEqual(self.lines("disktest: "), ["[]"])
        # Read-only on services: only is-active, never a start or stop.
        self.assertTrue(all(c.startswith("is-active ") for c in self.lines("systemctl: ")), self.lines("systemctl: "))
        with open(self.program, "rb") as a, open(f"{self.root}/run/mpfree-disktest/mpfree-disktest", "rb") as b:
            self.assertEqual(hashlib.sha256(a.read()).digest(), hashlib.sha256(b.read()).digest())
        self.assertEqual(os.stat(f"{self.root}/run/mpfree-disktest/mpfree-disktest").st_mode & 0o777, 0o700)
        self.assertEqual(sorted(os.listdir(f"{self.root}/run")), ["mpfree-data", "mpfree-disktest"])
        self.assertEqual(os.listdir(f"{self.root}/run/mpfree-data"), [])
        with open(self.report) as fh:
            self.assertIn("RESULT: the disk test ran", fh.read())

    def test_a_refusal_is_reported_as_nothing_written(self):
        res = self.run_helper(FAKE_DISKTEST_RC="3")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("refused: 40.0 MiB free", res.stdout)
        self.assertIn("RESULT: refused by the disk test; nothing was written on the stick.", res.stdout)

    def test_a_failed_run_says_how_to_clean_up(self):
        res = self.run_helper(FAKE_DISKTEST_RC="1")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("ended with exit code 1", res.stdout)
        self.assertIn("rm -r /run/mpfree-data/mpfree-disktest", res.stdout)

    def test_needs_the_opt_in_and_refuses_stock(self):
        res = self.run_helper(MPFREE_ACCEPT_DISKTEST="")
        self.assertEqual(res.returncode, 1)
        self.assertIn("opt-in required", res.stdout)
        th.write(self.stage, "36\n")
        res = self.run_helper()
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("the test build did not answer", res.stdout)
        self.assertEqual(self.lines("disktest: "), [])
        self.assertFalse(os.path.exists(f"{self.root}/run/mpfree-disktest"))

    def test_stops_on_a_dynamic_program_before_any_ssh(self):
        th.write(self.program, th.arm_elf(interp=True), 0o755)
        res = self.run_helper()
        self.assertEqual(res.returncode, 1)
        self.assertIn("dynamically linked", res.stdout)
        self.assertEqual(self.lines("ssh["), [])

    def test_q_stops_before_the_copy_and_s_skips_the_run(self):
        res = self.run_helper(answers="q\n")
        self.assertEqual(res.returncode, 1)
        self.assertIn("STOP: stopped by you", res.stdout)
        self.assertFalse(os.path.exists(f"{self.root}/run/mpfree-disktest"))
        res = self.run_helper(answers="\ns\n")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("skipped: nothing was written on the stick", res.stdout)
        self.assertEqual(self.lines("disktest: "), [])


if __name__ == "__main__":
    unittest.main(verbosity=2)
