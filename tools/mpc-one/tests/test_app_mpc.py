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
              os.path.join(REPO, "rust", "mpfree-prof", "src"), os.path.join(REPO, "rust", "mpfree-store", "src"),
              os.path.join(REPO, "rust", "mpfree-library", "src"), os.path.join(REPO, "rust", "mpfree-browser-ui", "src"),
              # The first start on a creator stick (mpfree-app/setup.rs) reads the eMMC backup's copy on the stick with
              # these; they only read (the disk image writer, ext2w, writes only through a WriteAt it is handed).
              os.path.join(REPO, "rust", "mpfree-factory", "src"), os.path.join(REPO, "rust", "mpfree-diskimage", "src"))
# The two writers the storage spec allows (approved by Jay 12 Nov 00:56Z): mpfree-store under the folder it is
# given (settings/ and files/), and mpfree-library under the library/ folder in it. The library is linked since
# the Pads tab's browser (12 Nov 2026); its write sites are reviewed line by line below.
LIBRARY_NEEDLES = (
    # Reviewed by the app thread, 12 Nov 2026. Every path is the library's root joined with a place's folder and
    # names checked by valid_name (Library::path), or a .part / temp / trash slot beside one; links are refused
    # (symlink_metadata, O_NOFOLLOW). The Pads tab's browser reaches none of these: its picker refuses every
    # button but Load, Load to pad, Open, Audition and Favourite (Pick::allows), which read; Recent and
    # Favourites are saved through mpfree-store. The full browser (BROWSE, 13 Nov 2026) reaches them all:
    # New folder, Rename, Copy and Paste, Delete into the Trash (asks first; Put back undoes it), Empty
    # Trash (asks first; for good, the Trash only) and the project save (SHIFT + BROWSE, Save, Save as),
    # each inside library/ under --data and nowhere else. At start, Library::at makes library/ and its
    # place folders when they are missing, and work_dir makes library/.work/<owner> (mkdir, the DirBuilder
    # check below), on the --data folder only.
    ("mpfree-library/lib.rs", "fs::rename(&part, fp.join(&name))?;"),                  # write_new: .part renamed in
    ("mpfree-library/lib.rs", "let _ = fs::remove_file(&part);"),                       # write_new, copy: a cut-off .part
    ("mpfree-library/lib.rs", "fs::rename(&p, &tp)?;"),                                 # rename in place
    ("mpfree-library/lib.rs", "let r = copy_tree(&src, &tmp, 0).and_then(|_| fs::rename(&tmp, &dst));"),  # copy a folder
    ("mpfree-library/lib.rs", "let _ = fs::remove_dir_all(&tmp);"),                     # copy: a cut-off temp folder
    ("mpfree-library/lib.rs", "let r = copy_file(&src, &part).and_then(|_| fs::rename(&part, &dst));"),  # copy a file
    ("mpfree-library/lib.rs", "fs::rename(&src, dst_dir.join(&name))?;"),               # move_to
    ("mpfree-library/lib.rs", "fs::rename(&src, slot.join(loc.name()))?;"),             # delete: into a Trash slot
    ("mpfree-library/lib.rs", "let _ = fs::remove_dir_all(&slot);"),                    # delete: a slot left half made
    ("mpfree-library/lib.rs", "fs::rename(slot.join(&item.name), dst_dir.join(&name))?;"),  # restore from the Trash
    ("mpfree-library/lib.rs", "let _ = fs::remove_file(slot.join(\"origin.txt\"));"),   # restore: the slot's note
    ("mpfree-library/lib.rs", "let _ = fs::remove_dir(&slot);"),                        # restore: the empty slot
    ("mpfree-library/lib.rs", "fs::remove_dir_all(&p)?;"),                              # empty_trash (Prefs, browser), Trash only
    ("mpfree-library/lib.rs", "fs::remove_file(&p)?;"),                                 # empty_trash (Prefs, browser), Trash only;
                                                                                        # clear_work: files (not links) in .work/<owner>
    ("mpfree-library/project.rs", "fs::remove_dir_all(&new)?;"),                        # project save: a stale parts.new
    ("mpfree-library/project.rs", "fs::rename(&parts, v.join(PARTS))?;"),               # project save: old parts kept
    ("mpfree-library/project.rs", "fs::rename(&new, &parts)?;"),                        # project save: new parts in
    ("mpfree-library/project.rs", "fs::rename(&part, dir.join(PROJECT_FILE))?;"),       # project save: project.txt in
    ("mpfree-library/project.rs", "let _ = fs::remove_file(dir.join(format!(\"{PROJECT_FILE}.part\")));"),
    ("mpfree-library/project.rs", "let _ = fs::remove_dir_all(dir.join(VERSIONS).join(&old.id));"),  # versions past 5
    ("mpfree-library/project.rs", "fs::rename(&tmp, &dst)?;"),                          # project samples copied in
    ("mpfree-library/project.rs", "fs::remove_file(&tmp)?;"),
    ("mpfree-library/project.rs", "let _ = fs::remove_file(&tmp);"),
    ("mpfree-library/project.rs", "fs::rename(&tmp, &dst)"),                            # a part file written
)
LIBRARY_OPENS = (
    "let mut f = OpenOptions::new().write(true).create(true).truncate(true).mode(0o644).custom_flags(0x20000) "
    "// O_NOFOLLOW.open(&part)?;",                                                      # write_new's .part
    "let mut f = OpenOptions::new().write(true).create_new(true).mode(0o644).open(slot.join(\"origin.txt\"))?;",
    "let mut o = OpenOptions::new().write(true).create_new(true).mode(0o644).open(dst)?;",  # copy_file
    "let mut f = OpenOptions::new().write(true).create(true).truncate(true).mode(0o644).custom_flags(0x20000) "
    "// O_NOFOLLOW.open(path)?;",                                                       # project.txt
    "} let mut o = OpenOptions::new().write(true).create(true).truncate(true).mode(0o644).open(&tmp)?;",
    "let mut f = OpenOptions::new().write(true).create(true).truncate(true).mode(0o644).custom_flags(0x20000) "
    "// O_NOFOLLOW.open(&tmp)?; {",
)
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
  show)
    case "$3" in
      LoadState) if [ -n "$FAKE_SPLASH" ]; then echo loaded; else echo not-found; fi ;;
      ActiveState) echo inactive ;;
      Result) echo success ;;
      *) exit 99 ;;
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
        for text in ("static ARM EABI5 hard-float", f"release: {NEW}",
                     "started by: no soft-reboot marker: a kexec (the boot hook's opt-in kexec_file route or its chain, "
                     "or --kexec-boot on 3.6.0), or a direct boot", "no MMC block device", "/: read-only",
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

    def test_says_when_the_boot_hook_started_the_test_build(self):
        # After a soft-reboot the kernel and release are Akai's; the hook's marker in /run says how it started.
        write(f"{self.root}/run/mpfree-hook/soft-reboot", "6.05\n")
        res = self.run_helper()
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("started by: the boot hook's soft-reboot (asked at 6.05 s after power-on)", res.stdout)
        self.assertIn("CHECKS-OK", res.stdout)

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

    def test_app_step_with_no_checks_asks_nothing(self):
        """Jay, 15 Nov: no leftover questions. With no checks named the app step only reports, from the log."""
        write(self.app, "active\n")
        write(self.screen, "inactive\n")
        self.add_journal([(5.1, "systemd[1]: Started MPFree test boot: the pad player, from power-on."),
                          (6.0, "mpfree[211]: pad player: 6 kits, 81 sounds"),
                          (6.5, "mpfree[211]: saved settings: pad curve Hard, sound cores shared, loop memory 5 min, x"),
                          (9.5, "mpfree[211]: playing until the power goes off: ..."),
                          (40.0, "mpfree[211]: status: 60 hits (0 from the screen), 40 repeats, 0 underruns in "
                                 "20000 periods, ...")])
        res = self.run_helper("", MPFREE_APP_STEPS="app")
        self.assertEqual(res.returncode, 0, res.stdout)
        out = res.stdout
        for text in ("steps: app", "[3/3] The pad player that started at power-on (read-only",
                     "pad player (mpfree-test-app.service): active",
                     "start-up: playing 9.5 s after power-on; startup screen: not on this stick",
                     "    saved settings: pad curve Hard, sound cores shared, loop memory 5 min, x",
                     "no PANIC or safe start lines",
                     "No checks named (MPFREE_APP_CHECKS), so nothing is asked",
                     "Session summary (the whole run since power-on",
                     "    status: 60 hits (0 from the screen), 40 repeats, 0 underruns in 20000 periods",
                     "RESULT: all steps ran"):
            self.assertIn(text, out)
        for text in ("Ready?", "your answer", "Phase ", "Check 1", "startup screen play"):
            self.assertNotIn(text, out)
        self.assertNotIn("pad player: 6 kits", out, "the start-up's whole log goes to the report only")
        with open(self.report) as fh:
            self.assertIn("| [     6.000] pad player: 6 kits, 81 sounds", fh.read())
        # Nothing was copied, started or stopped; only reads.
        self.assertFalse(os.path.exists(f"{self.root}/run/mpfree-app"))
        self.assertEqual(self.log_lines("program: "), [])
        self.assertEqual([l for l in self.log_lines("systemctl: ")
                          if not l.startswith(("is-active ", "show -p "))], [])
        self.assertTrue(all(l.startswith("-b -u mpfree-test-app.service -o short-monotonic")
                            for l in self.log_lines("journalctl: ")))

    def transport_lines(self, t):
        """What the app logs for the transport check's steps, from uptime t on."""
        lines = [(0.0, "button 73 pressed"), (0.1, "looper: Record: armed, the first pass starts on the next bar "
                                                   "line (2 bars)"),
                 (1.0, "button 82 pressed"), (5.0, "button 80 pressed"), (5.1, "looper: overdub on (layer 2)"),
                 (7.0, "button 80 pressed"), (7.1, "looper: overdub off"), (8.0, "button 67 pressed"),
                 (8.1, "looper: Undo: layer 2 fades out"), (9.0, "button 81 pressed")]
        return [(t + dt, "mpfree[211]: " + (f"+{t + dt:.3f} s Private: 90 00 7F = {m}" if m.startswith("button")
                                             else m)) for dt, m in lines]

    def test_app_step_runs_only_the_named_checks_and_reads_the_log(self):
        """MPFREE_APP_CHECKS: only those checks, numbered of their own count; the log says what was seen and
        each check asks its one question."""
        write(self.app, "active\n")
        r = self.root
        write(f"{r}/proc/interrupts", " 61:        42          0  rockchip_gpio_irq   6 Edge      ili2117\n")
        write(f"{r}/run/mpfree-data/settings/map.conf", "pad-bank-a-e = button 35\npad-bank-b-f = button 36\n"
              "track-mix-pad-mix = button 115\nmain = button 52\nsampler = button 51\n")
        # The start-up read moves the fake clock to 110; check 1 starts there and its lines are due at 120;
        # check 2 starts at 120 and its lines are due at 130.
        self.add_journal(self.transport_lines(111.0), at=120)
        self.add_journal([(121.0, "mpfree[211]: +1.000 s Private: 99 25 12 = pad 1 down, velocity 18"),
                          (121.5, "mpfree[211]: +1.500 s Private: 99 26 7C = pad 2 down, velocity 124"),
                          (122.0, "mpfree[211]: bank: B (kit 2: Lo-fi)"),
                          (122.1, "mpfree[211]: pad level: Full Level (every hit 127)"),
                          (122.2, "mpfree[211]: pad level: as played"),
                          (123.0, "mpfree[211]: +3.000 s Private: B0 64 01 = data wheel +1"),
                          (124.0, "mpfree[211]: +4.000 s Private: B0 10 05 = Q-Link 1 turned +5")], at=130)
        answers = "\n\nlights right\n" + "\n\npads fine\n"
        res = self.run_helper(answers, MPFREE_APP_STEPS="app", MPFREE_APP_CHECKS="transport, basics")
        self.assertEqual(res.returncode, 0, res.stdout)
        out = res.stdout
        for text in ("Checks: transport, basics",
                     "Check 1 of 2: REC, OVERDUB, UNDO and PLAY on a fresh start (transport)",
                     "for: #387", "1. Press REC once: REC and PLAY blink.",
                     "seen      REC (73) armed the loop (button 73 x1)",
                     "seen      OVERDUB (80) turned overdub on (button 80 x2)",
                     "seen      UNDO (67) took the layer back", "seen      STOP (81) reached the app",
                     "Did the lights do what each step says?", "your answer: lights right",
                     "Check 2 of 2: Pads, banks, Full Level, the wheel, Q-Links and touch reach the app (basics)",
                     "NOT SEEN  pad hits, soft and hard (2 hits on 2 pads, velocities 18 to 124)",
                     "seen      PAD BANK changed the bank", "seen      FULL LEVEL on and off",
                     "seen      the data wheel (1 wheel steps, net +1)", "seen      the Q-Links (1 Q-Link messages)",
                     "NOT SEEN  the touchscreen (touch interrupts 42 -> 42)",
                     "The checks:", "transport: 6 of 6 seen in the log; your answer: lights right",
                     "basics: 4 of 6 seen in the log; your answer: pads fine", "RESULT: all steps ran"):
            self.assertIn(text, out)
        for text in ("Phase ", "of 19", "startup screen play", "Map controls has not found"):
            self.assertNotIn(text, out)
        self.assertEqual(out.count("Enter:   your answer:"), 2)
        self.assertEqual(self.log_lines("program: "), [])

    def test_auto_checks_read_the_stick_and_ask_nothing(self):
        """kept and hour: no steps and no question; partition 4's settings and the status lines decide."""
        write(self.app, "active\n")
        r = self.root
        write(f"{r}/run/mpfree-data/settings/app.conf", "curve = hard\nsound-cores = two\nloop-memory = 10\n")
        self.add_journal([(6.5, "mpfree[211]: saved settings: pad curve Hard, screen velocity 100, developer tools "
                                "off, theme Synthwave, sound cores two, loop memory 10 min, metronome off at -12 dB"),
                          (6.6, "mpfree[211]: sound cores: two: the audio thread on core 3, lanes on core 2"),
                          (3600.0, "mpfree[211]: status: 900 hits (0 from the screen), 0 repeats, 0 underruns in "
                                   "1240100 periods, ..."),
                          (3630.0, "mpfree[211]: status: 900 hits (0 from the screen), 0 repeats, 0 underruns in "
                                   "1250440 periods, ...")])
        res = self.run_helper("", MPFREE_APP_STEPS="app", MPFREE_APP_CHECKS="kept,hour")
        self.assertEqual(res.returncode, 0, res.stdout)
        out = res.stdout
        for text in ("Check 1 of 2: The settings partition 4 kept came back at this start (kept)",
                     "seen      the start's settings agree with settings/app.conf (sound cores two, loop memory 10 min; "
                     "the same in settings/app.conf)",
                     "Check 2 of 2: An hour of play with no underrun (hour)",
                     "seen      an hour, no underrun, no restart (0 underruns in 1,250,440 periods (60 min))",
                     "kept: 1 of 1 seen in the log", "hour: 1 of 1 seen in the log"):
            self.assertIn(text, out)
        for text in ("Ready?", "your answer"):
            self.assertNotIn(text, out)
        # Not an hour yet, a restart, and a file that disagrees.
        write(self.journal, "")
        write(f"{r}/run/mpfree-data/settings/app.conf", "sound-cores = shared\n")
        self.add_journal([(6.5, "mpfree[211]: saved settings: pad curve Hard, sound cores two, loop memory 10 min, x"),
                          (600.0, "mpfree[211]: status: 9 hits (0 from the screen), 0 repeats, 0 underruns in "
                                  "200000 periods, ...")])
        res = self.run_helper("", MPFREE_APP_STEPS="app", MPFREE_APP_CHECKS="kept,hour")
        out = res.stdout
        self.assertIn("NOT SEEN  the start's settings agree with settings/app.conf (sound cores two, loop memory "
                      "10 min; the file says sound cores shared)", out)
        self.assertRegex(out, r"unclear   an hour, no underrun, no restart \(0 underruns in 200,000 periods "
                              r"\(10 min\); NOT YET an hour: run this check again in \d+ min\)")
        self.add_journal([(630.0, "mpfree[211]: status: 1 hits (0 from the screen), 0 repeats, 0 underruns in "
                                  "3000 periods, ...")])
        res = self.run_helper("", MPFREE_APP_STEPS="app", MPFREE_APP_CHECKS="hour")
        self.assertIn("NOT SEEN  an hour, no underrun, no restart (0 underruns in 3,000 periods (0 min); the period "
                      "count went back, so the pad player restarted)", res.stdout)

    def test_a_check_says_which_buttons_map_controls_has_not_found(self):
        write(self.app, "active\n")
        write(f"{self.root}/run/mpfree-data/settings/map.conf", "pad-bank-a-e = button 35\ntrack-mute = skipped\n")
        res = self.run_helper("s\ns\n", MPFREE_APP_STEPS="app", MPFREE_APP_CHECKS="track-mute")
        self.assertEqual(res.returncode, 0, res.stdout)
        out = res.stdout
        for text in ("Map controls has not found these buttons yet, and a step uses them",
                     "    TRACK MUTE (check track-mute)", "    PAD BANK B/F (check track-mute)",
                     "So map-controls runs first, to find them.", "Checks: map-controls, track-mute",
                     "Check 1 of 2: Map controls", "map-controls: skipped", "track-mute: skipped"):
            self.assertIn(text, out)
        self.assertNotIn("PAD BANK A/E (check", out)
        # Named already: it stays where the round put it, and nothing is added.
        res = self.run_helper("s\ns\n", MPFREE_APP_STEPS="app", MPFREE_APP_CHECKS="track-mute,map-controls")
        self.assertIn("Checks: track-mute, map-controls", res.stdout)
        self.assertNotIn("So map-controls runs first", res.stdout)
        # Every button found: nothing is said and nothing added.
        write(f"{self.root}/run/mpfree-data/settings/map.conf",
              "pad-bank-a-e = button 35\npad-bank-b-f = button 36\ntrack-mute = button 43\n")
        res = self.run_helper("s\n", MPFREE_APP_STEPS="app", MPFREE_APP_CHECKS="track-mute")
        self.assertNotIn("Map controls has not found", res.stdout)
        self.assertIn("Checks: track-mute\n", res.stdout)

    def test_a_check_stops_when_the_input_ends(self):
        """No more input (the terminal closed) stops at the next question, as q does, and does not go on."""
        write(self.app, "active\n")
        res = self.run_helper("", MPFREE_APP_STEPS="app", MPFREE_APP_CHECKS="load")
        self.assertIn("Check 1 of 1:", res.stdout)
        self.assertIn("answer: quit", res.stdout)
        self.assertNotIn("Go ahead.", res.stdout)
        self.assertIn("stopped by you", res.stdout)

    def test_the_load_check_waits_for_a_status_line_and_reads_the_cores(self):
        """load: no underrun between the status line before it and the one after; then the cores, once."""
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
        self.add_journal([(100.0, "mpfree[211]: status: 10 hits (0 from the screen), 0 repeats, 2 underruns in "
                                  "30000 periods, ...")])
        # The start-up read moves the clock to 110; the check starts there; the wait for a status line reads
        # at 120, the check's own read at 130.
        self.add_journal([(111.0 + k / 10, f"mpfree[211]: +1.000 s Private: 99 25 7F = pad {k} down, velocity 127")
                          for k in range(1, 9)], at=120)
        self.add_journal([(125.0, "mpfree[211]: status: 400 hits (0 from the screen), 300 repeats, 2 underruns in "
                                  "40000 periods, ...")], at=120)
        res = self.run_helper("\n\nno crackles\n", MPFREE_APP_STEPS="app", MPFREE_APP_CHECKS="load")
        self.assertEqual(res.returncode, 0, res.stdout)
        out = res.stdout
        for text in ("Check 1 of 1: The heaviest sounds on the MPC's own cores (load)",
                     "waiting up to 40 s for the app's next status line",
                     "seen      pads held (8 hits on 8 pads, velocities 127 to 127)",
                     "seen      no underrun while it played (0 underruns in 10,000 periods (29 s around the check))",
                     "cores, clock and threads now (read-only):", "/sys/devices/system/cpu/isolated: 2-3",
                     "/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor: interactive",
                     "current_clocksource: arch_sys_counter", "app pid 900",
                     "900 mpfree: allowed \t0-1 last_cpu=1 rt_priority=70 policy=1",
                     "903 mpfree-screen: allowed \t0-1 last_cpu=1", "app processes: 1",
                     "/sys/devices/system/cpu/cpu[0-3]/cpufreq/scaling_cur_freq: not there",
                     "your answer: no crackles", "load: 2 of 2 seen in the log"):
            self.assertIn(text, out)
        self.assertEqual(out.count("cores, clock and threads now"), 1)
        with open(self.report) as fh:
            self.assertIn("|  61:         42          0  ili2117", fh.read())

    def test_app_step_refuses_unknown_checks_and_the_old_phases(self):
        for env, said in (({"MPFREE_APP_CHECKS": "transport,nope"}, "unknown check(s) nope"),
                          ({"MPFREE_APP_PHASES": "1-3"}, "MPFREE_APP_PHASES is gone"),
                          ({"MPFREE_APP_STEPS": "inventory", "MPFREE_APP_CHECKS": "transport"},
                           "MPFREE_APP_CHECKS goes with the app step")):
            res = self.run_helper("", **dict({"MPFREE_APP_STEPS": "app"}, **env))
            self.assertNotEqual(res.returncode, 0, res.stdout)
            self.assertIn(said, res.stdout)
            self.assertNotIn("[2/", res.stdout, "nothing reached the MPC")

    def test_targeted_run_on_a_soft_booted_stick(self):
        """The targeted app run's block (app-targeted-run.md) on the stick's system after the boot hook's
        soft-reboot: / is the stick's partition mounted ro,noload, /run is Akai's tmpfs carried over, and
        kexec_load_disabled comes from the stick's sysctl.d. The checks pass and only the run's checks ask."""
        self.make_root(run_opts="rw,nosuid,nodev,size=404268k,nr_inodes=819200,mode=755")
        with open(os.path.join(self.root, "proc/mounts"), "w") as fh:
            fh.write("/dev/sda1 / ext4 ro,relatime,noload 0 0\n"
                     "tmpfs /run tmpfs rw,nosuid,nodev,size=404268k,nr_inodes=819200,mode=755 0 0\n"
                     "tmpfs /data tmpfs rw,size=262144k,mode=755 0 0\n")
        write(self.app, "active\n")
        write(f"{self.root}/run/mpfree-data/settings/map.conf",
              "".join(f"{k} = button {n}\n" for n, k in enumerate(
                  ("track-mute", "pad-bank-a-e", "pad-bank-b-f", "data-dial-push", "main", "sampler", "erase"), 40)))
        chosen = "track-mute,wheel,touch,note-repeat,looper,console"
        answers = "\n\nfine\n" * 6
        res = self.run_helper(answers, MPFREE_APP_STEPS="app", MPFREE_APP_CHECKS=chosen)
        self.assertEqual(res.returncode, 0, res.stdout)
        out = res.stdout
        for text in ("/: read-only", "kexec_load_disabled: 1", "no MMC block device", "CHECKS-OK",
                     "Checks: track-mute, wheel, touch, note-repeat, looper, console",
                     "Check 6 of 6: The console chord, TAP TEMPO and STOP (console)",
                     "Press TRACK MUTE: it lights", "drag the kick's clip to", "Tap a block, push the wheel, turn it"):
            self.assertIn(text, out)
        self.assertEqual(out.count("Enter:   your answer: fine"), 6)
        self.assertEqual(out.count("; your answer: fine"), 6, "and once each in the checks' list")
        self.assertNotIn("of 19", out)
        self.assertIn("RESULT: all steps ran", out)
        # The app step copies, starts and stops nothing.
        self.assertEqual(self.log_lines("program: "), [])

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

    def test_app_step_reads_the_pad_players_privileges(self):
        """Read-only: the pad player's user, groups, capabilities and limits, the device nodes' owners, the pan
        and eMMC bind files, and the groups it needs (for a build with and without --app-user)."""
        write(self.app, "active\n")
        r = self.root
        res = self.run_helper("\nq\n", MPFREE_APP_STEPS="app")
        self.assertIn("privileges: no mpfree process", res.stdout)
        write(f"{r}/proc/733/comm", "mpfree\n")
        write(f"{r}/proc/733/status", "Name:\tmpfree\nUid:\t960\t960\t960\t960\nGid:\t960\t960\t960\t960\n"
              "Groups:\t29 44 101 960\nCapEff:\t0000000000000000\nCapBnd:\t0000000000000000\nNoNewPrivs:\t1\n"
              "VmRSS:\t100 kB\n")
        write(f"{r}/proc/733/limits", "Limit                     Soft Limit           Hard Limit           Units\n"
              "Max locked memory         unlimited            unlimited            bytes\n"
              "Max realtime priority     70                   70\n"
              "Max open files            1024                 4096                 files\n")
        write(f"{r}/dev/fb0", "")
        write(f"{r}/dev/snd/pcmC0D0p", "")
        write(f"{r}/sys/class/graphics/fb0/pan", "0,0\n")
        write(f"{r}/sys/bus/platform/drivers/dwmmc_rockchip/bind", "")
        write(f"{r}/etc/group", "root:x:0:\naudio:x:29:mpfree\nvideo:x:44:mpfree\ndisk:x:6:\nmpfree:x:960:\n")
        res = self.run_helper("\nq\n", MPFREE_APP_STEPS="app")
        for text in ("privileges: pid 733 Uid:\t960\t960\t960\t960", "privileges: pid 733 CapEff:\t0000000000000000",
                     "privileges: pid 733 NoNewPrivs:\t1"):
            self.assertIn(text, res.stdout)
        self.assertNotIn("privileges: group", res.stdout, "the rest goes to the report")
        with open(self.report) as fh:
            out = fh.read()
        for text in ("privileges: pid 733 Uid:\t960\t960\t960\t960", "privileges: pid 733 Groups:\t29 44 101 960",
                     "privileges: pid 733 CapEff:\t0000000000000000", "privileges: pid 733 NoNewPrivs:\t1",
                     "privileges: pid 733 Max realtime priority     70",
                     "privileges: pid 733 Max locked memory         unlimited",
                     "privileges: group audio:x:29:mpfree", "privileges: group mpfree:x:960:"):
            self.assertIn(text, out)
        for path in ("/dev/fb0", "/dev/snd/pcmC0D0p", "/sys/class/graphics/fb0/pan",
                     "/sys/bus/platform/drivers/dwmmc_rockchip/bind"):
            self.assertRegex(out, rf"privileges: {re.escape(path)} -rw\S* uid \d+ gid \d+")
        self.assertNotIn("VmRSS", out)
        self.assertNotIn("Max open files", out)
        self.assertNotIn("disk:x:6", out)

    def test_app_step_reads_the_gpu_and_display_read_only(self):
        """Read-only: the GPU's libraries, the release, /dev/dri, the panel's mode, who holds /dev/dri,
        Panfrost, and debugfs only where it is already there; a missing path says so."""
        write(self.app, "active\n")
        r = self.root
        res = self.run_helper("\nq\n", MPFREE_APP_STEPS="app")
        self.assertRegex(res.stdout, r"\(the GPU read: \d+ lines, in the report\)")
        self.assertNotIn("gpu: ", res.stdout, "the GPU read goes to the report only")
        with open(self.report) as fh:
            first = fh.read()
        for text in ("gpu: no libEGL, libGLESv2, libgbm, libdrm or /usr/lib/dri",
                     "gpu: /etc/os-release not there", "gpu: /dev/dri not there",
                     "gpu: no /sys/class/drm/card*-* connectors", "gpu: nothing has /dev/dri open",
                     "gpu: panfrost driver not there",
                     "gpu: debugfs dri/0 not there (it is not mounted for this)"):
            self.assertIn(text, first)
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
        self.assertNotIn("plane[0]", res.stdout)
        with open(self.report) as fh:
            out = fh.read()
        for text in ("gpu: /usr/lib/libEGL.so.1 -> libEGL.so.1.0.0", "gpu: /usr/lib/libEGL.so.1.0.0",
                     "gpu: /usr/lib/dri/rockchip_dri.so", 'gpu: os-release NAME="Akai"',
                     "gpu: os-release VERSION_ID=3.10.0", "gpu: /dev/dri/card0",
                     "gpu: /sys/class/drm/card0-DSI-1: connected, modes: 800x1280",
                     "gpu: /dev/dri is open in pid 412 (weston)",
                     "gpu: panfrost driver bound to: ff9a0000.gpu",
                     "gpu: debugfs dri/0 state (first 80 lines):", "gpu:   plane[0]: x", "gpu:   plane[79]: x"):
            self.assertIn(text, out)
        self.assertNotIn("HOME_URL", out)
        self.assertNotIn("plane[80]", out, "80 debugfs lines at most")

    def test_app_step_summary_reads_the_whole_session(self):
        """No checks: the start-up and the summary still show the whole run, the touch interrupts and the status."""
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
        res = self.run_helper("", MPFREE_APP_STEPS="app", FAKE_SPLASH="1")
        self.assertEqual(res.returncode, 0, res.stdout)
        out = res.stdout
        for text in ("startup screen: inactive, result success",
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
        self.assertEqual([l for l in self.log_lines("systemctl: ")
                          if not l.startswith(("is-active ", "show -p "))], [])

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
        res = self.run_helper("", MPFREE_APP_STEPS="app", MPFREE_APP_CHECKS="transport")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("pad player (mpfree-test-app.service): inactive", res.stdout)
        self.assertIn("    STOP: no sound card called ACVA after 30 s", res.stdout)
        self.assertIn("The pad player is not running, so no check runs.", res.stdout)
        self.assertNotIn("Check 1", res.stdout)

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
        self.assertIn("switch it on with the stick in", res.stdout)
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
                 "APP_START", "APP_STATE", "UPTIME", "APP_LOG", "TOUCH_IRQ", "REPORT", "CONTROLLER", "LINE_PROBE",
                 "PRIVS_STATE")
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
        for name in ("APP_STATE", "UPTIME", "APP_LOG", "TOUCH_IRQ", "PRIVS_STATE"):
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
            text = text.replace("#[cfg(test)]\npub(crate) mod tests {", "#[cfg(test)]\nmod tests {")
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
                    ("mpfree-store/lib.rs", "let _ = fs::remove_file(&part);"),
                    # The host-only Pads tab picture (--preview and --display-lists, never on the MPC): the scratch
                    # library pads_browse made a moment before in the workstation's temp folder, removed again.
                    ("mpfree-app/gui.rs", "let _ = std::fs::remove_dir_all(&dir);"),
                    # The safe start (crash.rs): /run/mpfree-app/running, in RAM beside the report, holds the pid
                    # while the app runs and says "ended" when the run ends well; a stop's handler unlinks it
                    # (sys.rs, below).
                    ("mpfree-app/crash.rs", "fs::write(path, format!(\"pid {}\\n\", std::process::id()))?;"),
                    ("mpfree-app/crash.rs", "let _ = fs::write(&self.path, ENDED);"),
                    # The first start on a creator stick (setup.rs): the backup's copy of the eMMC's user area is a
                    # FILE on partition 4 (backup/<folder>/mmcblk0.img, made by the stick's root service), opened
                    # read-only with File::open; the pad player never names an eMMC device.
                    ("mpfree-app/setup.rs", 'folder.join("mmcblk0.img")'),
                    # The power card's request (power.rs): one word, `poweroff` or `restart`, written to
                    # power-request.new beside the file given with --power-request (start-app.sh gives
                    # /run/mpfree-app/power-request, in RAM), synced, then renamed over it for the stick's root
                    # path unit to read.
                    ("mpfree-app/power.rs", "let mut f = fs::File::create(&tmp)?;"),
                    ("mpfree-app/power.rs", "fs::rename(&tmp, path)")} | set(LIBRARY_NEEDLES)
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
            # a panic's line, appended to the report and to the safe start's file, both in /run/mpfree-app
            # (crash.rs: note_panics, called with those two paths only)
            "let to_report = OpenOptions::new().create(true).append(true).open(&report);",
            "let to_running = OpenOptions::new().append(true).open(&running);",
            # the framebuffer and its sysfs pan file (mpfree-ui's FbTarget)
            "let file = OpenOptions::new().write(true).open(fb).map_err(|e| format!(\"{}: {e}\", fb.display()))?;",
            # (FbTarget::show, at open and again after the GPU's process has had the screen)
            "let _ = OpenOptions::new().write(true).open(&self.pan).and_then(|mut f| f.write_all(b\"0,0\\n\"));",
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
            # setup.rs: the first start's request for the eMMC backup, an empty file made new in /run/mpfree-app
            # (RAM), which the stick's mpfree-setup-backup.path watches (Paths::on_the_stick, with REPORT_DIR)
            "match OpenOptions::new().write(true).create_new(true).mode(0o644).open(&paths.request) {",
        )
        expected += LIBRARY_OPENS
        self.assertEqual(sorted(texts), sorted(expected), "\n".join(texts))
        files = {name for name, _ in found}
        self.assertEqual(files, {"mpfree-app/main.rs", "mpfree-app/crash.rs", "mpfree-app/alsa.rs", "mpfree-ui/fb.rs",
                                 "mpfree-app/setup.rs", "mpfree-store/lib.rs",
                                 "mpfree-library/lib.rs", "mpfree-library/project.rs"}, files)
        # Folders are made only by the store and the library, and only start-app.sh names the folder they are given.
        for name, code in self.program_sources():
            if name not in ("mpfree-store/lib.rs", "mpfree-library/lib.rs"):
                self.assertNotIn("DirBuilder", code, name)
            for line in code.splitlines():
                if not line.lstrip().startswith("//"):
                    self.assertNotIn("/run/mpfree-data", line, name)

    def test_it_starts_only_a_read_only_journalctl_and_the_gpu_process_it_is_given(self):
        code = "".join(c for _, c in self.program_sources())
        self.assertEqual(code.count("Command::new("), 2)
        self.assertIn('Command::new("journalctl")', code)
        # The GPU drawing process (mpfree_ui::gpu_screen), only with --gpu-screen PROGRAM: that program, with
        # the app's frames on its stdin and its answers on its stdout.
        gpu = dict(self.program_sources())["mpfree-ui/gpu_screen.rs"]
        self.assertIn("Command::new(program)", gpu)
        main = dict(self.program_sources())["mpfree-app/main.rs"]
        self.assertIn('"--gpu-screen" => o.gpu_screen = Some(PathBuf::from(value()?)),', main)
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
        # With the data partition mounted, and without; in start-app.sh as root and as --app-user's mpfree.
        self.assertEqual(len(starts), 4, starts)
        for start in starts:
            self.assertNotIn("--save-patterns", start)

    def test_unsafe_code_only_in_sys(self):
        """The C calls (ioctl, fcntl, sched_setscheduler, mlockall, sync for the power card, and signal, raise and unlink for the safe
        start's stop handler) are the only unsafe code in MPFree's own source."""
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
                         ["fcntl", "ioctl", "mlockall", "raise", "sched_setscheduler", "signal", "sync", "unlink"])
        # The stop handler unlinks only the file crash.rs gives it: /run/mpfree-app/running.
        main = dict(self.program_sources())["mpfree-app/main.rs"]
        crash = dict(self.program_sources())["mpfree-app/crash.rs"]
        self.assertEqual(crash.count("sys::remove_on_stop("), 1)
        self.assertIn("let path = dir.join(RUNNING);", crash)
        self.assertEqual(crash.count("mark(dir, &path)"), 1)
        self.assertIn('pub const RUNNING: &str = "running";', crash)
        self.assertEqual(main.count("crash::start("), 1)
        self.assertIn("crash::start(Path::new(REPORT_DIR))", main)
        self.assertEqual(main.count("crash::note_panics("), 1)
        self.assertEqual(crash.count("panic::set_hook("), 1)
        self.assertIn('Path::new(REPORT_DIR).join("report.txt"), Path::new(REPORT_DIR).join(crash::RUNNING)', main)

if __name__ == "__main__":
    unittest.main(verbosity=2)
