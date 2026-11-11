#!/usr/bin/env python3
"""Tests for host_app_start_capture.py (collect-over-ssh.sh --app-start-capture) against a fake stock 3.6. No device.

The fake ssh runs the tool's real remote scripts under BusyBox applets (as on
the MPC) against a fake root folder, setting the scripts' R='' root prefix to
it. It refuses the test build's host key alias, as the real stock host key
does. Two things change: `systemctl` is a stand-in that logs what it is asked,
and the capture's `cat` of the usbmon reader is replaced by a generator that
writes usbmon events (binary or text) the way the kernel does, including a
startup burst once the stand-in systemctl has restarted the app.

Set MPFREE_REQUIRE_BUSYBOX=1 (CI does) to fail instead of skip without BusyBox.
"""
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
TOOLS = os.path.dirname(HERE)
HELPER = os.path.join(TOOLS, "host_app_start_capture.py")
WRAPPER = os.path.join(TOOLS, "collect-over-ssh.sh")
STOCK = "6.6.95-az01-2025-06-27-rt55"
APPLETS = ("sh", "cat", "awk", "sleep", "kill")

FAKE_SSH = r'''#!/usr/bin/env python3
import os, subprocess, sys
args = sys.argv[1:]
opts, i = [], 0
while i < len(args) and args[i].startswith("-"):
    if args[i] in ("-o", "-i", "-p", "-l", "-F"):
        opts.append(args[i + 1]); i += 2
    else:
        i += 1
command = args[-1]
with open(os.environ["FAKE_LOG"], "a") as fh:
    fh.write(f"ssh: {command}\n")
if any(o.startswith("HostKeyAlias=") for o in opts):
    print("Host key verification failed.", file=sys.stderr)
    sys.exit(255)
root = os.environ["FAKE_ROOT"]
env = dict(os.environ, PATH=os.environ["FAKE_REMOTE_PATH"])
sh = os.path.join(os.environ["FAKE_REMOTE_PATH"], "sh")
script = sys.stdin.read() if command == "sh -s" else command
assert script.startswith("R=''"), "every remote script starts with R=''"
script = f"R='{root}'" + script[len("R=''"):]
reader = 'cat "$R$src" &'
if reader in script:
    script = script.replace(reader, '"$FAKE_USBMON" "$R$src" &')
import signal
proc = subprocess.Popen([sh, "-c", script], env=env, start_new_session=True)
def stop(signum, frame):  # like a closed SSH connection: the remote reader goes away with it
    os.killpg(proc.pid, signal.SIGTERM)
    sys.exit(143)
signal.signal(signal.SIGTERM, stop)
sys.exit(proc.wait())
'''

FAKE_SYSTEMCTL = r'''#!/bin/sh
echo "systemctl: $*" >> "$FAKE_LOG"
case "$1" in
  is-active)
    case "$2" in
      acvs.service) cat "$FAKE_ACVS" ;;
      az01-script-runner.service) echo active ;;
      *) echo inactive ;;
    esac ;;
  show) [ "$2" = acvs.service ] && echo "MainPID=$(cat "$FAKE_PID")" ;;
  restart)
    [ "$2" = acvs.service ] || { echo "refusing $*" >&2; exit 99; }
    echo 412 > "$FAKE_PID"
    echo restarted > "$FAKE_RESTARTED"
    [ -n "$FAKE_NEW_DEVNUM" ] && echo "$FAKE_NEW_DEVNUM" > "$FAKE_DEVNUM_FILE"
    cat "$FAKE_ACVS_AFTER" > "$FAKE_ACVS" ;;
  *) echo "systemctl stand-in: refusing $*" >&2; exit 99 ;;
esac
'''

# Writes usbmon events like the kernel: binary (48-byte header + data) or text lines.
FAKE_USBMON = r'''#!/usr/bin/env python3
import os, struct, sys, time
src = sys.argv[1]
binary = "/dev/usbmon" in src
out = sys.stdout.buffer
n = [0]
def usb_midi(cable, msg):
    """USB-MIDI 1.0 event packets for one message."""
    if msg[0] == 0xF0:
        pkts, i = b"", 0
        while i < len(msg):
            part = msg[i:i + 3]
            i += 3
            last = i >= len(msg)
            cin = {1: 0x5, 2: 0x6, 3: 0x7}[len(part)] if last else 0x4
            pkts += bytes([cable << 4 | cin]) + part + bytes(3 - len(part))
        return pkts
    return bytes([cable << 4 | msg[0] >> 4]) + msg + bytes(3 - len(msg))
def event(kind, ep, data, length=None):
    n[0] += 1
    ts = 1000 + n[0] / 100
    length = len(data) if length is None else length
    if binary:
        hdr = struct.pack("<QBBBBHBBqiiII8s", n[0], ord(kind), 3, ep, 2, 1, ord("-"),
                          0 if data else ord("<"), int(ts), int(ts % 1 * 1e6), 0, length, len(data), bytes(8))
        out.write(hdr + data)
    else:
        d = "i" if ep & 0x80 else "o"
        words = " ".join(data[i:i + 4].hex() for i in range(0, min(len(data), 32), 4))
        tail = f" = {words}" if data else " <"
        out.write(f"dd65f000 {int(ts * 1e6)} {kind} B{d}:1:002:{ep & 0x7f} 0 {length}{tail}\n".encode())
    out.flush()
ping = bytes.fromhex("f0 47 00 46 52 00 04 7d 6d 57 35 f7")
reply = bytes.fromhex("f0 47 7f 46 53 00 05 7d 6d 57 35 01 f7")
init = bytes.fromhex("f0 47 7f 46 62 00 01 01 f7")
leds = b"".join(usb_midi(1, bytes([0xB0, b, 3])) for b in range(30, 42))
try:
    sent_init = False
    while True:
        event("S", 0x01, usb_midi(0, ping))
        event("C", 0x81, usb_midi(1, reply))
        if os.path.exists(os.environ["FAKE_RESTARTED"]) and not sent_init:
            event("S", 0x01, usb_midi(0, init) + leds)  # 60 bytes in one transfer
            event("C", 0x81, usb_midi(1, bytes([0x99, 0x25, 0x40])) + usb_midi(1, bytes([0xA9, 0x25, 0x20])))
            sent_init = True
        time.sleep(0.05)
except (BrokenPipeError, OSError):
    pass
'''


def write(path, data, mode=0o644):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as fh:
        fh.write(data)
    os.chmod(path, mode)


class AppStartCaptureTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.busybox = shutil.which("busybox")
        if not cls.busybox:
            if os.environ.get("MPFREE_REQUIRE_BUSYBOX") == "1":
                raise RuntimeError("BusyBox is required for these tests")
            raise unittest.SkipTest("BusyBox not installed")

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="mpfree-appcap-test-")
        t = self.tmp
        self.log, self.root, self.report = os.path.join(t, "log"), os.path.join(t, "root36"), os.path.join(t, "r.txt")
        self.acvs, self.acvs_after, self.pid = os.path.join(t, "acvs"), os.path.join(t, "acvs-after"), os.path.join(t, "pid")
        self.restarted = os.path.join(t, "restarted")
        write(self.log, "")
        write(self.acvs, "active\n")
        write(self.acvs_after, "active\n")
        write(self.pid, "311\n")
        fakebin, remote = os.path.join(t, "fakebin"), os.path.join(t, "remote-bin")
        write(os.path.join(fakebin, "ssh"), FAKE_SSH, 0o755)
        os.makedirs(remote)
        for name in APPLETS:
            os.symlink(self.busybox, os.path.join(remote, name))
        write(os.path.join(remote, "systemctl"), FAKE_SYSTEMCTL, 0o755)
        usbmon = os.path.join(t, "usbmon-standin")
        write(usbmon, FAKE_USBMON.replace("#!/usr/bin/env python3", f"#!{sys.executable}", 1), 0o755)
        self.env = {"PATH": fakebin + os.pathsep + os.environ.get("PATH", "/usr/bin:/bin"),
                    "HOME": t, "MPFREE_APP_CAPTURE_TEST": "1", "FAKE_LOG": self.log, "FAKE_ROOT": self.root,
                    "FAKE_REMOTE_PATH": remote, "FAKE_USBMON": usbmon, "FAKE_ACVS": self.acvs,
                    "FAKE_ACVS_AFTER": self.acvs_after, "FAKE_PID": self.pid, "FAKE_RESTARTED": self.restarted,
                    "FAKE_NEW_DEVNUM": "", "FAKE_DEVNUM_FILE": os.path.join(self.root, "sys/bus/usb/devices/1-1/devnum")}
        self.key = os.path.join(t, "key")
        write(self.key, "fake key\n", 0o600)
        self.make_root()

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def make_root(self, release=STOCK, usbid="09e8:0046", binary=True, drive=False):
        r = self.root
        shutil.rmtree(r, ignore_errors=True)
        write(f"{r}/proc/sys/kernel/osrelease", release + "\n")
        write(f"{r}/proc/asound/card0/usbid", usbid + "\n")
        write(f"{r}/proc/asound/card0/usbbus", "001/002\n")
        write(f"{r}/proc/mounts", "/dev/root / ext4 ro 0 0\ndebugfs /sys/kernel/debug debugfs rw 0 0\n")
        for name, value in (("busnum", "1"), ("devnum", "2"), ("idVendor", "09e8"), ("idProduct", "0046"),
                            ("bcdDevice", "0010")):
            write(f"{r}/sys/bus/usb/devices/1-1/{name}", value + "\n")
        write(f"{r}/sys/bus/usb/devices/usb1/busnum", "1\n")
        write(f"{r}/sys/bus/usb/devices/usb1/devnum", "1\n")
        os.makedirs(f"{r}/sys/block/mmcblk0")
        if drive:
            os.makedirs(f"{r}/sys/block/sda")
        if binary:
            os.makedirs(f"{r}/dev")
            os.symlink("/dev/null", f"{r}/dev/usbmon1")  # a character device, as the real one is
        write(f"{r}/sys/kernel/debug/usb/usbmon/1u", "")

    def run_helper(self, answers="\nRESTART\n\n\nyes, pads lit\n", **env):
        cmd = [sys.executable, "-I", HELPER, "--ip", "192.0.2.141", "--key", self.key, "--report", self.report]
        return subprocess.run(cmd, input=answers, env=dict(self.env, **env), text=True, stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, timeout=120)

    def systemctl(self):
        with open(self.log) as fh:
            return [ln[len("systemctl: "):] for ln in fh.read().splitlines() if ln.startswith("systemctl: ")]

    def test_binary_capture_records_the_startup_messages(self):
        res = self.run_helper()
        self.assertEqual(res.returncode, 0, res.stdout)
        out = res.stdout
        for text in (f"release: {STOCK}", "CHECKS-OK", "capture: binary /dev/usbmon1",
                     "usb device: 1-1 09e8:0046 bcdDevice 0010 devnum 2",
                     "systemctl restart acvs.service: exit code 0", "Akai's app: running again (process 311 before, 412 now)",
                     "controller: the same USB device as before (1-1 09e8:0046 bcdDevice 0010 devnum 2)",
                     "your answer: yes, pads lit", "RESULT: all steps ran."):
            self.assertIn(text, out)
        # The whole 60-byte transfer is kept: the init SysEx and all twelve LED messages after it.
        order = out[out.index("EVERY MESSAGE FROM THE RESTART ON"):]
        self.assertIn("OUT cable0 sysex len=9 [f0 47 7f 46 62 00 01 01 f7]", order)
        self.assertIn("OUT cable1 cc ch=1 41 3 [b0 29 03]", order)
        self.assertIn("IN  cable1 note_on ch=10 37 64 [99 25 40]", order)
        self.assertNotIn("[a9 25 20]", order)  # pad pressure left out of the ordered list
        self.assertNotIn("cut short", out)
        self.assertEqual([c for c in self.systemctl() if c.startswith("restart")], ["restart acvs.service"])
        with open(self.report) as fh:
            self.assertIn("EVERY CAPTURED USB TRANSFER", fh.read())

    def test_text_capture_when_there_is_no_binary_reader(self):
        self.make_root(binary=False)
        res = self.run_helper()
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("capture: text /sys/kernel/debug/usb/usbmon/1u", res.stdout)
        self.assertIn("OUT cable0 sysex len=9 [f0 47 7f 46 62 00 01 01 f7]", res.stdout)
        self.assertIn("transfers were cut short by the capture (text mode keeps 32 bytes of each)", res.stdout)

    def test_refuses_anything_but_ready_stock(self):
        cases = ((dict(release="6.18.44-az01-2026-08-10-rt6"), "this is not stock 3.6"),
                 (dict(usbid="0483:df11"), "card 0 is not the controller in its normal MIDI mode"),
                 (dict(drive=True), "a USB drive is attached (sda): take it out first"))
        for kwargs, problem in cases:
            with self.subTest(problem=problem):
                self.make_root(**kwargs)
                write(self.log, "")
                res = self.run_helper()
                self.assertEqual(res.returncode, 1, res.stdout)
                self.assertIn(f"PROBLEM: {problem}", res.stdout)
                self.assertIn("STOP: the checks did not pass; nothing was started or changed", res.stdout)
                self.assertEqual(self.systemctl()[:1], ["is-active acvs.service"])
                self.assertFalse([c for c in self.systemctl() if c.startswith("restart")])
        write(self.acvs, "inactive\n")
        self.make_root()
        res = self.run_helper()
        self.assertIn("PROBLEM: Akai's app is not running", res.stdout)

    def test_restart_needs_the_typed_word(self):
        res = self.run_helper("\nrestart\nyes\nq\n")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("Please type RESTART exactly, or q.", res.stdout)
        self.assertIn("STOP: stopped by you; Akai's app was not restarted", res.stdout)
        self.assertIn("Akai's app was not restarted.", res.stdout)
        self.assertFalse([c for c in self.systemctl() if c.startswith("restart")])
        res = self.run_helper("q\n")
        self.assertIn("STOP: stopped by you; nothing was started", res.stdout)
        with open(self.log) as fh:
            self.assertNotIn('"$R$src"', fh.read())

    def test_reports_a_changed_controller_or_a_missing_app(self):
        write(self.acvs_after, "failed\n")
        res = self.run_helper(FAKE_NEW_DEVNUM="3")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("Akai's app: NOT RUNNING", res.stdout)
        self.assertIn("switch the MPC off with its power switch and on again", res.stdout)
        self.assertIn("WARNING: the controller changed", res.stdout)

    def test_the_test_build_is_refused(self):
        # The tool uses only the stock host key; the fake ssh refuses any HostKeyAlias, as for the test build.
        with open(HELPER) as fh:
            src = fh.read()
        self.assertNotIn("test=True", src)
        self.assertIn("kexec.Mpc(args, os.devnull)", src)


class StaticTests(unittest.TestCase):
    def test_one_state_changing_command(self):
        sys.path.insert(0, TOOLS)
        try:
            import importlib.util
            spec = importlib.util.spec_from_file_location("appcap", HELPER)
            mod = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(mod)
        finally:
            sys.path.pop(0)
        scripts = {"CHECK": mod.CHECK, "IDENTITY": mod.IDENTITY, "CAPTURE": mod.CAPTURE, "AFTER": mod.AFTER}
        for name, script in scripts.items():
            text = script.replace("2>/dev/null", "").replace(">&2", "")
            for word in (r"\brestart\b", r"\bstop\b", r"\bstart\b", r"kill -9", r"\brm\b", r" > ", r">>",
                         r"\bmount\b", r"\bdd\b", r"\bmkdir\b", r"\btee\b"):
                self.assertIsNone(re.search(word, text), f"{name}: {word}")
        commands = [ln for ln in mod.RESTART.splitlines()[1:] if not ln.startswith("echo ")]
        self.assertEqual(commands, ["systemctl restart acvs.service"])

    def test_wrapper_needs_the_opt_in(self):
        env = dict(os.environ, MPFREE_ACCEPT_APP_RESTART="")
        res = subprocess.run(["bash", WRAPPER, "--app-start-capture"], input="", env=env, text=True,
                             stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=30)
        self.assertNotEqual(res.returncode, 0)
        self.assertIn("--app-start-capture", res.stdout)


if __name__ == "__main__":
    unittest.main()
