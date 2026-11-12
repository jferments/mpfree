#!/usr/bin/env python3
"""Tests for host_app_mpc.py (collect-over-ssh.sh --app) against a fake test build. No device.

The fake ssh runs the tool's real remote scripts under BusyBox applets (as on
the MPC) against a fake root folder, setting the scripts' R='' root prefix to
it. It accepts only the test build's host key alias, and only while the fake
MPC is "on the test build", as the real host keys do. Running the program is
the one other change: the fake ssh runs a shell stand-in instead of the ARM
program, which logs its arguments and prints what the real one would.

The static tests also read the program's Rust source, to keep what it opens
for writing to a short reviewed list.

Set MPFREE_REQUIRE_BUSYBOX=1 (CI does) to fail instead of skip without BusyBox.
"""
import hashlib
import os
import pty
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
TOOLS = os.path.dirname(HERE)
REPO = os.path.dirname(os.path.dirname(TOOLS))
HELPER = os.path.join(TOOLS, "host_app_mpc.py")
WRAPPER = os.path.join(TOOLS, "collect-over-ssh.sh")
APP_SRC = os.path.join(REPO, "rust", "mpfree-app", "src")
UI_SRC = os.path.join(REPO, "rust", "mpfree-ui", "src")
# Library code from the other crates the program links (their own programs, main.rs and bin/, are not part of it).
LINKED_SRC = (os.path.join(REPO, "rust", "mpfree-touch", "src"), os.path.join(REPO, "rust", "mpfree-screen-log", "src"),
              os.path.join(REPO, "rust", "mpfree-prof", "src"), os.path.join(REPO, "rust", "mpfree-store", "src"))
# mpfree-prof/alloc.rs holds the counting allocator, the crate's only unsafe code. It compiles only with the
# crate's counting-alloc feature, which mpfree does not enable (mpfree-bench does), so it never
# reaches the pad player and is left out of the scan.
NOT_LINKED = ("mpfree-prof/alloc.rs",)
NEW = "6.18.44-fake-rt6"
APPLETS = ("sh", "cat", "grep", "readlink", "ls", "df", "tail", "sha256sum", "rm", "mkdir", "mv", "chmod", "sleep")

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
test = "HostKeyAlias=mpfree-kexec-test" in opts
with open(os.environ["FAKE_STAGE"]) as fh:
    stage = fh.read().strip()
with open(os.environ["FAKE_LOG"], "a") as fh:
    fh.write(f"ssh[{'test' if test else '3.6'}@{stage}]: {command}\n")
if not test or stage != "310":
    print("Host key verification failed.", file=sys.stderr)
    sys.exit(255)
root = os.environ["FAKE_ROOT"]
env = dict(os.environ, PATH=os.environ["FAKE_REMOTE_PATH"])
sh = os.path.join(os.environ["FAKE_REMOTE_PATH"], "sh")
script = sys.stdin.read() if command == "sh -s" else command
assert script.startswith("R=''"), "every remote script starts with R=''"
script = f"R='{root}'" + script[len("R=''"):]
program = 'exec "$R/run/mpfree-app/mpfree"'
if program in script:
    script = script.replace(program, 'exec "$FAKE_PROGRAM"')
sys.exit(subprocess.run([sh, "-c", script], env=env, stdin=None if command == "sh -s" else sys.stdin).returncode)
'''

FAKE_SYSTEMCTL = r'''#!/bin/sh
echo "systemctl: $*" >> "$FAKE_LOG"
case "$1" in
  is-active)
    case "$2" in
      mpfree-test-screen.service) cat "$FAKE_SCREEN" ;;
      mpfree-test-app.service) cat "$FAKE_APP" ;;
      acvs.service) echo "$FAKE_ACVS" ;;
      *) echo inactive ;;
    esac ;;
  stop)
    case "$2" in
      mpfree-test-screen.service) echo inactive > "$FAKE_SCREEN" ;;
      mpfree-test-app.service) echo inactive > "$FAKE_APP" ;;
    esac ;;
  start)
    case "$2" in
      mpfree-test-screen.service) echo active > "$FAKE_SCREEN" ;;
      mpfree-test-app.service) echo active > "$FAKE_APP" ;;
    esac ;;
  *) echo "systemctl stand-in: refusing $*" >&2; exit 99 ;;
esac
'''

# The pad player's log: every line of FAKE_JOURNAL, as journalctl -o short-monotonic prints them. Each
# call moves the fake clock (and /proc/uptime) on by 10 s first and adds the lines due by then
# (FAKE_JOURNAL.<n>), so a phase that starts at uptime n - 10 sees the lines of FAKE_JOURNAL.<n>.
FAKE_JOURNALCTL = r'''#!/bin/sh
echo "journalctl: $*" >> "$FAKE_LOG"
n=$(cat "$FAKE_CLOCK"); n=$((n + 10)); echo "$n" > "$FAKE_CLOCK"
echo "$n.00 1.00" > "$FAKE_ROOT/proc/uptime"
[ -f "$FAKE_JOURNAL.$n" ] && cat "$FAKE_JOURNAL.$n" >> "$FAKE_JOURNAL"
cat "$FAKE_JOURNAL"
'''

FAKE_PROGRAM = r'''#!/bin/sh
echo "program: $*" >> "$FAKE_LOG"
echo "===== mpfree $1 =====" >> "$FAKE_REPORT"
case "$1" in
  inventory) echo "audio card for the click: 1 (ACVA)"; echo "inventory done" ;;
  screen) echo "mpfree."; echo "screen: card drawn" ;;
  touch) echo "touch: 1 contact(s): slot 0: x 100 y 200"; echo "touch done: 40 events, 10 frames, 3 lines shown" ;;
  pads)
    case "$*" in *--keepalive*) echo "sent keep-alive 1"; h=$FAKE_PADS_KEEPALIVE ;; *) h=$FAKE_PADS ;; esac
    echo "pads done: 30 bytes, 10 events, $h pad hits, 0 keep-alives sent, 0 replies" ;;
  lights) echo "lights done: 90 messages sent, pad hits on Public 4, Private 0, MIDI Port 0" ;;
  setup) echo "setup done: 140 messages sent, pad hits on Public 0, Private 4, MIDI Port 0" ;;
  drums) if [ -f kits.bin ]; then echo "kit: 8 sounds"; else echo "STOP: no kits"; exit 1; fi
         echo "drums done: 12 pad hits played" ;;
  play) if [ -f kits.bin ]; then echo "pad player: 2 kits, 20 sounds"; else echo "STOP: no kits"; exit 1; fi
        echo "play done (STOP): 40 pad hits, 0 underruns" ;;
  click) echo "click done: 3445 periods (10.0 s), 0 underruns, 0 late periods"; exit "$FAKE_CLICK_RC" ;;
esac
'''


def write(path, data, mode=0o644):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb" if isinstance(data, bytes) else "w") as fh:
        fh.write(data)
    os.chmod(path, mode)


def arm_elf(machine=40, flags=0x05000400, interp=False):
    """A minimal 32-bit ELF header plus one program header (PT_LOAD, or PT_INTERP if asked)."""
    header = bytearray(52)
    header[:7] = b"\x7fELF\x01\x01\x01"
    struct.pack_into("<HHI", header, 16, 2, machine, 1)
    struct.pack_into("<I", header, 28, 52)          # e_phoff
    struct.pack_into("<I", header, 36, flags)
    struct.pack_into("<HHH", header, 40, 52, 32, 1)  # e_ehsize, e_phentsize, e_phnum
    ph = bytearray(32)
    struct.pack_into("<I", ph, 0, 3 if interp else 1)
    return bytes(header + ph) + os.urandom(4000)



def logical_lines(code):
    """Rust source with each statement that rustfmt wraps put back on one line, so the source checks
    below read what the code says, not how it is wrapped. A line joins the one before it while a
    ( or [ is open (but not into or out of a { } block, such as a closure's body), or when it starts
    a method call (.x) or ?, or is the lone { that rustfmt puts after a wrapped `if`.
    Wrapped argument lists lose rustfmt's trailing comma: `f(a, b,\n)` reads `f(a, b)`."""
    out, depth, in_str = [], 0, False
    for raw in code.splitlines():
        line = raw.strip()
        inside = depth > 0 and not out[-1].rstrip().endswith(("{", ";")) and not line.startswith("}") if out else False
        cont = out and (inside or line.startswith((".", "?")) or line == "{")
        if cont:
            prev = out[-1]
            glue = "" if prev.endswith(("(", "[")) or line.startswith((")", "]", ".", "?")) else " "
            out[-1] = prev + glue + line
        else:
            out.append(raw)
        i = 0  # in_str carries over: a string literal can run over several lines
        while i < len(line):
            c = line[i]
            if in_str:
                if c == "\\":
                    i += 1
                elif c == '"':
                    in_str = False
            elif c == '"':
                in_str = True
            elif c == "'" and line[i + 1:i + 2] == "\\":
                i = line.find("'", i + 3)  # an escaped char literal: '\'', '\n', '\u{..}'
                if i < 0:
                    break
            elif c == "'" and line[i + 2:i + 3] == "'":
                i += 2  # a char literal such as '(' or '"'
            elif line.startswith("//", i):
                break
            elif c in "([":
                depth += 1
            elif c in ")]":
                depth = max(0, depth - 1)
            i += 1
    return "\n".join(re.sub(r",([)\]])", r"\1", ln) for ln in out) + "\n"

class AppTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.busybox = shutil.which("busybox")
        if not cls.busybox:
            if os.environ.get("MPFREE_REQUIRE_BUSYBOX") == "1":
                raise RuntimeError("BusyBox is required for these tests")
            raise unittest.SkipTest("BusyBox not installed")

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="mpfree-test-")
        t = self.tmp
        self.work, self.log, self.stage = os.path.join(t, "work"), os.path.join(t, "log"), os.path.join(t, "stage")
        self.root, self.report = os.path.join(t, "root310"), os.path.join(t, "report.txt")
        self.screen = os.path.join(t, "screen-state")
        write(self.stage, "310\n")
        write(self.log, "")
        write(self.screen, "active\n")
        fakebin, remote = os.path.join(t, "fakebin"), os.path.join(t, "remote-bin")
        write(os.path.join(fakebin, "ssh"), FAKE_SSH, 0o755)
        os.makedirs(remote)
        for name in APPLETS:
            os.symlink(self.busybox, os.path.join(remote, name))
        write(os.path.join(remote, "systemctl"), FAKE_SYSTEMCTL, 0o755)
        write(os.path.join(remote, "journalctl"), FAKE_JOURNALCTL, 0o755)
        self.app, self.journal, self.clock = (os.path.join(t, n) for n in ("app-state", "journal", "clock"))
        write(self.app, "inactive\n")
        write(self.journal, "")
        write(self.clock, "100\n")
        program_standin = os.path.join(t, "program-standin")
        write(program_standin, FAKE_PROGRAM, 0o755)
        self.env = {"PATH": fakebin + os.pathsep + os.environ.get("PATH", "/usr/bin:/bin"),
                    "HOME": t, "MPFREE_APP_TEST": "1", "FAKE_STAGE": self.stage, "FAKE_LOG": self.log,
                    "FAKE_ROOT": self.root, "FAKE_REMOTE_PATH": remote, "FAKE_SCREEN": self.screen,
                    "FAKE_PROGRAM": program_standin, "FAKE_ACVS": "inactive", "FAKE_PADS": "3",
                    "FAKE_PADS_KEEPALIVE": "2", "FAKE_CLICK_RC": "0", "FAKE_APP": self.app,
                    "FAKE_JOURNAL": self.journal, "FAKE_CLOCK": self.clock,
                    "FAKE_REPORT": os.path.join(self.root, "run/mpfree-app/report.txt")}
        self.key = os.path.join(t, "key")
        write(self.key, "fake key\n", 0o600)
        self.program = os.path.join(t, "mpfree")
        write(self.program, arm_elf(), 0o755)
        self.build()
        self.make_root()

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def build(self):
        folder = os.path.join(self.work, "kexec-stick-20260101T000000Z-abc")
        write(os.path.join(folder, "stick.img"), b"\x00" * 4096)
        write(os.path.join(folder, "known_hosts"), "mpfree-kexec-test ssh-ed25519 AAAAfake\n")
        write(os.path.join(folder, "report.txt"), f"its kernel release: {NEW}\nrunning release: 6.6.95-fake\n")

    def make_root(self, mmc=False, run_opts="rw,nosuid,nodev,mode=755"):
        r = self.root
        shutil.rmtree(r, ignore_errors=True)
        write(f"{r}/proc/sys/kernel/osrelease", NEW + "\n")
        write(f"{r}/proc/sys/kernel/kexec_load_disabled", "1\n")
        write(f"{r}/proc/uptime", "100.00 1.00\n")
        for name in ("sda", "loop0") + (("mmcblk0",) if mmc else ()):
            os.makedirs(f"{r}/sys/block/{name}")
        write(f"{r}/proc/mounts", "/dev/root / ext4 ro,relatime 0 0\n"
                                  f"tmpfs /run tmpfs {run_opts} 0 0\n"
                                  "tmpfs /data tmpfs rw,size=262144k,mode=755 0 0\n")
        write(f"{r}/sys/bus/usb/devices/1-1/idVendor", "09e8\n")
        write(f"{r}/sys/bus/usb/devices/1-1/idProduct", os.environ.get("FAKE_USB_PRODUCT", "0046") + "\n")
        write(f"{r}/sys/bus/usb/devices/usb1/idVendor", "1d6b\n")
        write(f"{r}/proc/321/comm", "mpfree-screen-l\n")
        os.makedirs(f"{r}/proc/321/fd")
        os.symlink("/dev/fb0", f"{r}/proc/321/fd/3")
        os.symlink("/dev/null", f"{r}/proc/321/fd/0")
        os.makedirs(f"{r}/run")

    def run_helper(self, answers="\n" * 12, program=None, **env):
        cmd = [sys.executable, "-I", HELPER, "--ip", "192.0.2.141", "--key", self.key, "--work", self.work,
               "--report", self.report, "--program", program or self.program]
        return subprocess.run(cmd, input=answers, env=dict(self.env, **env), text=True, stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, timeout=120)

    def log_lines(self, prefix):
        with open(self.log) as fh:
            return [ln[len(prefix):].strip() for ln in fh.read().splitlines() if ln.startswith(prefix)]

    def test_runs_every_step_in_order(self):
        res = self.run_helper()
        self.assertEqual(res.returncode, 0, res.stdout)
        out = res.stdout
        for text in ("static ARM EABI5 hard-float", f"release: {NEW}", "no MMC block device", "/: read-only",
                     "/run: tmpfs rw,nosuid,nodev,mode=755", "Akai's app (acvs.service): inactive",
                     "/dev/fb0: pid 321 (mpfree-screen-l)", "CHECKS-OK", "its hash on the MPC matches",
                     "inventory done", "pad player: was inactive, now inactive",
                     "log display: was active, now inactive", "screen: card drawn", "touch done:",
                     "3 pad hits arrived without the keep-alive, so it is not sent.", "click done:",
                     "Starting the log display again:", "log display: active", "| ===== mpfree click =====",
                     "RESULT: all steps ran", "NEXT: switch the MPC off with its power switch"):
            self.assertIn(text, out)
        self.assertEqual(self.log_lines("program: "),
                         ["inventory", "screen", "touch --seconds 10", "pads --seconds 15",
                          "click --seconds 10 --gain-db -12"])
        self.assertEqual(self.log_lines("systemctl: "),
                         ["is-active acvs.service", "is-active mpfree-test-screen.service",
                          "is-active mpfree-test-app.service",
                          "is-active mpfree-test-app.service", "is-active mpfree-test-app.service",
                          "is-active mpfree-test-screen.service", "stop mpfree-test-screen.service",
                          "is-active mpfree-test-screen.service",
                          "start mpfree-test-screen.service", "is-active mpfree-test-screen.service"])
        with open(self.program, "rb") as a, open(f"{self.root}/run/mpfree-app/mpfree", "rb") as b:
            self.assertEqual(hashlib.sha256(a.read()).digest(), hashlib.sha256(b.read()).digest())
        self.assertEqual(os.stat(f"{self.root}/run/mpfree-app/mpfree").st_mode & 0o777, 0o700)
        self.assertFalse(os.path.exists(f"{self.root}/run/mpfree-app/mpfree.part"))
        with open(self.report) as fh:
            self.assertIn("RESULT: all steps ran", fh.read())

    def test_keepalive_only_without_pad_hits(self):
        # Enter and an answer for: inventory, screen, touch, pads, the keep-alive try, click.
        res = self.run_helper("\n\nscreen ok\n\ntouch ok\n\npads 1 to 4, wheel\n\npads again\n\nclick ok\n",
                              FAKE_PADS="0")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("No pad hits arrived.", res.stdout)
        for answer in ("screen ok", "touch ok", "pads 1 to 4, wheel", "pads again", "click ok"):
            self.assertIn(f"your answer: {answer}", res.stdout)
        self.assertEqual(self.log_lines("program: "),
                         ["inventory", "screen", "touch --seconds 10", "pads --seconds 15",
                          "pads --seconds 15 --keepalive", "click --seconds 10 --gain-db -12"])

    def test_skips_and_stops(self):
        # inventory: skip; screen: skip; touch: go (and a note); pads: quit. Touch draws, so it stops the
        # log display, which is started again at the end.
        res = self.run_helper("s\ns\n\nyes, both fingers\nq\n")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertEqual(self.log_lines("program: "), ["touch --seconds 10"])
        self.assertEqual([l for l in self.log_lines("systemctl: ") if "mpfree-test-screen" in l and "is-active" not in l],
                         ["stop mpfree-test-screen.service", "start mpfree-test-screen.service"])
        self.assertIn("STOP: stopped by you", res.stdout)
        self.assertIn("your answer: yes, both fingers", res.stdout)
        # inventory only: nothing draws, so the log display is never stopped.
        write(self.log, "")
        res = self.run_helper("\n", MPFREE_APP_STEPS="inventory")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertEqual(self.log_lines("program: "), ["inventory"])
        self.assertNotIn("stop mpfree-test-screen.service", self.log_lines("systemctl: "))
        # inventory: go; screen: go (and a note); touch: quit. The log display is started again.
        write(self.log, "")
        res = self.run_helper("\n\ny\nq\n")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertEqual(self.log_lines("program: "), ["inventory", "screen"])
        self.assertEqual(self.log_lines("systemctl: ")[-2:],
                         ["start mpfree-test-screen.service", "is-active mpfree-test-screen.service"])
        with open(self.screen) as fh:
            self.assertEqual(fh.read().strip(), "active")

    def test_other_answers_ask_again(self):
        res = self.run_helper("yes\n" + "\n" * 12)
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("Please press just Enter, or s, or q.", res.stdout)

    def test_a_failed_step_is_reported_and_the_rest_go_on(self):
        res = self.run_helper(FAKE_CLICK_RC="1")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("click: exit code 1", res.stdout)

    def kit(self, data=b"MPFKITS1" + struct.pack("<I", 2) + b"\x01" * 100, name="factory-kit-20261110T000000Z-abc"):
        path = os.path.join(self.work, name, "kits.bin")
        write(path, data)
        return path

    def test_lights_setup_and_drums(self):
        kit = self.kit()
        # lights: Enter + answer; drums: Enter + answer; setup: a wrong word, then SEND + answer.
        res = self.run_helper("\nall pads lit\n\nkick on pad 1\nsend\nSEND\nbuttons lit\n",
                              MPFREE_APP_STEPS="drums,setup,lights")
        self.assertEqual(res.returncode, 0, res.stdout)
        out = res.stdout
        for text in ("steps: lights, drums, setup", f"drum kits: {kit}", "2 kits, 112 bytes",
                     "drum kits copied; their hash on the MPC matches",
                     "[4/6] Lights", "[5/6] Drums", "[6/6] Akai's setup", "Please type SEND exactly, or s, or q.",
                     "your answer: all pads lit", "your answer: buttons lit", "your answer: kick on pad 1",
                     "kit: 8 sounds", "drums done: 12 pad hits played", "RESULT: all steps ran"):
            self.assertIn(text, out)
        self.assertEqual(out.count("controller: 09e8:0046"), 3)
        self.assertEqual(self.log_lines("program: "),
                         ["lights --seconds 15", "drums --seconds 30 --gain-db -12", "setup --seconds 20"])
        with open(kit, "rb") as a, open(f"{self.root}/run/mpfree-app/kits.bin", "rb") as b:
            self.assertEqual(a.read(), b.read())
        self.assertEqual(os.stat(f"{self.root}/run/mpfree-app/kits.bin").st_mode & 0o777, 0o600)
        # Without the screen step, the first drawing step stops the log display; it is started again at the end.
        self.assertEqual([l for l in self.log_lines("systemctl: ") if "mpfree-test-screen" in l and "is-active" not in l],
                         ["stop mpfree-test-screen.service", "start mpfree-test-screen.service"])

    def test_setup_can_be_skipped_and_drums_needs_a_kit(self):
        res = self.run_helper("s\n", MPFREE_APP_STEPS="setup,drums")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("no factory kits found (make them with offline-factory-kit.py); left out: drums", res.stdout)
        self.assertIn("steps: drums, setup", res.stdout)
        self.assertNotIn("[5/", res.stdout)
        self.assertEqual(self.log_lines("program: "), [])
        self.assertFalse(os.path.exists(f"{self.root}/run/mpfree-app/kits.bin"))
        # A file from round 3 (one kit, the old format) is not taken.
        self.kit(b"MPFKIT01" + b"\x01" * 100)
        write(self.log, "")
        res = self.run_helper("", MPFREE_APP_STEPS="drums")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("is not a kits file", res.stdout)
        self.assertEqual(self.log_lines("ssh["), [])
        res = self.run_helper("", MPFREE_APP_STEPS="pads,format")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("unknown step(s) format", res.stdout)

    def test_play_needs_send_and_the_kits(self):
        kit = self.kit()
        # play: a wrong word, then SEND + answer.
        res = self.run_helper("go\nSEND\nkits and level followed\n", MPFREE_APP_STEPS="play")
        self.assertEqual(res.returncode, 0, res.stdout)
        out = res.stdout
        for text in ("steps: play", f"drum kits: {kit}", "drum kits copied; their hash on the MPC matches",
                     "[4/4] Pad player", "STATE-CHANGING", "Please type SEND exactly, or s, or q.",
                     "pad player: 2 kits, 20 sounds", "play done (STOP): 40 pad hits, 0 underruns",
                     "controller: 09e8:0046", "your answer: kits and level followed", "RESULT: all steps ran"):
            self.assertIn(text, out)
        self.assertEqual(self.log_lines("program: "), ["play --seconds 120 --gain-db -12"])
        self.assertEqual([l for l in self.log_lines("systemctl: ") if "mpfree-test-screen" in l and "is-active" not in l],
                         ["stop mpfree-test-screen.service", "start mpfree-test-screen.service"])
        # s skips it: nothing runs and the log display is never stopped.
        write(self.log, "")
        res = self.run_helper("s\n", MPFREE_APP_STEPS="play")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertEqual(self.log_lines("program: "), [])
        self.assertNotIn("stop mpfree-test-screen.service", self.log_lines("systemctl: "))
        # q stops.
        res = self.run_helper("q\n", MPFREE_APP_STEPS="play")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("STOP: stopped by you", res.stdout)
        self.assertEqual(self.log_lines("program: "), [])
        # Without kits, play is left out with drums.
        shutil.rmtree(os.path.dirname(kit))
        write(self.log, "")
        res = self.run_helper("", MPFREE_APP_STEPS="drums,play")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("left out: drums, play", res.stdout)
        self.assertIn("STOP: no steps left to run; nothing was sent to the MPC", res.stdout)
        self.assertEqual(self.log_lines("ssh["), [])

    def add_journal(self, lines, at=None):
        """Lines for the fake pad player's log, written now or when the fake clock reaches `at`."""
        path = self.journal if at is None else f"{self.journal}.{at}"
        with open(path, "a") as fh:
            for t, message in lines:
                fh.write(f"[{t:12.6f}] {message}\n")

    def test_app_step_only_reads_and_follows_each_phase(self):
        write(self.app, "active\n")
        write(self.screen, "inactive\n")
        self.add_journal([(5.1, "systemd[1]: Started MPFree test boot: the pad player, from power-on."),
                      (6.0, "mpfree[211]: pad player: 6 kits, 81 sounds"),
                      (9.5, "mpfree[211]: playing until the power goes off: ...")])
        # The start-up read moves the fake clock to 110; phase k starts at 100 + 10 k and its lines are due
        # at 110 + 10 k.
        self.add_journal([(111.0, "mpfree[211]: +40.100 s Private: 90 0B 7F = button 11 pressed"),
                      (111.1, "mpfree[211]: button guide: MENU is button 11")], at=120)
        self.add_journal([(121.0, "mpfree[211]: +50.000 s Private: 99 25 12 = pad 1 down, velocity 18"),
                      (121.1, "mpfree[211]: +50.100 s Private: 89 25 00 = pad 1 up"),
                      (121.5, "mpfree[211]: +50.500 s Private: 99 25 7C = pad 1 down, velocity 124"),
                      (122.0, "mpfree[211]: +51.000 s Private: 90 27 7F = button 39 pressed"),
                      (122.1, "mpfree[211]: pad level: Full Level (every hit 127)"),
                      (123.0, "mpfree[211]: +52.000 s Private: B0 64 01 = data wheel +1"),
                      (123.5, "mpfree[211]: +52.500 s -> Public: F0 47 00 46 52 00 04 15 21 6D 43 F7 = session ping")],
                     at=130)
        self.add_journal([(141.0, "mpfree[211]: Latch on"),
                      (143.0, "mpfree[211]: repeat: 16 hits in the last second on pads 1 2; latched: 1 2; "
                              "engine dropped 0 events, overflow 0"),
                      (150.0, "mpfree[211]: status: 60 hits (0 from the screen), 40 repeats, 0 underruns in "
                              "20000 periods, ...")], at=150)
        # Start-up: Enter + note; phases 1 to 3: Enter, Enter when done, note; phase 4: Enter, then q.
        answers = "\nloading ok\n" + "\n\nguide ok\n" + "\n\nsoft and hard differ\n" + "\n\nrepeat ok\n" + "\nq\n"
        res = self.run_helper(answers, MPFREE_APP_STEPS="app")
        self.assertEqual(res.returncode, 1, res.stdout)
        out = res.stdout
        for text in ("steps: app", "[3/3] The pad player that started at power-on (read-only",
                     "pad player (mpfree-test-app.service): active", "(systemd[1]) Started MPFree test boot",
                     "pad player: 6 kits, 81 sounds", "start-up: the app was playing 9.5 s after power-on",
                     "your answer: loading ok", "Phase 1 of 16: Developer tools and Map controls",
                     "button guide: MENU is button 11", "buttons pressed (note numbers): 11",
                     "Phase 2 of 16: Developer pages: Performance, CPU by engine, Processes", "pads hit (pad:velocity as played): 1:18 1:124",
                     "buttons pressed (note numbers): 39", "pad level: Full Level (every hit 127)",
                     "data wheel: +1", "session pings and replies: 1", "your answer: soft and hard differ",
                     "Phase 4 of 16: Play page: the four views and the mode buttons", "repeat: 16 hits in the last second on pads 1 2; latched: 1 2; "
                     "engine dropped 0 events, overflow 0", "STOP: stopped by you"):
            self.assertIn(text, out)
        # Phase 1's lines are not shown again in phase 2.
        self.assertEqual(out.count("button guide: MENU is button 11"), 1)
        self.assertNotIn("[3/", out.split("steps: app")[0])
        self.assertNotIn("Copy the program", out)
        with open(self.report) as fh:
            report = fh.read()
        self.assertIn("| [   121.500] +50.500 s Private: 99 25 7C = pad 1 down, velocity 124", report)
        # Nothing was copied, started or stopped; only reads.
        self.assertFalse(os.path.exists(f"{self.root}/run/mpfree-app"))
        self.assertEqual(self.log_lines("program: "), [])
        self.assertEqual([l for l in self.log_lines("systemctl: ") if not l.startswith("is-active ")], [])
        self.assertTrue(all(l.startswith("-b -u mpfree-test-app.service -o short-monotonic")
                            for l in self.log_lines("journalctl: ")))
        with open(self.app) as fh:
            self.assertEqual(fh.read().strip(), "active")

    def test_app_step_all_phases_and_the_status(self):
        write(self.app, "active\n")
        self.add_journal([(100.0, "mpfree[211]: status: 30 hits (0 from the screen), 20 repeats, 0 underruns"),
                          (100.1, "mpfree[211]: status: looper: Empty, 0 layers"),
                          (130.0, "mpfree[211]: status: 60 hits (0 from the screen), 40 repeats, 0 underruns"),
                          (130.1, "mpfree[211]: status: synth: Poly (Brass)"),
                          (130.2, "mpfree[211]: status: looper: Playing, 2 layers"),
                          (130.3, "mpfree[211]: status: cpu by engine (mean/p99/worst of a period): period 9.3"),
                          (130.4, "mpfree[211]: status: Q-Link touch: 3 touches, 1 bounces held")])
        answers = "\n\n" + "\n\n\n" * 16
        res = self.run_helper(answers, MPFREE_APP_STEPS="app")
        self.assertEqual(res.returncode, 0, res.stdout)
        for title in ("Phase 5 of 16: Throws, the XY pad (Perform FX) and FILL",
                      "Phase 6 of 16: Arrange: clips, takes, Capture, the song and the morph",
                      "Phase 7 of 16: TRACK MUTE and SHIFT + TRACK MUTE",
                      "Phase 11 of 16: Looper tab: record, layers, tools, undo history, trim",
                      "Phase 12 of 16: Mixer, effect chains and macros",
                      "Phase 13 of 16: Sample tab: Pads, Wave and Chop",
                      "Phase 15 of 16: Touch: Touch lab, Calibrate touch and the Dev Touch page",
                      "Phase 16 of 16: Menus, console and STOP"):
            self.assertIn(title, res.stdout)
        self.assertIn("start-up: no playing line yet", res.stdout)
        self.assertIn("The app's latest status", res.stdout)
        self.assertIn("    status: 60 hits (0 from the screen), 40 repeats, 0 underruns", res.stdout)
        # The latest whole set of status lines, not only the last three.
        summary = res.stdout.split("The app's latest status")[1].split("The developer tools'")[0]
        for line in ("status: 60 hits", "status: synth: Poly (Brass)", "status: looper: Playing, 2 layers",
                     "status: Q-Link touch: 3 touches, 1 bounces held"):
            self.assertIn(f"    {line}", summary)
        self.assertNotIn("status: 30 hits", summary)
        self.assertNotIn("status: looper: Empty", summary)
        self.assertIn("RESULT: all steps ran", res.stdout)

    def test_app_step_reads_partition_4_and_its_grow(self):
        """Read-only: partition 4's size and free space, and what the online grow at power-on said."""
        write(self.app, "active\n")
        r = self.root
        with open(f"{r}/proc/mounts", "a") as fh:
            fh.write("/dev/sda4 /run/mpfree-data ext4 rw,nosuid,nodev,noexec,noatime 0 0\n")
        os.makedirs(f"{r}/run/mpfree-data", exist_ok=True)
        write(f"{r}/run/mpfree-data-grow.txt", "resize2fs 1.47.0 (5-Feb-2023)\n"
              "The filesystem on /dev/sda4 is now 7520000 (4k) blocks long.\n")
        res = self.run_helper("\nq\n", MPFREE_APP_STEPS="app")
        self.assertRegex(res.stdout, r"partition 4: \d+ KiB, \d+ KiB used, \d+ KiB free")
        self.assertIn("partition 4 grow:\n    resize2fs 1.47.0 (5-Feb-2023)\n"
                      "    The filesystem on /dev/sda4 is now 7520000 (4k) blocks long.", res.stdout)
        os.remove(f"{r}/run/mpfree-data-grow.txt")
        with open(f"{r}/proc/mounts") as fh:
            mounts = fh.read().replace("/dev/sda4 /run/mpfree-data ext4 rw,nosuid,nodev,noexec,noatime 0 0\n", "")
        write(f"{r}/proc/mounts", mounts)
        res = self.run_helper("\nq\n", MPFREE_APP_STEPS="app")
        self.assertIn("partition 4: not mounted", res.stdout)
        self.assertIn("partition 4 grow: no /run/mpfree-data-grow.txt", res.stdout)

    def test_app_step_reads_the_gpu_and_display_read_only(self):
        """Read-only: the GPU's libraries, the release, /dev/dri, the panel's mode, who holds /dev/dri,
        Panfrost, and debugfs only where it is already there; a missing path says so."""
        write(self.app, "active\n")
        r = self.root
        res = self.run_helper("\nq\n", MPFREE_APP_STEPS="app")
        for text in ("gpu: no libEGL, libGLESv2, libgbm, libdrm or /usr/lib/dri",
                     "gpu: /etc/os-release not there", "gpu: /dev/dri not there",
                     "gpu: no /sys/class/drm/card*-* connectors", "gpu: nothing has /dev/dri open",
                     "gpu: panfrost driver not there",
                     "gpu: debugfs dri/0 not there (it is not mounted for this)"):
            self.assertIn(text, res.stdout)
        write(f"{r}/usr/lib/libEGL.so.1.0.0", "")
        os.symlink("libEGL.so.1.0.0", f"{r}/usr/lib/libEGL.so.1")
        write(f"{r}/usr/lib/dri/rockchip_dri.so", "")
        write(f"{r}/etc/os-release", 'NAME="Akai"\nVERSION_ID=3.10.0\nHOME_URL=x\n')
        write(f"{r}/dev/dri/card0", "")
        write(f"{r}/sys/class/drm/card0-DSI-1/status", "connected\n")
        write(f"{r}/sys/class/drm/card0-DSI-1/modes", "800x1280\n")
        os.makedirs(f"{r}/proc/412/fd", exist_ok=True)
        write(f"{r}/proc/412/comm", "weston\n")
        os.symlink("/dev/dri/card0", f"{r}/proc/412/fd/7")
        for name in ("bind", "uevent", "ff9a0000.gpu"):
            write(f"{r}/sys/bus/platform/drivers/panfrost/{name}", "")
        write(f"{r}/sys/kernel/debug/dri/0/state", "".join(f"plane[{i}]: x\n" for i in range(100)))
        res = self.run_helper("\nq\n", MPFREE_APP_STEPS="app")
        out = res.stdout
        for text in ("gpu: /usr/lib/libEGL.so.1 -> libEGL.so.1.0.0", "gpu: /usr/lib/libEGL.so.1.0.0",
                     "gpu: /usr/lib/dri/rockchip_dri.so", 'gpu: os-release NAME="Akai"',
                     "gpu: os-release VERSION_ID=3.10.0", "gpu: /dev/dri/card0",
                     "gpu: /sys/class/drm/card0-DSI-1: connected, modes: 800x1280",
                     "gpu: /dev/dri is open in pid 412 (weston)",
                     "gpu: panfrost driver bound to: ff9a0000.gpu",
                     "gpu: debugfs dri/0 state (first 80 lines):",
                     "(the GPU read's other 80 lines are in the report)"):
            self.assertIn(text, out)
        self.assertNotIn("HOME_URL", out)
        self.assertNotIn("plane[0]", out, "debugfs lines go to the report only")

    def test_app_step_reads_the_cores_after_the_dev_pages(self):
        """Read-only, during play: the governor, the clock source, each app thread's cores and priority."""
        write(self.app, "active\n")
        r = self.root
        write(f"{r}/sys/devices/system/cpu/online", "0-3\n")
        write(f"{r}/sys/devices/system/cpu/isolated", "2-3\n")
        write(f"{r}/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor", "interactive\n")
        write(f"{r}/sys/devices/system/clocksource/clocksource0/current_clocksource", "arch_sys_counter\n")
        write(f"{r}/proc/900/comm", "mpfree\n")
        fields = " ".join(["S"] + ["0"] * 35 + ["1", "70", "1"] + ["0"] * 10)
        for tid, name, cpus in (("900", "mpfree", "0-1"), ("903", "mpfree-screen", "0-1")):
            write(f"{r}/proc/900/task/{tid}/comm", name + "\n")
            write(f"{r}/proc/900/task/{tid}/stat", f"{tid} ({name}) {fields}\n")
            write(f"{r}/proc/900/task/{tid}/status", f"Name:\t{name}\nCpus_allowed_list:\t{cpus}\n")
        write(f"{r}/proc/stat", "cpu  10 0 10 100 0 0 0 0 0 0\ncpu0 5 0 5 50 0 0 0 0 0 0\n")
        write(f"{r}/proc/interrupts", "           CPU0       CPU1\n 61:         42          0  ili2117\n")
        answers = "\n\n" + "\n\n\n" * 2 + "q\n"
        res = self.run_helper(answers, MPFREE_APP_STEPS="app")
        out = res.stdout
        for text in ("cores, clock and threads now (read-only):", "/sys/devices/system/cpu/isolated: 2-3",
                     "/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor: interactive",
                     "current_clocksource: arch_sys_counter", "app pid 900",
                     "900 mpfree: allowed \t0-1 last_cpu=1 rt_priority=70 policy=1",
                     "903 mpfree-screen: allowed \t0-1 last_cpu=1", "app processes: 1",
                     "cpu0 5 0 5 50 0 0 0 0 0 0",
                     "/sys/devices/system/cpu/cpu[0-3]/cpufreq/scaling_cur_freq: not there"):
            self.assertIn(text, out)
        self.assertEqual(out.count("cores, clock and threads now"), 1, "only after phase 2 so far")
        self.assertNotIn(" 61:         42", out.split("cores, clock and threads now")[1].split("Phase 3")[0])
        with open(self.report) as fh:
            self.assertIn("|  61:         42          0  ili2117", fh.read())

    def test_app_step_summary_reads_the_whole_session(self):
        """Every phase skipped: the summary still shows the whole run, the touch interrupts and the status."""
        write(self.app, "active\n")
        write(f"{self.root}/proc/interrupts", " 61:        42          0  rockchip_gpio_irq   6 Edge      ili2117\n")
        self.add_journal([(10.5, "mpfree[211]: touchscreen: /dev/input/event0 (ILI2117 Touchscreen); read-only, not grabbed"),
                          (11.0, "mpfree[211]: touch-reader: seeded=yes contacts=0 events=0"),
                          (12.0, "mpfree[211]: saved settings: pad curve Hard, screen velocity 100, latch "
                                 "replace, audio buffer 3 periods, developer tools on"),
                          (12.5, "mpfree[211]: saving: settings and found buttons are kept in /run/mpfree-data"),
                          (20.0, "mpfree[211]: +9.000 s Private: 99 25 40 = pad 1 down, velocity 64"),
                          (20.5, "mpfree[211]: +9.500 s Private: 90 27 7F = button 39 pressed"),
                          (20.6, "mpfree[211]: pad level: Full Level (every hit 127)"),
                          (30.0, "mpfree[211]: button guide done: MENU = button 123, REC = button 7"),
                          (31.0, "mpfree[211]: developer tools: on"),
                          (32.0, "mpfree[211]: map: PAD BANK A/E = button 35"),
                          (33.0, "mpfree[211]: bank: B (kit 2: Lo-fi)"),
                          (34.0, "mpfree[211]: perf: cpu 30.1 % of a core (mpfree (812) 25.0)"),
                          (35.0, "mpfree[211]: time: transport: Record"),
                          (40.0, "mpfree[211]: touch-lab: step=taps result=pass downs=5 note=ok"),
                          (45.0, "mpfree[211]: touch-calibrate: result=match"),
                          (60.0, "mpfree[211]: status: 1 hits (0 from the screen), 0 underruns; sampler: render mean 90 us")])
        answers = "\n\n" + "s\n" * 12
        res = self.run_helper(answers, MPFREE_APP_STEPS="app")
        self.assertEqual(res.returncode, 0, res.stdout)
        out = res.stdout
        for text in ("touch interrupts at the start: 61: 42 0 rockchip_gpio_irq 6 Edge ili2117",
                     "    saved settings: pad curve Hard, screen velocity 100, latch replace, audio buffer 3 periods, "
                     "developer tools on",
                     "    saving: settings and found buttons are kept in /run/mpfree-data",
                     "Session summary (the whole run since power-on",
                     "    touch-reader: seeded=yes contacts=0 events=0", "    pad level: Full Level (every hit 127)",
                     "    button guide done: MENU = button 123, REC = button 7", "    time: transport: Record",
                     "    developer tools: on", "    map: PAD BANK A/E = button 35", "    bank: B (kit 2: Lo-fi)",
                     "The developer tools' process numbers (1 perf lines; the latest):",
                     "    perf: cpu 30.1 % of a core (mpfree (812) 25.0)",
                     "    touch-lab: step=taps result=pass downs=5 note=ok",
                     "    touch-calibrate: result=match",
                     "pad hits from the controller: 1; buttons pressed: 39",
                     "touch interrupts now: 61: 42 0 rockchip_gpio_irq 6 Edge ili2117",
                     "    status: 1 hits (0 from the screen), 0 underruns; sampler: render mean 90 us"):
            self.assertIn(text, out)
        self.assertEqual([l for l in self.log_lines("systemctl: ") if not l.startswith("is-active ")], [])

    def make_acva(self):
        """The ACVA card's procfs files as the MPC shows them while nothing records, and a card before it."""
        r = self.root
        write(f"{r}/proc/asound/card0/id", "rockchiphdmi\n")
        write(f"{r}/proc/asound/card1/id", "ACVA\n")
        write(f"{r}/proc/asound/card1/pcm0c/info", "card: 1\ndevice: 0\nstream: CAPTURE\nsubdevices_count: 1\n")
        write(f"{r}/proc/asound/card1/pcm0c/sub0/status", "closed\n")
        write(f"{r}/proc/asound/card1/pcm0c/sub0/hw_params", "closed\n")
        write(f"{r}/proc/asound/card1/pcm0p/sub0/hw_params", "access: RW_INTERLEAVED\nformat: S32_LE\nrate: 44100 (44100/1)\n")
        write(f"{r}/dev/snd/pcmC1D0c", "")

    def test_line_step_reads_then_waits_for_the_word_and_follows_each_phase(self):
        """The line step reads the capture device's state and the log; the pad player does the opening."""
        write(self.app, "active\n")
        self.make_acva()
        self.add_journal([(101.0, "mpfree[211]: line input: on: /dev/snd/pcmC1D0c, rate 44100, channels 2, "
                                  "period 128 frames, buffer 256 frames"),
                          (101.1, "mpfree[211]: looper: records from line input")], at=110)
        self.add_journal([(111.0, "mpfree[211]: looper: Record: armed, the first pass starts on the next bar line "
                                  "(2 bars)"),
                          (115.0, "mpfree[211]: looper: loop of 2 bars at 3.1.000: 176400 frames (4.00 s), playing")],
                         at=120)
        self.add_journal([(121.0, "mpfree[211]: line input: off (2000 periods, 0 short, 0 frames dropped, "
                                  "0 overruns, 0 errors)")], at=130)
        answers = "LINE\n" + "\n\nmeter moved, L left\n" + "\n\nloop ok\n" + "\n\noff ok\n"
        res = self.run_helper(answers, MPFREE_APP_STEPS="line")
        self.assertEqual(res.returncode, 0, res.stdout)
        out = res.stdout
        for text in ("steps: line", "[3/3] The line input (STATE-CHANGING on the MPC, RAM only",
                     "pad player (mpfree-test-app.service): active", "the line input before (read-only):",
                     "    ACVA is card 1", "    --- card1/pcm0c/sub0/status", "    --- card1/pcm0p/sub0/hw_params",
                     "Undo: Line input Off", "answer: LINE", "Phase 1 of 3: Line input on, and which jack is which",
                     "line input: on: /dev/snd/pcmC1D0c, rate 44100", "the line input now (read-only):",
                     "your answer: meter moved, L left", "Phase 2 of 3: Loop the line input",
                     "looper: loop of 2 bars at 3.1.000", "Phase 3 of 3: Line input off", "your answer: off ok",
                     "The line input and the looper, the whole session:", "    line input: off (2000 periods",
                     "RESULT: all steps ran"):
            self.assertIn(text, out)
        self.assertEqual(out.count("ACVA is card 1"), 4, "before, and after each phase")
        self.assertNotIn("rockchiphdmi", out)
        # Nothing was copied, started or stopped; only reads.
        self.assertNotIn("Copy the program", out)
        self.assertFalse(os.path.exists(f"{self.root}/run/mpfree-app"))
        self.assertEqual(self.log_lines("program: "), [])
        self.assertEqual([l for l in self.log_lines("systemctl: ") if not l.startswith("is-active ")], [])
        with open(self.app) as fh:
            self.assertEqual(fh.read().strip(), "active")

    def test_line_step_needs_the_word_and_the_pad_player(self):
        write(self.app, "active\n")
        self.make_acva()
        res = self.run_helper("line\ns\n", MPFREE_APP_STEPS="line")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("Please type LINE exactly, or s, or q.", res.stdout)
        self.assertIn("answer: s", res.stdout)
        self.assertNotIn("Phase 1", res.stdout)
        self.assertEqual(res.stdout.count("ACVA is card 1"), 1, "only the read-only look before")
        # Without the pad player, after the app step, it reads and stops there; no ACVA card, it says so.
        write(self.app, "inactive\n")
        shutil.rmtree(f"{self.root}/proc/asound")
        res = self.run_helper("\nit showed an error\n", MPFREE_APP_STEPS="line,app")
        self.assertEqual(res.returncode, 0, res.stdout)
        out = res.stdout
        self.assertLess(out.index("[3/4] The pad player that started at power-on"), out.index("[4/4] The line input"))
        self.assertIn("no sound card with the id ACVA", out)
        self.assertIn("The pad player is not running, so this step is left out. Nothing was opened.", out)
        self.assertNotIn("Type LINE", out)
        self.assertNotIn("Copy the program", out)

    def test_app_step_when_the_app_is_not_running(self):
        self.add_journal([(7.0, "mpfree[211]: STOP: no sound card called ACVA after 30 s")])
        res = self.run_helper("\nit showed an error\n", MPFREE_APP_STEPS="app")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("pad player (mpfree-test-app.service): inactive", res.stdout)
        self.assertIn("STOP: no sound card called ACVA after 30 s", res.stdout)
        self.assertIn("The pad player is not running, so the other phases are left out.", res.stdout)
        self.assertNotIn("Phase 1", res.stdout)

    def test_other_steps_stop_the_app_and_start_it_again(self):
        write(self.app, "active\n")
        write(self.screen, "inactive\n")
        res = self.run_helper("\nok\n", MPFREE_APP_STEPS="screen")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("pad player: was active, now inactive", res.stdout)
        self.assertIn("log display: was inactive, now inactive", res.stdout)
        self.assertIn("Starting the pad player again:\n  pad player: active", res.stdout)
        self.assertNotIn("Starting the log display again", res.stdout)
        self.assertEqual([l for l in self.log_lines("systemctl: ") if not l.startswith("is-active ")],
                         ["stop mpfree-test-app.service", "start mpfree-test-app.service"])
        with open(self.screen) as fh:
            self.assertEqual(fh.read().strip(), "inactive")

    def test_stops_if_the_controller_changes(self):
        os.environ["FAKE_USB_PRODUCT"] = "0047"
        try:
            self.make_root()
        finally:
            del os.environ["FAKE_USB_PRODUCT"]
        res = self.run_helper("\n\n\n", MPFREE_APP_STEPS="lights,click")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("controller: 09e8:0047", res.stdout)
        self.assertIn("STOP: the controller's USB id changed", res.stdout)
        self.assertEqual(self.log_lines("program: "), ["lights --seconds 15"])

    def test_refuses_anything_but_the_test_build(self):
        write(self.stage, "36\n")
        res = self.run_helper()
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("the test build did not answer", res.stdout)
        self.assertFalse(os.path.exists(f"{self.root}/run/mpfree-app"))
        self.assertEqual(self.log_lines("program: "), [])

    def test_checks_stop_before_anything_is_copied(self):
        for kwargs, env, problem in (
                ({"mmc": True}, {}, "an MMC block device exists"),
                ({"run_opts": "rw,nosuid,nodev,noexec"}, {}, "/run does not allow programs (noexec)"),
                ({}, {"FAKE_ACVS": "active"}, "Akai's app is running")):
            self.make_root(**kwargs)
            res = self.run_helper(**env)
            self.assertEqual(res.returncode, 1, res.stdout)
            self.assertIn(f"PROBLEM: {problem}", res.stdout)
            self.assertIn("the test build is not ready; nothing was changed on it", res.stdout)
            self.assertFalse(os.path.exists(f"{self.root}/run/mpfree-app"))
        self.assertEqual(self.log_lines("program: "), [])

    def test_bad_programs_stop_before_contacting_the_mpc(self):
        for data, why in ((arm_elf(machine=62), "not an ARM executable"),
                          (arm_elf(flags=0x05000200), "not EABI5 hard-float"),
                          (arm_elf(interp=True), "dynamically linked"),
                          (b"#!/bin/sh\necho hi\n" * 10, "not a 32-bit little-endian ELF")):
            write(self.program, data, 0o755)
            res = self.run_helper()
            self.assertEqual(res.returncode, 1, res.stdout)
            self.assertIn(why, res.stdout)
        res = self.run_helper(program=os.path.join(self.tmp, "missing"))
        self.assertIn("build it first", res.stdout)
        self.assertEqual(self.log_lines("ssh["), [])

    def test_wrapper_needs_a_terminal_and_the_opt_in(self):
        env = dict(self.env, MPC_KEY=self.key)
        res = subprocess.run(["bash", WRAPPER, "--app"], input="", env=env, text=True,
                             stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=60)
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("run it from a terminal", res.stdout)
        master, slave = pty.openpty()
        try:
            res = subprocess.run(["bash", WRAPPER, "--app"], stdin=slave, env=env, text=True,
                                 stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=60)
        finally:
            os.close(slave)
            os.close(master)
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("MPFREE_ACCEPT_APP=YES", res.stdout)
        self.assertEqual(self.log_lines("ssh["), [])


class StaticTests(unittest.TestCase):
    def test_remote_scripts_change_only_ram(self):
        import importlib.util
        spec = importlib.util.spec_from_file_location("host_app_mpc", HELPER)
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        names = ("CHECK", "PREPARE", "RECEIVE", "RECEIVE_KIT", "HASH", "HASH_KIT", "RUN", "SCREEN_STOP", "SCREEN_START",
                 "APP_START", "APP_STATE", "UPTIME", "APP_LOG", "TOUCH_IRQ", "REPORT", "CONTROLLER", "LINE_PROBE")
        scripts = {name: getattr(mod, name) for name in names}
        for name, text in scripts.items():
            self.assertTrue(text.startswith("R=''"), name)
            self.assertIsNone(re.search(r"(?<![0-9])>\s*\S*/dev/", text), f"{name}: writes to a device")
            self.assertIsNone(re.search(r"(^|[;&|(]\s*)(mkfs|mke2fs|sfdisk|fdisk|parted|wipefs)", text, re.M), name)
            for needle in ("of=", " dd ", "fw_setenv", "mount ", "kexec --", "systemctl kexec", "flash", "/boot/", "echo 1 >", "tee ",
                           "mmcblk0p", "/dev/sd"):
                self.assertNotIn(needle, text, f"{name}: {needle}")
            self.assertIsNone(re.search(r"(^|[;&|(]\s*|systemctl\s+)(reboot|poweroff|halt)\b", text, re.M), name)
        writes = [ln for t in scripts.values() for ln in t.splitlines() if re.search(r"(^|[^2])>[^&]", ln)]
        self.assertEqual(writes, [mod.RECEIVE, mod.RECEIVE_KIT], writes)
        self.assertIn('cat > "$R/run/mpfree-app/mpfree.part"', mod.RECEIVE)
        self.assertIn('cat > "$R/run/mpfree-app/kits.bin.part"', mod.RECEIVE_KIT)
        services = re.findall(r"systemctl (stop|start|restart|enable|disable|mask|unmask|kill) (\S+)",
                              " ".join(scripts.values()))
        self.assertEqual(sorted(services), [("start", "mpfree-test-app.service"), ("start", "mpfree-test-screen.service"),
                                            ("stop", "mpfree-test-app.service"), ("stop", "mpfree-test-screen.service")])
        # The app step's scripts only read.
        for name in ("APP_STATE", "UPTIME", "APP_LOG", "TOUCH_IRQ"):
            self.assertIsNone(re.search(r"systemctl (?!is-active)", scripts[name]), name)
        self.assertIn("journalctl -b -u mpfree-test-app.service -o short-monotonic --no-pager", mod.APP_LOG)
        # The program's arguments are fixed in the tool; nothing from the command line reaches the MPC.
        self.assertEqual(sorted(mod.PHASES), ["click", "drums", "inventory", "lights", "pads", "pads-keepalive", "play",
                                              "screen", "setup", "touch"])
        for args, _ in mod.PHASES.values():
            self.assertRegex(args, r"^[a-z]+( --[a-z-]+( -?[0-9]+)?)*$")

    def program_sources(self):
        """(name, code) for every Rust file built into the program, without its test module (tests never
        reach the MPC; test code may write temporary files)."""
        files = [(f"mpfree-app/{os.path.relpath(os.path.join(d, n), APP_SRC)}", os.path.join(d, n))
                 for d, _, names in sorted(os.walk(APP_SRC)) for n in sorted(names) if n.endswith(".rs")]
        files += [(f"mpfree-ui/{n}", os.path.join(UI_SRC, n)) for n in sorted(os.listdir(UI_SRC))]
        for d in LINKED_SRC:
            crate = os.path.basename(os.path.dirname(d))
            files += [(f"{crate}/{n}", os.path.join(d, n)) for n in sorted(os.listdir(d))
                      if n.endswith(".rs") and n != "main.rs" and f"{crate}/{n}" not in NOT_LINKED]
        out = []
        for name, path in files:
            with open(path) as fh:
                text = fh.read()
            marker = "#[cfg(test)]\nmod tests {"
            self.assertLessEqual(text.count(marker), 1, name)
            if marker in text:
                code, tests = text.split(marker)
                # The test module is the last thing in the file, so nothing else is cut: its closing brace
                # is the only one at the start of a line, and no other item starts there.
                lines = tests.rstrip("\n").splitlines()
                self.assertEqual([i for i, ln in enumerate(lines) if ln == "}"], [len(lines) - 1], name)
                self.assertEqual([ln for ln in lines if re.match(r"(pub|fn|#\[|impl|struct|enum|const|static|mod|use)\b", ln)],
                                 [], name)
            else:
                code = text
            out.append((name, logical_lines(code)))
        self.assertGreaterEqual(len(out), 20)
        return out

    def test_the_program_writes_only_where_reviewed(self):
        """Every place the program opens a file for writing, in its Rust source."""
        found = []
        # The arp's patterns are saved only through mpfree-store (files/arp/patterns.txt under --data); the
        # pad player itself writes no patterns file (checked below).
        reviewed = {# mpfree-store's save (storage-spec.md): NAME.part beside the file, synced, renamed over it,
                    # and a .part left by a cut-off save removed first. Only under the folder given with --data.
                    ("mpfree-store/lib.rs", "match fs::remove_file(&part) {"),
                    ("mpfree-store/lib.rs", "fs::rename(&part, &path)?;"),
                    ("mpfree-store/lib.rs", "let _ = fs::remove_file(&part);")}
        for name, code in self.program_sources():
            for line in code.splitlines():
                if re.search(r"\.(write|append|create|create_new|truncate)\(true\)", line):
                    found.append((name, line.strip()))
                if (name, line.strip()) in reviewed:
                    continue
                for needle in ("mmcblk", "/dev/sd", "/boot", "mount(", "reboot", "fw_setenv", "/dev/mtd",
                               "/sbin/kexec", "/sys/class/block", "EVIOCGRAB", "remove_file", "remove_dir",
                               "fs::write", "File::create", "fs::rename", "fs::copy", "set_permissions"):
                    self.assertNotIn(needle, line, f"{name}: {line.strip()}")
        with open(os.path.join(HERE, "..", "offline-kexec-stick.py")) as fh:
            stick = fh.read()
        self.assertIn("exec /etc/mpfree-test/mpfree app --kits /run/mpfree-kits/kits.bin{keep}", stick)
        self.assertNotIn("--save-patterns", stick)
        texts = [line for _, line in found]
        expected = (
            # the program's own report in /run/mpfree-app (Out::new and the STOP line in main)
            "OpenOptions::new().create(true).append(true).mode(0o644).open(Path::new(REPORT_DIR).join(\"report.txt\")).ok()",
            "if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(Path::new(REPORT_DIR).join(\"report.txt\")) {",
            # the framebuffer and its sysfs pan file (mpfree-ui's FbTarget)
            "let file = OpenOptions::new().write(true).open(fb).map_err(|e| format!(\"{}: {e}\", fb.display()))?;",
            "let _ = OpenOptions::new().write(true).open(sysfs.join(\"pan\")).and_then(|mut f| f.write_all(b\"0,0\\n\"));",
            # the host-only preview pictures (never on the MPC)
            "OpenOptions::new().write(true).create_new(true).open(path).and_then(|mut f| f.write_all(&png))"
            ".map_err(|e| format!(\"{}: {e}\", path.display()))?;",
            # the host-only display lists for the GPU benchmark (--display-lists, never on the MPC)
            "OpenOptions::new().write(true).create_new(true).open(&path).and_then(|mut f| f.write_all(&bytes))"
            ".map_err(|e| format!(\"{}: {e}\", path.display()))?;",
            # the ACVA playback device
            "let file = OpenOptions::new().read(true).write(true).custom_flags(sys::O_NONBLOCK).open(path)?;",
            # the ACVA capture device (the line input), opened only when the owner turns Line input on
            "let file = OpenOptions::new().read(true).write(true).custom_flags(sys::O_NONBLOCK).open(path)?;",
            # the controller's MIDI ports, output side: keep-alive, pad colours, button lights, Akai's setup
            "Direction::Output => opts.write(true),",
            # mpfree-store's NAME.part, made new (never through a link) under the --data folder
            "let mut f = OpenOptions::new().write(true).create_new(true).mode(0o644).open(&part)?;",
        )
        self.assertEqual(sorted(texts), sorted(expected), "\n".join(texts))
        files = {name for name, _ in found}
        self.assertEqual(files, {"mpfree-app/main.rs", "mpfree-app/alsa.rs", "mpfree-ui/fb.rs", "mpfree-store/lib.rs"},
                         files)
        # Folders are made only by the store, and only start-app.sh names the folder it is given.
        for name, code in self.program_sources():
            if name != "mpfree-store/lib.rs":
                self.assertNotIn("DirBuilder", code, name)
            for line in code.splitlines():
                if not line.lstrip().startswith("//"):
                    self.assertNotIn("/run/mpfree-data", line, name)

    def test_the_only_program_it_starts_is_a_read_only_journalctl(self):
        code = "".join(c for _, c in self.program_sources())
        self.assertEqual(code.count("Command::new("), 1)
        self.assertIn('Command::new("journalctl")', code)
        self.assertIn('.args(["-b", "-f", "-n", "300", "--no-pager", "-o", "short-monotonic", "--no-hostname"])', code)
        for needle in ("systemctl", "std::net", "kexec"):
            self.assertNotIn(needle, "\n".join(l for l in code.splitlines() if not l.lstrip().startswith("//")))

    def test_arp_patterns_are_saved_only_through_the_store(self):
        """The pad player has no --save-patterns any more: Save keeps the arp's patterns in mpfree-store
        (arp/patterns.txt under --data), and --patterns FILE is only read."""
        main = dict(self.program_sources())["mpfree-app/main.rs"]
        self.assertNotIn('"--save-patterns"', main)
        self.assertNotIn("fn write_patterns", main)
        self.assertIn("store.write(app::saved::ARP, app::saved::ARP_PATTERNS, text.into_bytes())", main)
        with open(os.path.join(os.path.dirname(HERE), "offline-kexec-stick.py")) as fh:
            starts = [ln for ln in fh.read().splitlines() if "mpfree app" in ln]
        self.assertEqual(len(starts), 2, starts)   # with the data partition mounted, and without
        for start in starts:
            self.assertNotIn("--save-patterns", start)

    def test_unsafe_code_only_in_sys(self):
        """The C calls (ioctl, fcntl, sched_setscheduler, mlockall) are the only unsafe code in MPFree's own source."""
        for name, code in self.program_sources():
            if name == "mpfree-app/main.rs":
                self.assertIn("#![deny(unsafe_code)]", code)
            if name in ("mpfree-ui/lib.rs", "mpfree-touch/lib.rs", "mpfree-screen-log/lib.rs"):
                self.assertIn("#![forbid(unsafe_code)]", code, name)
            if name != "mpfree-app/sys.rs":
                self.assertNotIn("unsafe {", code, name)
                self.assertNotIn("unsafe extern", code, name)
                self.assertNotIn("allow(unsafe_code)", code, name)
        with open(os.path.join(APP_SRC, "sys.rs")) as fh:
            sys_rs = fh.read()
        self.assertEqual(sorted(set(re.findall(r"\bfn (\w+)\(", sys_rs.split("extern \"C\" {")[1].split("}")[0]))),
                         ["fcntl", "ioctl", "mlockall", "sched_setscheduler"])

if __name__ == "__main__":
    unittest.main(verbosity=2)
