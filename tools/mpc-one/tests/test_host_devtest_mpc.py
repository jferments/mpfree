#!/usr/bin/env python3
"""Tests for host_devtest_mpc.py against a fake test build. No device.

As in test_host_bench_mpc.py, a fake ssh runs the tool's real remote scripts
under BusyBox applets against a fake root folder (setting the scripts' R=''
root prefix to it), accepts only the test build's host key alias, and only
while the fake MPC is "on the test build". Playing a workflow is the one other
change: a shell stand-in for mpfree logs its arguments and prints what the
real one would.

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
HELPER = os.path.join(TOOLS, "host_devtest_mpc.py")
_spec = importlib.util.spec_from_file_location("test_app_mpc", os.path.join(HERE, "test_app_mpc.py"))
th = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(th)

FAKE_SSH = th.FAKE_SSH.replace(
    "program = 'exec \"$R/run/mpfree-app/mpfree\"'",
    "program = 'exec \"$R/run/mpfree-devtest/mpfree\"'")
assert FAKE_SSH != th.FAKE_SSH, "the fake ssh's program line moved in test_app_mpc.py"

# Stands in for mpfree play --script: logs its arguments (with the data folder relative to the fake root),
# checks the workflow and the data folder are where the tool put them, and prints a workflow's lines.
FAKE_MPFREE = r'''#!/bin/sh
echo "mpfree: $*" | sed "s|$FAKE_ROOT||g" >> "$FAKE_LOG"
[ "$1" = play ] && [ "$2" = --script ] || { echo "STOP: not a script run"; exit 1; }
[ -f "$3" ] || { echo "STOP: no workflow $3"; exit 1; }
[ -d "$7" ] && [ -z "$(ls -A "$7")" ] || { echo "STOP: no empty data folder"; exit 1; }
name=$(basename "$3" .mpw)
echo "script: \"$name\", 3 steps, 2.0 s"
echo "script: line 2 at 0.000 s: mark start"
echo "status: 0 hits (0 from the screen), 0 repeats, 0 underruns in 700 periods"
echo "script: done"
case ",$FAKE_FAIL," in *",$name,"*) exit 3 ;; esac
exit 0
'''


class DevtestTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.busybox = shutil.which("busybox")
        if not cls.busybox:
            if os.environ.get("MPFREE_REQUIRE_BUSYBOX") == "1":
                raise RuntimeError("BusyBox is required for these tests")
            raise unittest.SkipTest("BusyBox not installed")

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="mpfree-devtest-test-")
        t = self.tmp
        self.work, self.log, self.stage = os.path.join(t, "work"), os.path.join(t, "log"), os.path.join(t, "stage")
        self.root, self.report = os.path.join(t, "root310"), os.path.join(t, "report.txt")
        th.write(self.stage, "310\n")
        th.write(self.log, "")
        fakebin, remote = os.path.join(t, "fakebin"), os.path.join(t, "remote-bin")
        th.write(os.path.join(fakebin, "ssh"), FAKE_SSH, 0o755)
        os.makedirs(remote)
        for name in th.APPLETS + ("basename", "sed"):
            os.symlink(self.busybox, os.path.join(remote, name))
        th.write(os.path.join(remote, "systemctl"), th.FAKE_SYSTEMCTL, 0o755)
        screen, app = os.path.join(t, "screen-state"), os.path.join(t, "app-state")
        th.write(screen, "active\n")
        th.write(app, "active\n")
        standin = os.path.join(t, "mpfree-standin")
        th.write(standin, FAKE_MPFREE, 0o755)
        self.env = {"PATH": fakebin + os.pathsep + os.environ.get("PATH", "/usr/bin:/bin"), "HOME": t,
                    "MPFREE_APP_TEST": "1", "MPFREE_ACCEPT_DEVTEST": "YES", "FAKE_STAGE": self.stage,
                    "FAKE_LOG": self.log, "FAKE_ROOT": self.root, "FAKE_REMOTE_PATH": remote, "FAKE_SCREEN": screen,
                    "FAKE_APP": app, "FAKE_ACVS": "inactive", "FAKE_PROGRAM": standin, "FAKE_FAIL": ""}
        self.key = os.path.join(t, "key")
        th.write(self.key, "fake key\n", 0o600)
        self.program = os.path.join(t, "build", "mpfree")
        th.write(self.program, th.arm_elf(), 0o755)
        self.flows = os.path.join(t, "workflows")
        for name, body in (("beat", "title A beat\nwait 1\npad 1 100 0.5\ntap header tab Play\n"),
                           ("heavy-load", "title Heavy\nwait 30\n"), ("songs", "title Songs\nwait 60\n")):
            th.write(os.path.join(self.flows, f"{name}.mpw"), body)
        folder = os.path.join(self.work, "kexec-stick-20260101T000000Z-abc")
        th.write(os.path.join(folder, "stick.img"), b"\x00" * 4096)
        th.write(os.path.join(folder, "known_hosts"), "mpfree-kexec-test ssh-ed25519 AAAAfake\n")
        th.write(os.path.join(folder, "report.txt"), f"its kernel release: {th.NEW}\n")
        r = self.root
        th.write(f"{r}/proc/sys/kernel/osrelease", th.NEW + "\n")
        th.write(f"{r}/proc/sys/kernel/kexec_load_disabled", "1\n")
        os.makedirs(f"{r}/sys/block/sda")
        th.write(f"{r}/proc/mounts", "/dev/root / ext4 ro,relatime 0 0\ntmpfs /run tmpfs rw,nosuid,nodev,mode=755 0 0\n")
        th.write(f"{r}/run/mpfree-kits/kits.bin", b"MPFKITS1" + b"\x00" * 64)
        th.write(f"{r}/run/mpfree-data/projects/mine.mpfp", "a project of yours\n")

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def run_helper(self, answers="\n\nfine\n\n", extra=(), **env):
        cmd = [sys.executable, "-I", HELPER, "--ip", "192.0.2.141", "--key", self.key, "--work", self.work,
               "--report", self.report, "--program", self.program, "--workflows", self.flows, *extra]
        return subprocess.run(cmd, input=answers, env=dict(self.env, **env), text=True, stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, timeout=120)

    def lines(self, prefix):
        with open(self.log) as fh:
            return [ln[len(prefix):].strip() for ln in fh.read().splitlines() if ln.startswith(prefix)]

    def run_line(self, workflow, data, cores=""):
        d = "/run/mpfree-devtest"
        return (f"play --script {d}/workflows/{workflow}.mpw --kits /run/mpfree-kits/kits.bin "
                f"--data {d}/data-{data} --gain-db -12" + (f" --sound-cores {cores}" if cores else ""))

    def test_plays_every_workflow_then_heavy_load_on_each_sound_core_setting(self):
        res = self.run_helper()
        self.assertEqual(res.returncode, 0, res.stdout)
        for text in ("static ARM EABI5 hard-float", "beat: about 2 s", "heavy-load: about 30 s", "CHECKS-OK",
                     "kits: at /run/mpfree-kits/kits.bin",
                     "copied; the program's and 3 workflows' hashes on the MPC match",
                     "----- workflow beat (sound cores: default) -----", 'script: "beat", 3 steps, 2.0 s',
                     "----- end of workflow songs (sound cores: default): exit code 0 -----",
                     "your answer: fine", "----- workflow heavy-load (sound cores: three) -----",
                     "pad player: active (started again)", "log display: active (started again)",
                     "RESULT: 7 runs played."):
            self.assertIn(text, res.stdout)
        self.assertEqual(self.lines("mpfree: "), [
            self.run_line("beat", "beat"), self.run_line("heavy-load", "heavy-load"), self.run_line("songs", "songs"),
            self.run_line("heavy-load", "heavy-load-shared", "shared"),
            self.run_line("heavy-load", "heavy-load-core3", "core3"),
            self.run_line("heavy-load", "heavy-load-two", "two"),
            self.run_line("heavy-load", "heavy-load-three", "three")])
        # Services: only our own pad player and log display are stopped, and started again after.
        changes = [c for c in self.lines("systemctl: ") if not c.startswith("is-active ")]
        self.assertEqual(changes, ["stop mpfree-test-app.service", "stop mpfree-test-screen.service",
                                   "start mpfree-test-app.service", "start mpfree-test-screen.service"])
        # Everything it wrote is in /run/mpfree-devtest; the kits and partition 4 are as they were.
        with open(self.program, "rb") as a, open(f"{self.root}/run/mpfree-devtest/mpfree", "rb") as b:
            self.assertEqual(hashlib.sha256(a.read()).digest(), hashlib.sha256(b.read()).digest())
        self.assertEqual(sorted(os.listdir(f"{self.root}/run")), ["mpfree-data", "mpfree-devtest", "mpfree-kits"])
        self.assertEqual(os.listdir(f"{self.root}/run/mpfree-data/projects"), ["mine.mpfp"])
        self.assertEqual(sorted(os.listdir(f"{self.root}/run/mpfree-devtest/workflows")),
                         ["beat.mpw", "heavy-load.mpw", "songs.mpw"])
        with open(self.report) as fh:
            self.assertIn("RESULT: 7 runs played.", fh.read())

    def test_a_failing_workflow_is_reported_and_the_rest_still_play(self):
        res = self.run_helper(FAKE_FAIL="beat,heavy-load")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("end of workflow beat (sound cores: default): exit code 3", res.stdout)
        self.assertIn("FAILED: beat (exit code 3), heavy-load (exit code 3), heavy-load on shared (exit code 3)",
                      res.stdout)
        self.assertEqual(len(self.lines("mpfree: ")), 7)
        self.assertIn("pad player: active (started again)", res.stdout)

    def test_only_names_the_workflows_and_no_cores_skips_the_settings(self):
        res = self.run_helper(answers="\n\n\n", extra=("--no-cores",), MPFREE_DEVTEST_ONLY="songs")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertEqual(self.lines("mpfree: "), [self.run_line("songs", "songs")])
        self.assertNotIn("[5/5]", res.stdout)
        res = self.run_helper(MPFREE_DEVTEST_ONLY="nothing")
        self.assertEqual(res.returncode, 1)
        self.assertIn("names no workflow called nothing", res.stdout)

    def test_s_at_the_last_part_skips_the_settings(self):
        res = self.run_helper(answers="\n\n\ns\n")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertEqual(len(self.lines("mpfree: ")), 3)

    def test_needs_the_opt_in_the_kits_and_the_test_build(self):
        res = self.run_helper(MPFREE_ACCEPT_DEVTEST="")
        self.assertEqual(res.returncode, 1)
        self.assertIn("opt-in required", res.stdout)
        os.remove(f"{self.root}/run/mpfree-kits/kits.bin")
        res = self.run_helper()
        self.assertEqual(res.returncode, 1)
        self.assertIn("the stick's kits are not at /run/mpfree-kits/kits.bin", res.stdout)
        th.write(f"{self.root}/run/mpfree-kits/kits.bin", b"MPFKITS1")
        th.write(self.stage, "36\n")
        res = self.run_helper()
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("the test build did not answer", res.stdout)
        self.assertEqual(self.lines("mpfree: "), [])
        self.assertFalse(os.path.exists(f"{self.root}/run/mpfree-devtest"))

    def test_stops_on_a_bad_program_or_workflow_and_q_before_the_copy(self):
        th.write(self.program, th.arm_elf(interp=True), 0o755)
        res = self.run_helper()
        self.assertEqual(res.returncode, 1)
        self.assertIn("dynamically linked", res.stdout)
        self.assertEqual(self.lines("ssh["), [])
        th.write(self.program, th.arm_elf(), 0o755)
        th.write(os.path.join(self.flows, "Bad Name.mpw"), "title x\n")
        res = self.run_helper()
        self.assertIn("a workflow's name is lower-case letters", res.stdout)
        os.remove(os.path.join(self.flows, "Bad Name.mpw"))
        th.write(os.path.join(self.flows, "odd.mpw"), "title x\nwait soon\n")
        res = self.run_helper()
        self.assertIn("a workflow's wait line does not read", res.stdout)
        os.remove(os.path.join(self.flows, "odd.mpw"))
        res = self.run_helper(answers="q\n")
        self.assertEqual(res.returncode, 1)
        self.assertIn("STOP: stopped by you", res.stdout)
        self.assertFalse(os.path.exists(f"{self.root}/run/mpfree-devtest"))
        self.assertEqual([c for c in self.lines("systemctl: ") if not c.startswith("is-active ")], [])


if __name__ == "__main__":
    unittest.main(verbosity=2)
