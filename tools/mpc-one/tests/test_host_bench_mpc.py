#!/usr/bin/env python3
"""Tests for host_bench_mpc.py against a fake test build. No device.

As in test_hello_mpc.py, a fake ssh runs the tool's real remote scripts under
BusyBox applets against a fake root folder (setting the scripts' R='' root
prefix to it), accepts only the test build's host key alias, and only while
the fake MPC is "on the test build". Running the benchmark is the one other
change: a shell stand-in logs its arguments and prints what the real one
would, and writes its JSON file where the real one would.

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
HELPER = os.path.join(TOOLS, "host_bench_mpc.py")
_spec = importlib.util.spec_from_file_location("test_hello_mpc", os.path.join(HERE, "test_hello_mpc.py"))
th = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(th)

FAKE_SSH = th.FAKE_SSH.replace(
    "program = 'exec \"$R/run/mpfree-hello/mpfree-hello\"'\n"
    "if program in script:\n"
    "    script = script.replace(program, 'exec \"$FAKE_PROGRAM\"')",
    "program = 'exec \"$R/run/mpfree-bench/$p\"'\n"
    "if program in script:\n"
    "    script = script.replace(program, 'exec \"$FAKE_PROGRAM\" \"$p\"')")
assert FAKE_SSH != th.FAKE_SSH, "the fake ssh's program line moved in test_hello_mpc.py"

FAKE_BENCH = r'''#!/bin/sh
build=$1; shift
echo "bench: $build $*" >> "$FAKE_LOG"
[ -x "./$build" ] || { echo "STOP: not copied"; exit 1; }
case "$1" in
  --calibrate) echo "# mpfree-bench calibration: reference kernel, ns per call"; echo "biquad 9000.0" ;;
  --json) echo "Poly Init, 8 voices   400.0"; echo "{\"schema\": \"mpfree-bench/1\", \"build\": \"$build\"}" > "$2"
          echo "Wrote $2"; exit "$FAKE_BENCH_RC" ;;
esac
'''


class BenchTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.busybox = shutil.which("busybox")
        if not cls.busybox:
            if os.environ.get("MPFREE_REQUIRE_BUSYBOX") == "1":
                raise RuntimeError("BusyBox is required for these tests")
            raise unittest.SkipTest("BusyBox not installed")

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="mpfree-bench-test-")
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
        standin = os.path.join(t, "bench-standin")
        th.write(standin, FAKE_BENCH, 0o755)
        self.env = {"PATH": fakebin + os.pathsep + os.environ.get("PATH", "/usr/bin:/bin"), "HOME": t,
                    "MPFREE_HELLO_TEST": "1", "MPFREE_ACCEPT_BENCH": "YES", "FAKE_STAGE": self.stage,
                    "FAKE_LOG": self.log, "FAKE_ROOT": self.root, "FAKE_REMOTE_PATH": remote, "FAKE_SCREEN": screen,
                    "FAKE_APP": app, "FAKE_ACVS": "inactive", "FAKE_PROGRAM": standin, "FAKE_BENCH_RC": "0"}
        self.key = os.path.join(t, "key")
        th.write(self.key, "fake key\n", 0o600)
        self.programs = {n: os.path.join(t, n, "mpfree-bench") for n in ("default", "a17")}
        for path in self.programs.values():
            th.write(path, th.arm_elf(), 0o755)
        folder = os.path.join(self.work, "kexec-stick-20260101T000000Z-abc")
        th.write(os.path.join(folder, "stick.img"), b"\x00" * 4096)
        th.write(os.path.join(folder, "known_hosts"), "mpfree-kexec-test ssh-ed25519 AAAAfake\n")
        th.write(os.path.join(folder, "report.txt"), f"its kernel release: {th.NEW}\n")
        r = self.root
        th.write(f"{r}/proc/sys/kernel/osrelease", th.NEW + "\n")
        th.write(f"{r}/proc/sys/kernel/kexec_load_disabled", "1\n")
        os.makedirs(f"{r}/sys/block/sda")
        th.write(f"{r}/proc/mounts", "/dev/root / ext4 ro,relatime 0 0\ntmpfs /run tmpfs rw,nosuid,nodev,mode=755 0 0\n")
        os.makedirs(f"{r}/run")

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def run_helper(self, answers="\n" * 4, **env):
        cmd = [sys.executable, "-I", HELPER, "--ip", "192.0.2.141", "--key", self.key, "--work", self.work,
               "--report", self.report, "--default-program", self.programs["default"],
               "--a17-program", self.programs["a17"]]
        return subprocess.run(cmd, input=answers, env=dict(self.env, **env), text=True, stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, timeout=120)

    def lines(self, prefix):
        with open(self.log) as fh:
            return [ln[len(prefix):].strip() for ln in fh.read().splitlines() if ln.startswith(prefix)]

    def test_copies_both_builds_and_runs_only_the_fixed_arguments(self):
        res = self.run_helper()
        self.assertEqual(res.returncode, 0, res.stdout)
        for text in ("static ARM EABI5 hard-float", "CHECKS-OK", "default: copied; its hash on the MPC matches",
                     "a17: copied; its hash on the MPC matches", "biquad 9000.0", "Poly Init, 8 voices",
                     "----- default.json -----", '{"schema": "mpfree-bench/1", "build": "a17"}',
                     "RESULT: both builds ran"):
            self.assertIn(text, res.stdout)
        self.assertEqual(self.lines("bench: "), ["default --calibrate", "default --json default.json",
                                                 "a17 --calibrate", "a17 --json a17.json"])
        # Read-only on services: only is-active, never a start or stop.
        self.assertTrue(all(c.startswith("is-active ") for c in self.lines("systemctl: ")), self.lines("systemctl: "))
        for name, path in self.programs.items():
            with open(path, "rb") as a, open(f"{self.root}/run/mpfree-bench/{name}", "rb") as b:
                self.assertEqual(hashlib.sha256(a.read()).digest(), hashlib.sha256(b.read()).digest())
            self.assertEqual(os.stat(f"{self.root}/run/mpfree-bench/{name}").st_mode & 0o777, 0o700)
        self.assertEqual(sorted(os.listdir(f"{self.root}/run")), ["mpfree-bench"])
        with open(self.report) as fh:
            self.assertIn("RESULT: both builds ran", fh.read())

    def test_needs_the_opt_in_and_refuses_stock(self):
        res = self.run_helper(MPFREE_ACCEPT_BENCH="")
        self.assertEqual(res.returncode, 1)
        self.assertIn("opt-in required", res.stdout)
        th.write(self.stage, "36\n")
        res = self.run_helper()
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("the test build did not answer", res.stdout)
        self.assertEqual(self.lines("bench: "), [])
        self.assertFalse(os.path.exists(f"{self.root}/run/mpfree-bench"))

    def test_stops_on_a_bad_program_or_a_failed_run(self):
        th.write(self.programs["a17"], th.arm_elf(interp=True), 0o755)
        res = self.run_helper()
        self.assertEqual(res.returncode, 1)
        self.assertIn("dynamically linked", res.stdout)
        self.assertEqual(self.lines("ssh["), [])
        th.write(self.programs["a17"], th.arm_elf(), 0o755)
        res = self.run_helper(FAKE_BENCH_RC="3")
        self.assertEqual(res.returncode, 1)
        self.assertIn("default --json ended with exit code 3", res.stdout)
        self.assertEqual(self.lines("bench: "), ["default --calibrate", "default --json default.json"])

    def test_q_stops_before_the_copy(self):
        res = self.run_helper(answers="q\n")
        self.assertEqual(res.returncode, 1)
        self.assertIn("STOP: stopped by you", res.stdout)
        self.assertFalse(os.path.exists(f"{self.root}/run/mpfree-bench"))


if __name__ == "__main__":
    unittest.main(verbosity=2)
