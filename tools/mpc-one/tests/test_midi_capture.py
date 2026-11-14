#!/usr/bin/env python3
"""Tests for host_midi_capture.py (collect-over-ssh.sh --midi-capture) against a fake stock 3.6. No device.

The whole wrapper runs, with a pseudo-terminal as its input (it refuses to run
without one) and fake ip, ping and ssh. The fake ssh runs the real remote
script, passive-usbmon-capture.sh, under BusyBox applets (as on the MPC),
with its /proc paths moved into a fake root folder and its `cat` of the
usbmon reader replaced by a generator that writes usbmon text lines the way
the kernel does: a ping on cable 0, a note in on cable 2 and a note out on
cable 2.

Set MPFREE_REQUIRE_BUSYBOX=1 (CI does) to fail instead of skip without BusyBox.
"""
import glob
import importlib.util
import os
import pty
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
TOOLS = os.path.dirname(HERE)
HELPER = os.path.join(TOOLS, "host_midi_capture.py")
WRAPPER = os.path.join(TOOLS, "collect-over-ssh.sh")
REMOTE = os.path.join(TOOLS, "passive-usbmon-capture.sh")
APPLETS = ("sh", "cat", "awk", "sleep", "kill")

FAKE_SSH = r'''#!/usr/bin/env python3
import os, signal, subprocess, sys
args = sys.argv[1:]
command = args[-1]
with open(os.environ["FAKE_LOG"], "a") as fh:
    fh.write("ssh: " + command + "\n")
if command != "sh -s":
    print("fake ssh: command not allowed: " + command, file=sys.stderr)
    sys.exit(255)
root = os.environ["FAKE_ROOT"]
script = sys.stdin.read()
with open(os.environ["FAKE_LOG"], "a") as fh:
    fh.write(script)
reader = 'cat "$mon" &'
assert script.count(reader) == 1, "the remote script reads usbmon with one cat"
script = script.replace(reader, '"$FAKE_USBMON" "$mon" &').replace("/proc/", root + "/proc/")
env = dict(os.environ, PATH=os.environ["FAKE_REMOTE_PATH"])
proc = subprocess.Popen([os.path.join(os.environ["FAKE_REMOTE_PATH"], "sh"), "-c", script], env=env,
                        start_new_session=True)
def stop(signum, frame):  # like a closed SSH connection: the remote side goes away with it
    os.killpg(proc.pid, signal.SIGTERM)
    sys.exit(143)
signal.signal(signal.SIGTERM, stop)
sys.exit(proc.wait())
'''

FAKE_USBMON = r'''#!/usr/bin/env python3
import sys, time
n = [0]
def line(kind, d, ep, pkts):
    n[0] += 1
    words = " ".join(pkts[i:i + 4].hex() for i in range(0, len(pkts), 4))
    sys.stdout.write(f"dd65f000 {1000000 + n[0]} {kind} B{d}:1:002:{ep} 0 {len(pkts)} = {words}\n")
    sys.stdout.flush()
try:
    while True:
        line("S", "o", 1, bytes.fromhex("04f04700 04460000 07047d6d"))  # cable 0 out: part of a ping
        line("C", "i", 1, bytes.fromhex("29903c64"))  # cable 2 in: note on 60
        line("S", "o", 1, bytes.fromhex("29904050"))  # cable 2 out: note on 64
        time.sleep(0.05)
except (BrokenPipeError, OSError):
    pass
'''

TEST_IP, TEST_IFACE, TEST_HOST, TEST_MAC = "192.0.2.141", "mpfreetest0", "192.0.2.1/24", "02:00:00:00:00:01"
FAKE_IP = r'''#!/bin/sh
case "$*" in
  *"addr show dev mpfreetest0"*) echo "    inet 192.0.2.1/24 brd 192.0.2.255 scope global mpfreetest0" ;;
  *"route get 192.0.2.141"*) echo "192.0.2.141 dev mpfreetest0 src 192.0.2.1 uid 1000" ;;
  *"neigh show dev mpfreetest0"*) echo "192.0.2.141 lladdr 02:00:00:00:00:01 REACHABLE" ;;
  *"neigh get 192.0.2.141 dev mpfreetest0"*) echo "192.0.2.141 dev mpfreetest0 lladdr 02:00:00:00:00:01 REACHABLE" ;;
  *) exit 1 ;;
esac
'''


def write(path, data, mode=0o644):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as fh:
        fh.write(data)
    os.chmod(path, mode)


def load_helper():
    spec = importlib.util.spec_from_file_location("host_midi_capture", HELPER)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


class MidiCaptureWrapperTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.busybox = shutil.which("busybox")
        if not cls.busybox:
            if os.environ.get("MPFREE_REQUIRE_BUSYBOX") == "1":
                raise RuntimeError("BusyBox is required for these tests")
            raise unittest.SkipTest("BusyBox not installed")

    def setUp(self):
        self.tmp = t = tempfile.mkdtemp(prefix="mpfree-midicap-test-")
        self.log, self.root, self.reports = os.path.join(t, "log"), os.path.join(t, "root36"), os.path.join(t, "research")
        write(self.log, "")
        fakebin, remote = os.path.join(t, "fakebin"), os.path.join(t, "remote-bin")
        write(os.path.join(fakebin, "ssh"), FAKE_SSH.replace("#!/usr/bin/env python3", f"#!{sys.executable}", 1), 0o755)
        write(os.path.join(fakebin, "ip"), FAKE_IP, 0o755)
        write(os.path.join(fakebin, "ping"), "#!/bin/sh\nexit 0\n", 0o755)
        os.makedirs(remote)
        for name in APPLETS:
            os.symlink(self.busybox, os.path.join(remote, name))
        usbmon = os.path.join(t, "usbmon-standin")
        write(usbmon, FAKE_USBMON.replace("#!/usr/bin/env python3", f"#!{sys.executable}", 1), 0o755)
        r = self.root
        write(f"{r}/proc/asound/card0/usbid", "09e8:0046\n")
        write(f"{r}/proc/asound/card0/usbbus", "001/002\n")
        write(f"{r}/proc/mounts", f"/dev/root / ext4 ro 0 0\ndebugfs {r}/sys/kernel/debug debugfs rw 0 0\n")
        write(f"{r}/sys/kernel/debug/usb/usbmon/1u", "")
        key = os.path.join(t, "key")
        write(key, "not a real key\n", 0o600)
        self.env = {
            "HOME": t, "LC_ALL": "C", "PATH": fakebin + os.pathsep + "/usr/bin:/bin",
            "MPC_KEY": key, "MPC_REPORT_DIR": self.reports,
            "MPC_IP": TEST_IP, "MPC_IFACE": TEST_IFACE, "MPC_HOST_IP_CIDR": TEST_HOST, "MPC_EXPECT_MAC": TEST_MAC,
            "FAKE_LOG": self.log, "FAKE_ROOT": r, "FAKE_REMOTE_PATH": remote, "FAKE_USBMON": usbmon,
            "MPFREE_MIDI_CAPTURE_TEST": "1", "MPFREE_ACCEPT_USBMON": "YES",
        }

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def run_wrapper(self, answers, **extra):
        """The wrapper with a terminal as its input, as when the owner runs it."""
        master, slave = pty.openpty()
        try:
            proc = subprocess.Popen(["bash", WRAPPER, "--midi-capture"], env={**self.env, **extra}, stdin=slave,
                                    stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
            os.write(master, answers.encode())
            out, _ = proc.communicate(timeout=120)
        finally:
            os.close(slave)
            os.close(master)
        return proc.returncode, out

    def report(self):
        [path] = glob.glob(os.path.join(self.reports, "mpc-one-midi-capture-*.txt"))
        with open(path) as fh:
            return fh.read()

    def test_midi_cv_phases(self):
        # Enter after the setup list, then Enter for each of the six phases.
        rc, out = self.run_wrapper("\n" * 7, MPFREE_CAPTURE_PHASES="midi-cv")
        self.assertEqual(rc, 0, out)
        self.assertIn("Make a new CV track", out)
        self.assertIn("Do not save the project or change Preferences.", out)
        for name in ("idle", "din_in", "din_out", "cv_notes", "cv_hold", "cv_slider"):
            self.assertIn(f"] {name}: ", out)
        text = self.report()
        self.assertIn("phase set: midi-cv", text)
        self.assertIn("MESSAGES PER CABLE AND PHASE", text)
        self.assertRegex(text, r"  din_in: .*IN cable2 \d+ \([+-]\d+\)")
        self.assertIn("IN  cable2 note_on ch=1 60 100 [90 3c 64]", text)
        self.assertIn("OUT cable2 note_on ch=1 64 80 [90 40 50]", text)
        self.assertIn("END GUIDED PASSIVE USB-MIDI CAPTURE", text)
        with open(self.log) as fh:
            ssh = [ln for ln in fh.read().splitlines() if ln.startswith("ssh: ")]
        self.assertEqual(ssh, ["ssh: sh -s"])

    def test_cv_scale_phases(self):
        # Enter after the setup list, then Enter for each of the five phases.
        rc, out = self.run_wrapper("\n" * 6, MPFREE_CAPTURE_PHASES="cv-scale")
        self.assertEqual(rc, 0, out)
        self.assertIn("Note down exactly what its output field says.", out)
        self.assertIn("Do not save the project or change Preferences.", out)
        for name in ("idle", "cv_octaves", "cv_semitones", "cv_out2", "din_out"):
            self.assertIn(f"] {name}: ", out)
        text = self.report()
        self.assertIn("phase set: cv-scale", text)
        self.assertRegex(text, r"  cv_octaves: .*IN cable2 \d+ \([+-]\d+\)")

    def test_midi_out_phases(self):
        # Enter after the setup list, then Enter for each of the five phases.
        rc, out = self.run_wrapper("\n" * 6, MPFREE_CAPTURE_PHASES="midi-out")
        self.assertEqual(rc, 0, out)
        self.assertIn("Keep the MIDI track routed the way that made the synth play", out)
        self.assertIn("Do not save the project or change Preferences.", out)
        for name in ("idle", "out_keys", "out_hold", "out_pads", "out_clock"):
            self.assertIn(f"] {name}: ", out)
        text = self.report()
        self.assertIn("phase set: midi-out", text)
        self.assertRegex(text, r"  out_keys: .*IN cable2 \d+ \([+-]\d+\)")
        self.assertIn("OUT cable2 note_on ch=1 64 80 [90 40 50]", text)
        with open(self.log) as fh:
            ssh = [ln for ln in fh.read().splitlines() if ln.startswith("ssh: ")]
        self.assertEqual(ssh, ["ssh: sh -s"])

    def test_cv_ports_phases(self):
        # Enter after the setup list, then Enter for each of the four phases.
        rc, out = self.run_wrapper("\n" * 5, MPFREE_CAPTURE_PHASES="cv-ports")
        self.assertEqual(rc, 0, out)
        self.assertIn("Find its CV Port and Gate Port fields", out)
        self.assertIn("Do not save the project or change Preferences.", out)
        for name in ("idle", "cv_ports_5_6", "cv_ports_7_8", "cv_ports_1_2"):
            self.assertIn(f"] {name}: ", out)
        self.assertIn("phase set: cv-ports", self.report())
        with open(self.log) as fh:
            ssh = [ln for ln in fh.read().splitlines() if ln.startswith("ssh: ")]
        self.assertEqual(ssh, ["ssh: sh -s"])

    def test_transport_phases(self):
        # Enter after the setup list, then Enter for each of the five phases.
        rc, out = self.run_wrapper("\n" * 6, MPFREE_CAPTURE_PHASES="transport")
        self.assertEqual(rc, 0, out)
        self.assertIn("Stay on the Main page with the transport stopped.", out)
        self.assertIn("save the project or change Preferences", out)
        for name in ("idle", "play_stop", "rec_play", "overdub_play", "play_start"):
            self.assertIn(f"] {name}: ", out)
        self.assertIn("phase set: transport", self.report())
        with open(self.log) as fh:
            ssh = [ln for ln in fh.read().splitlines() if ln.startswith("ssh: ")]
        self.assertEqual(ssh, ["ssh: sh -s"])

    def test_default_is_the_controller_set(self):
        rc, out = self.run_wrapper("\n" * 10)
        self.assertEqual(rc, 0, out)
        self.assertNotIn("Make a new CV track", out)
        self.assertIn("] pad1_soft_hard: ", out)
        self.assertIn("phase set: controller", self.report())

    def test_stopping_at_the_setup_runs_nothing_on_the_mpc(self):
        master, slave = pty.openpty()
        os.close(master)  # end of input at the first question
        try:
            res = subprocess.run(["bash", WRAPPER, "--midi-capture"], stdin=slave, stdout=subprocess.PIPE,
                                 stderr=subprocess.STDOUT, text=True, timeout=60,
                                 env={**self.env, "MPFREE_CAPTURE_PHASES": "midi-cv"})
        finally:
            os.close(slave)
        self.assertNotEqual(res.returncode, 0, res.stdout)
        with open(self.log) as fh:
            self.assertNotIn("ssh:", fh.read())

    def test_wrapper_refuses_an_unknown_phase_set(self):
        rc, out = self.run_wrapper("\n" * 7, MPFREE_CAPTURE_PHASES="cv")
        self.assertNotEqual(rc, 0, out)
        self.assertIn("MPFREE_CAPTURE_PHASES must be controller, midi-cv, cv-scale, midi-out, cv-ports or "
                      "transport, not: cv", out)
        with open(self.log) as fh:
            self.assertNotIn("ssh:", fh.read())


class StaticTests(unittest.TestCase):
    def test_phase_sets(self):
        mod = load_helper()
        self.assertEqual(sorted(mod.PHASE_SETS),
                         ["controller", "cv-ports", "cv-scale", "midi-cv", "midi-out", "transport"])
        self.assertEqual(sorted(mod.SETUPS), ["cv-ports", "cv-scale", "midi-cv", "midi-out", "transport"])
        self.assertIs(mod.PHASE_SETS["controller"], mod.PHASES)
        for phases in (mod.MIDI_CV_PHASES, mod.CV_SCALE_PHASES, mod.MIDI_OUT_PHASES, mod.CV_PORTS_PHASES,
                       mod.TRANSPORT_PHASES):
            names = [n for n, _ in phases]
            self.assertEqual(names[0], "idle")  # the baseline the per-cable counts compare against
            self.assertEqual(len(names), len(set(names)))
            # The phases of 8 s plus the owner's Enter presses fit the remote script's 240 s window.
            self.assertLessEqual(len(names) * 8.0, 120)

    def test_cv_ports_are_named(self):
        mod = load_helper()
        # Cable 1 (packet header 0x1B: cable 1, CIN B), channel 16 (BF): a pitch on CC 8 and 9 (port 5)
        # and a gate on CC 10 and 11 (port 6). A button light (B0) and a pad note are not ports.
        lines = [("cv_ports_5_6", "x 1 S Bo:1:002:2 -115 16 = 1bbf0833 1bbf0919 1bbf0a7f 1bbf0b7f"),
                 ("cv_ports_5_6", "x 1 S Bo:1:002:2 -115 8 = 1bb04303 09903c64"),
                 ("cv_ports_1_2", "x 1 S Bo:1:002:2 -115 8 = 1bbf0033 1bbf0119")]
        text = mod.analyze(lines, 1, 2)
        self.assertIn("  cv_ports_5_6: port 5 (CC 8, 9) 1x, port 6 (CC 10, 11) 1x", text)
        self.assertIn("  cv_ports_1_2: port 1 (CC 0, 1) 1x", text)

    def test_analyze_counts_against_idle(self):
        mod = load_helper()
        note_in = "x 1 C Bi:1:002:1 0 4 = 29903c64"
        lines = [("idle", note_in), ("din_in", note_in), ("din_in", note_in), ("gap", note_in)]
        text = mod.analyze(lines, 1, 2)
        self.assertIn("  idle: IN cable2 1 (+0)", text)
        self.assertIn("  din_in: IN cable2 2 (+1)", text)
        self.assertNotIn("  gap:", text)


if __name__ == "__main__":
    unittest.main()
