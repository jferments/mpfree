#!/usr/bin/env python3
"""Tests for host_streamtest_mpc.py (D3) against a fake test build. No device.

As in test_host_devtest_mpc.py, a fake ssh runs the tool's real remote scripts under BusyBox applets against a
fake root folder (setting the scripts' R='' root prefix to it), accepts only the test build's host key alias, and
only while the fake MPC is "on the test build". Partition 4 is a folder in the fake root that /proc/mounts names
as /dev/sda4, ext4, with a /sys entry on a USB disk; its free space is this machine's, so the tests ask for 1 KiB
of room (MPFREE_STREAMTEST_NEED_KIB, read only under the tests' MPFREE_APP_TEST=1), or for more than any disk
has. A shell stand-in for mpfree logs its arguments, writes a take and a project into its data folder, and
prints what the real one would.

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
HELPER = os.path.join(TOOLS, "host_streamtest_mpc.py")
_spec = importlib.util.spec_from_file_location("test_app_mpc", os.path.join(HERE, "test_app_mpc.py"))
th = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(th)

FAKE_SSH = th.FAKE_SSH.replace(
    "program = 'exec \"$R/run/mpfree-app/mpfree\"'",
    "program = 'exec \"$R/run/mpfree-streamtest/mpfree\"'")
assert FAKE_SSH != th.FAKE_SSH, "the fake ssh's program line moved in test_app_mpc.py"

# Stands in for mpfree play --script: logs its arguments (with paths relative to the fake root), checks the
# workflow and an empty data folder are where the tool put them, writes what a run writes there (a take in the
# library's work folder, a saved project) and prints a workflow's lines.
FAKE_MPFREE = r'''#!/bin/sh
echo "mpfree: $*" | sed "s|$FAKE_ROOT||g" >> "$FAKE_LOG"
[ "$1" = play ] && [ "$2" = --script ] || { echo "STOP: not a script run"; exit 1; }
[ -f "$3" ] || { echo "STOP: no workflow $3"; exit 1; }
[ -d "$7" ] && [ -z "$(ls -A "$7")" ] || { echo "STOP: no empty data folder"; exit 1; }
name=$(basename "$3" .mpw)
mkdir -p "$7/library/.work/audiotracks" "$7/library/Projects/Night Market/samples"
echo take > "$7/library/.work/audiotracks/Audio 1_0123456789abcdef0123456789abcdef.wav"
echo saved > "$7/library/Projects/Night Market/samples/Audio 1_0123456789abcdef0123456789abcdef.wav"
echo "script: \"$name\", 3 steps, 2.0 s"
echo "audio tracks: takes play from their files (the script needs stream-audio, for this run), through a 21 MB cache"
echo "script: line 2 at 0.000 s: mark start"
echo "status: 0 hits (0 from the screen), 0 repeats, 0 underruns in 700 periods, at most 0 sounds at once"
echo "status: audio tracks: 8 takes, 0 of 60 s used, 0 refused, 0 dropped, 0 cut short; from their files: 8 of 8 takes, 0.00 s went silent, 8 of 8 region starts held"
echo "script: done"
echo "status: 0 hits (0 from the screen), 0 repeats, 0 underruns in 1400 periods, at most 0 sounds at once"
echo "status: audio tracks: 8 takes, 0 of 60 s used, 0 refused, 0 dropped, 0 cut short; from their files: 8 of 8 takes, 0.00 s went silent, 8 of 8 region starts held"
case ",$FAKE_FAIL," in *",$name,"*) exit 3 ;; esac
exit 0
'''



class StreamtestTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.busybox = shutil.which("busybox")
        if not cls.busybox:
            if os.environ.get("MPFREE_REQUIRE_BUSYBOX") == "1":
                raise RuntimeError("BusyBox is required for these tests")
            raise unittest.SkipTest("BusyBox not installed")

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="mpfree-streamtest-test-")
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
                    "MPFREE_APP_TEST": "1", "MPFREE_ACCEPT_STREAMTEST": "YES", "FAKE_STAGE": self.stage,
                    "FAKE_LOG": self.log, "FAKE_ROOT": self.root, "FAKE_REMOTE_PATH": remote, "FAKE_SCREEN": screen,
                    "FAKE_APP": app, "FAKE_ACVS": "inactive", "FAKE_PROGRAM": standin, "FAKE_FAIL": "",
                    "MPFREE_STREAMTEST_NEED_KIB": "1"}
        self.key = os.path.join(t, "key")
        th.write(self.key, "fake key\n", 0o600)
        self.program = os.path.join(t, "build", "mpfree")
        th.write(self.program, th.arm_elf(), 0o755)
        self.flows = os.path.join(t, "workflows")
        for name, body in (("eight-tracks", "title Eight\nneeds demo-songs, stream-audio\nwait 120\nwait 60\n"),
                           ("seek", "title Seek\nneeds stream-audio\nwait 30\n")):
            th.write(os.path.join(self.flows, f"{name}.mpw"), body)
        folder = os.path.join(self.work, "kexec-stick-20260101T000000Z-abc")
        th.write(os.path.join(folder, "stick.img"), b"\x00" * 4096)
        th.write(os.path.join(folder, "known_hosts"), "mpfree-kexec-test ssh-ed25519 AAAAfake\n")
        th.write(os.path.join(folder, "report.txt"), f"its kernel release: {th.NEW}\n")
        r = self.root
        th.write(f"{r}/proc/sys/kernel/osrelease", th.NEW + "\n")
        th.write(f"{r}/proc/sys/kernel/kexec_load_disabled", "1\n")
        os.makedirs(f"{r}/sys/block/sda")
        self.mounts("/dev/sda4 /run/mpfree-data ext4 rw,noatime 0 0")
        disk = f"{r}/sys/devices/platform/ff500000.usb/usb1/1-1/1-1:1.0/host0/target0:0:0/0:0:0:0/block/sda"
        th.write(f"{disk}/sda4/partition", "4\n")
        th.write(f"{disk}/sda1/partition", "1\n")
        os.makedirs(f"{r}/sys/class/block")
        for part in ("sda1", "sda4"):
            os.symlink(f"../../devices/platform/ff500000.usb/usb1/1-1/1-1:1.0/host0/target0:0:0/0:0:0:0/block/sda/{part}",
                       f"{r}/sys/class/block/{part}")
        th.write(f"{r}/run/mpfree-kits/kits.bin", b"MPFKITS1" + b"\x00" * 64)
        th.write(f"{r}/run/mpfree-data/projects/mine.mpfp", "a project of yours\n")
        th.write(f"{r}/run/mpfree-data/library/kick.wav", "a sound of yours\n")

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def mounts(self, data_line):
        th.write(f"{self.root}/proc/mounts", "/dev/root / ext4 ro,relatime 0 0\n"
                 "tmpfs /run tmpfs rw,nosuid,nodev,mode=755 0 0\n" + (data_line + "\n" if data_line else ""))

    def run_helper(self, answers="\n\nfine\n", extra=(), **env):
        cmd = [sys.executable, "-I", HELPER, "--ip", "192.0.2.141", "--key", self.key, "--work", self.work,
               "--report", self.report, "--program", self.program, "--workflows", self.flows, *extra]
        return subprocess.run(cmd, input=answers, env=dict(self.env, **env), text=True, stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, timeout=120)

    def lines(self, prefix):
        with open(self.log) as fh:
            return [ln[len(prefix):].strip() for ln in fh.read().splitlines() if ln.startswith(prefix)]

    def run_line(self, workflow):
        return (f"play --script /run/mpfree-streamtest/workflows/{workflow}.mpw --kits /run/mpfree-kits/kits.bin "
                f"--data /run/mpfree-data/mpfree-streamtest/data-{workflow} --gain-db -12")

    def mine_as_they_were(self):
        """Partition 4 holds only what it held before the run; RAM holds the program and workflows."""
        r = self.root
        self.assertEqual(sorted(os.listdir(f"{r}/run/mpfree-data")), ["library", "projects"])
        self.assertEqual(os.listdir(f"{r}/run/mpfree-data/library"), ["kick.wav"])
        self.assertEqual(os.listdir(f"{r}/run/mpfree-data/projects"), ["mine.mpfp"])

    def test_plays_every_workflow_with_its_data_on_partition_4_and_deletes_it_after(self):
        res = self.run_helper()
        self.assertEqual(res.returncode, 0, res.stdout)
        for text in ("static ARM EABI5 hard-float", "eight-tracks: about 180 s", "seek: about 30 s", "CHECKS-OK",
                     "kits: at /run/mpfree-kits/kits.bin",
                     "partition 4: /dev/sda4 at /run/mpfree-data, ext4, rw,noatime",
                     "partition 4: /dev/sda4 is on a USB disk", "KiB free", "DATA-OK",
                     "copied; the program's and 2 workflows' hashes on the MPC match",
                     "----- workflow eight-tracks (sound cores: default) -----", 'script: "eight-tracks", 3 steps',
                     "from their files: 8 of 8 takes, 0.00 s went silent",
                     "----- end of workflow eight-tracks (sound cores: default): exit code 0 -----",
                     "data folder: deleted; partition 4: ",
                     "----- end of workflow seek (sound cores: default): exit code 0 -----",
                     "your answer: fine",
                     "/run/mpfree-data/mpfree-streamtest: deleted; partition 4: ",
                     "pad player: active (started again)", "log display: active (started again)",
                     "RESULT: 2 workflows played."):
            self.assertIn(text, res.stdout)
        self.assertNotIn("left from a run that was cut off", res.stdout)
        self.assertEqual(self.lines("mpfree: "), [self.run_line("eight-tracks"), self.run_line("seek")])
        # Services: only our own pad player and log display are stopped, and started again after.
        changes = [c for c in self.lines("systemctl: ") if not c.startswith("is-active ")]
        self.assertEqual(changes, ["stop mpfree-test-app.service", "stop mpfree-test-screen.service",
                                   "start mpfree-test-app.service", "start mpfree-test-screen.service"])
        # RAM has the program and the workflows; partition 4 is as it was, the run's folder gone.
        with open(self.program, "rb") as a, open(f"{self.root}/run/mpfree-streamtest/mpfree", "rb") as b:
            self.assertEqual(hashlib.sha256(a.read()).digest(), hashlib.sha256(b.read()).digest())
        self.assertEqual(sorted(os.listdir(f"{self.root}/run")), ["mpfree-data", "mpfree-kits", "mpfree-streamtest"])
        self.mine_as_they_were()
        with open(self.report) as fh:
            self.assertIn("RESULT: 2 workflows played.", fh.read())

    def test_a_hands_off_round_plays_without_a_terminal_and_records_the_question_as_not_asked(self):
        res = self.run_helper(answers="", MPFREE_APP_TEST="", MPFREE_ROUND="1")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertEqual(res.stdout.count("(hands-off round: going on)"), 2, res.stdout)
        self.assertIn("Did anything click, drop out, stick or sound wrong while they played?", res.stdout)
        self.assertIn("(not asked: hands-off round)", res.stdout)
        self.assertNotIn("your answer:", res.stdout)
        self.assertIn("RESULT: 2 workflows played.", res.stdout)
        self.mine_as_they_were()

    def test_the_device_reports_reader_reads_its_runs(self):
        res = self.run_helper()
        self.assertEqual(res.returncode, 0, res.stdout)
        tool = os.path.join(TOOLS, "devtest_report.py")
        out = subprocess.run([sys.executable, "-I", tool, self.report], text=True, stdout=subprocess.PIPE,
                             stderr=subprocess.STDOUT, timeout=60)
        self.assertEqual(out.returncode, 0, out.stdout)
        self.assertIn("| eight-tracks | 0 | yes | 0 of 0 | 0 | 0 | 0 | 0 | 0.00 |", out.stdout)
        self.assertIn("| start | 0 | 700 | 0 | 0 | - | - | 8 of 8 | 0.00 |", out.stdout)

    def test_a_folder_a_cut_off_run_left_is_deleted_first(self):
        left = f"{self.root}/run/mpfree-data/mpfree-streamtest/data-eight-tracks/library"
        th.write(f"{left}/half a take.wav.recording", "cut off\n")
        res = self.run_helper()
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("is left from a run that was cut off (deleted before the first workflow)", res.stdout)
        self.assertEqual(len(self.lines("mpfree: ")), 2)
        self.mine_as_they_were()

    def test_a_failing_workflow_is_reported_its_folder_still_goes_and_the_rest_play(self):
        res = self.run_helper(FAKE_FAIL="eight-tracks")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("end of workflow eight-tracks (sound cores: default): exit code 3", res.stdout)
        self.assertIn("FAILED: eight-tracks (exit code 3)", res.stdout)
        self.assertEqual(len(self.lines("mpfree: ")), 2)
        self.assertIn("pad player: active (started again)", res.stdout)
        self.mine_as_they_were()

    def test_q_before_the_workflows_deletes_the_folder(self):
        res = self.run_helper(answers="\nq\n")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("STOP: stopped by you", res.stdout)
        self.assertIn("/run/mpfree-data/mpfree-streamtest: deleted", res.stdout)
        self.assertEqual(self.lines("mpfree: "), [])
        self.assertIn("pad player: active (started again)", res.stdout)
        self.mine_as_they_were()

    def test_only_names_the_workflows(self):
        res = self.run_helper(MPFREE_STREAMTEST_ONLY="seek")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertEqual(self.lines("mpfree: "), [self.run_line("seek")])

    def test_partition_4_must_be_the_sticks_ext4_read_write_with_room(self):
        for line, why in (("", "partition 4: not mounted at /run/mpfree-data"),
                          ("/dev/sda4 /run/mpfree-data vfat rw 0 0", "partition 4: not ext4"),
                          ("/dev/sda4 /run/mpfree-data ext4 ro,noatime 0 0", "partition 4: not read-write"),
                          ("/dev/sda1 /run/mpfree-data ext4 rw 0 0", "partition 4: /dev/sda1 is not partition 4"),
                          ("tmpfs /run/mpfree-data ext4 rw 0 0", "partition 4: tmpfs is not a /dev block device")):
            self.mounts(line)
            res = self.run_helper()
            self.assertEqual(res.returncode, 1, res.stdout)
            self.assertIn(why, res.stdout)
            self.assertIn("partition 4 is not as the workflows need it", res.stdout)
        self.mounts("/dev/sda4 /run/mpfree-data ext4 rw,noatime 0 0")
        os.remove(f"{self.root}/sys/class/block/sda4")
        os.symlink("../../devices/virtual/block/sda4", f"{self.root}/sys/class/block/sda4")
        th.write(f"{self.root}/sys/devices/virtual/block/sda4/partition", "4\n")
        res = self.run_helper()
        self.assertIn("partition 4: /dev/sda4 is not on a USB disk", res.stdout)
        os.remove(f"{self.root}/sys/class/block/sda4")
        os.symlink("../../devices/platform/ff500000.usb/usb1/1-1/1-1:1.0/host0/target0:0:0/0:0:0:0/block/sda/sda4",
                   f"{self.root}/sys/class/block/sda4")
        res = self.run_helper(MPFREE_STREAMTEST_NEED_KIB=str(1 << 50))
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn(f"partition 4: less than {1 << 50} KiB free", res.stdout)
        # Nothing was copied or made, and no service was touched.
        self.assertFalse(os.path.exists(f"{self.root}/run/mpfree-streamtest"))
        self.assertFalse(os.path.exists(f"{self.root}/run/mpfree-data/mpfree-streamtest"))
        self.assertEqual([c for c in self.lines("systemctl: ") if not c.startswith("is-active ")], [])
        self.assertEqual(self.lines("mpfree: "), [])

    def test_needs_the_opt_in_and_the_test_build(self):
        res = self.run_helper(MPFREE_ACCEPT_STREAMTEST="")
        self.assertEqual(res.returncode, 1)
        self.assertIn("opt-in required", res.stdout)
        th.write(self.stage, "36\n")
        res = self.run_helper()
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("the test build did not answer", res.stdout)
        self.assertEqual(self.lines("mpfree: "), [])
        self.assertFalse(os.path.exists(f"{self.root}/run/mpfree-data/mpfree-streamtest"))


if __name__ == "__main__":
    unittest.main(verbosity=2)
