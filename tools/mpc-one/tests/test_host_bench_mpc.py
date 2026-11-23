#!/usr/bin/env python3
"""Tests for host_bench_mpc.py against a fake test build. No device.

As in test_app_mpc.py, a fake ssh runs the tool's real remote scripts under
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
_spec = importlib.util.spec_from_file_location("test_app_mpc", os.path.join(HERE, "test_app_mpc.py"))
th = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(th)

FAKE_SSH = th.FAKE_SSH.replace(
    "program = 'exec \"$R/run/mpfree-app/mpfree\"'\n"
    "if program in script:\n"
    "    script = script.replace(program, 'exec \"$FAKE_PROGRAM\"')",
    "program = 'exec \"$R/run/mpfree-bench/$p\"'\n"
    "if program in script:\n"
    "    script = script.replace(program, 'exec \"$FAKE_PROGRAM\" \"$p\"')")
assert FAKE_SSH != th.FAKE_SSH, "the fake ssh's program line moved in test_app_mpc.py"

FAKE_BENCH = r'''#!/bin/sh
build=$1; shift
echo "bench: $build $*" >> "$FAKE_LOG"
[ -x "./$build" ] || { echo "STOP: not copied"; exit 1; }
case "$1" in
  --calibrate) echo "# mpfree-bench calibration: reference kernel, ns per call"; echo "biquad 9000.0" ;;
  --json) echo "Poly Init, 8 voices   400.0"; echo "{\"schema\": \"mpfree-bench/1\", \"build\": \"$build\"}" > "$2"
          echo "Wrote $2"; exit "$FAKE_BENCH_RC" ;;
  --afford) echo "Poly Supersaw  18.4  7.40  16+ (75)" ;;
  --starts) echo "Keys Tines  120.0  10.0  12.0  6.88  130.0  12.0  over the synth budget" ;;
  --plain) [ "$2" = --json ] || { echo "STOP: plain args $*"; exit 1; }
           echo "{\"schema\": \"mpfree-bench/1\", \"build\": \"$build plain\"}" > "$3"; echo "Wrote $3" ;;
  --stems) [ "$*" = "--stems" ] || { echo "STOP: stems args $*"; exit 1; }
           echo "stems: NEON matches plain (the GEMM kernel over 300 calls, the half-to-float kernel on all 65,536 halves)"
           echo "stems: estimate (two cores): a 10 s sample 9.4 s at 1024 bins" ;;
  --lanes) [ "$*" = "--lanes --lane-cores 3,2" ] || { echo "STOP: lanes args $*"; exit 1; }
           echo "This thread: pinned to core 3."; echo "Lanes: a worker thread each; worker core: pinned to 2" ;;
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
                    "MPFREE_APP_TEST": "1", "MPFREE_ACCEPT_BENCH": "YES", "FAKE_STAGE": self.stage,
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

    def run_helper(self, answers="\n" * 4, before=(), **env):
        cmd = [sys.executable, "-I", HELPER, "--ip", "192.0.2.141", "--key", self.key, "--work", self.work,
               "--report", self.report, "--default-program", self.programs["default"],
               "--a17-program", self.programs["a17"]]
        for value in before:
            cmd += ["--before", value]
        return subprocess.run(cmd, input=answers, env=dict(self.env, **env), text=True, stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, timeout=120)

    def lines(self, prefix):
        with open(self.log) as fh:
            return [ln[len(prefix):].strip() for ln in fh.read().splitlines() if ln.startswith(prefix)]

    def test_copies_both_builds_and_runs_only_the_fixed_arguments(self):
        res = self.run_helper()
        self.assertEqual(res.returncode, 0, res.stdout)
        for text in ("static ARM EABI5 hard-float", "Poly Supersaw  18.4  7.40  16+ (75)", "CHECKS-OK", "default: copied; its hash on the MPC matches",
                     "a17: copied; its hash on the MPC matches", "biquad 9000.0", "Poly Init, 8 voices",
                     "----- default.json -----", "Poly Supersaw  18.4  7.40  16+ (75)", '{"schema": "mpfree-bench/1", "build": "a17"}',
                     "This thread: pinned to core 3.", "Lanes: a worker thread each; worker core: pinned to 2",
                     "Keys Tines  120.0  10.0  12.0  6.88  130.0  12.0  over the synth budget",
                     "----- default-plain.json -----", '{"schema": "mpfree-bench/1", "build": "default plain"}',
                     "stems: NEON matches plain", "stems: estimate", "RESULT: both builds ran"):
            self.assertIn(text, res.stdout)
        self.assertEqual(self.lines("bench: "), ["default --calibrate", "default --json default.json", "default --afford",
                                                 "default --lanes --lane-cores 3,2", "default --starts",
                                                 "default --plain --json default-plain.json", "default --stems",
                                                 "a17 --calibrate", "a17 --json a17.json", "a17 --afford",
                                                 "a17 --lanes --lane-cores 3,2"])
        # Services: only our own pad player is stopped for the runs and started again after; Akai's are only read.
        changes = [c for c in self.lines("systemctl: ") if not c.startswith("is-active ")]
        self.assertEqual(changes, ["stop mpfree-test-app.service", "start mpfree-test-app.service"])
        self.assertIn("pad player: active (started again)", res.stdout)
        for name, path in self.programs.items():
            with open(path, "rb") as a, open(f"{self.root}/run/mpfree-bench/{name}", "rb") as b:
                self.assertEqual(hashlib.sha256(a.read()).digest(), hashlib.sha256(b.read()).digest())
            self.assertEqual(os.stat(f"{self.root}/run/mpfree-bench/{name}").st_mode & 0o777, 0o700)
        self.assertEqual(sorted(os.listdir(f"{self.root}/run")), ["mpfree-bench"])
        with open(self.report) as fh:
            self.assertIn("RESULT: both builds ran", fh.read())

    def test_earlier_builds_run_calibrate_and_json_only_after_the_two(self):
        """--before COMMIT=PATH: each earlier build is checked and copied with the two, and runs the reference
        kernels and every engine after them, in its own last step; s at that step skips them all."""
        old = {c: os.path.join(self.tmp, c, "mpfree-bench") for c in ("11347cb0", "c0c9bce4")}
        for path in old.values():
            th.write(path, th.arm_elf(), 0o755)
        before = [f"{c}={p}" for c, p in old.items()]
        res = self.run_helper(answers="\n" * 4, before=before)
        self.assertEqual(res.returncode, 0, res.stdout)
        for text in ("11347cb0 (earlier): ", "c0c9bce4: copied; its hash on the MPC matches", "Copies 4 programs",
                     "[6/6] The earlier builds, 11347cb0, c0c9bce4", "----- c0c9bce4.json -----",
                     '{"schema": "mpfree-bench/1", "build": "11347cb0"}', "RESULT: both builds ran"):
            self.assertIn(text, res.stdout)
        self.assertEqual(self.lines("bench: ")[11:], ["11347cb0 --calibrate", "11347cb0 --json 11347cb0.json",
                                                     "c0c9bce4 --calibrate", "c0c9bce4 --json c0c9bce4.json"])
        for name, path in old.items():
            with open(path, "rb") as a, open(f"{self.root}/run/mpfree-bench/{name}", "rb") as b:
                self.assertEqual(hashlib.sha256(a.read()).digest(), hashlib.sha256(b.read()).digest())
        self.assertIn("pad player: active (started again)", res.stdout)
        # s at the last step skips the earlier builds; the two have run.
        res = self.run_helper(answers="\n" * 3 + "s\n", before=before)
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertEqual([ln for ln in self.lines("bench: ")[15:] if not ln.startswith(("default ", "a17 "))], [])
        # The copy starts afresh: the last run's JSON of an earlier build went before it was copied again.
        self.assertNotIn("11347cb0.json", os.listdir(f"{self.root}/run/mpfree-bench"))
        self.assertIn("11347cb0", os.listdir(f"{self.root}/run/mpfree-bench"))

    def test_an_earlier_build_needs_a_commit_label_and_a_good_program(self):
        for value in ("11347cb0", "main=/x", "11347CB0=/x", "1134=/x", "11347cb0=", "../x=/y", "a17=/x"):
            res = self.run_helper(before=[value])
            self.assertEqual(res.returncode, 1, value)
            self.assertIn("give COMMIT=PATH", res.stdout, value)
        path = os.path.join(self.tmp, "old", "mpfree-bench")
        th.write(path, th.arm_elf(), 0o755)
        res = self.run_helper(before=[f"11347cb0={path}", f"11347cb0={path}"])
        self.assertIn("--before 11347cb0 is given twice", res.stdout)
        th.write(path, th.arm_elf(interp=True), 0o755)
        res = self.run_helper(before=[f"11347cb0={path}"])
        self.assertEqual(res.returncode, 1)
        self.assertIn("dynamically linked", res.stdout)
        self.assertEqual(self.lines("bench: "), [])
        self.assertFalse(os.path.exists(f"{self.root}/run/mpfree-bench"))

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
        # A failed run still starts the pad player again.
        changes = [c for c in self.lines("systemctl: ") if not c.startswith("is-active ")]
        self.assertEqual(changes[-1], "start mpfree-test-app.service")
        self.assertIn("(started again)", res.stdout)

    def test_q_stops_before_the_copy(self):
        res = self.run_helper(answers="q\n")
        self.assertEqual(res.returncode, 1)
        self.assertIn("STOP: stopped by you", res.stdout)
        self.assertFalse(os.path.exists(f"{self.root}/run/mpfree-bench"))
        self.assertEqual([c for c in self.lines("systemctl: ") if not c.startswith("is-active ")], [])


if __name__ == "__main__":
    unittest.main(verbosity=2)
