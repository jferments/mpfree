#!/usr/bin/env python3
"""Tests for setup-sd-card-write.sh, the SD card's write window on the stick's system (step S2a of the SD card
plan, docs/dev/hardware/sd-card-and-internal-drive.md). No device.

It runs under dash with BusyBox applets against test_sd_card_window.py's fake kernel: the SD card controller
bound and unbound through FIFOs, the card's ro files following its force_ro file (or held at 1, as a card
with its lock switch on), /dev files holding its first sector. Stand-ins: fsck.exfat (logs whether the disk
was read-only when it ran, answers as told), mount and umount. The mount stand-in keeps /proc/mounts as the
kernel shows it and makes a mount visible the way the kernel would: a mount point becomes a link to the
filesystem's folder (the card's files live in a folder of their own), a bind of a folder shows that folder,
and the unmount gives back the empty mount point. The kernel's in-use mark is played by the tests (set while
open) and by the unmount stand-in (cleared at the last unmount of the card, unless told otherwise). Set
MPFREE_REQUIRE_BUSYBOX=1 (CI does) to fail instead of skip without BusyBox or dash.
"""
import os
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from test_stick_setup import DD_REFUSES, EMMC, SD, STAT, STICK, need_busybox, needs, program_dir, read_text, write  # noqa: E402
from test_sd_card_window import CARD_DIR, EMMC_DIR, RO_RULE, RULE, FakeSdKernel  # noqa: E402

WINDOW = os.path.join(STICK, "setup-sd-card-write.sh")
MNT = "/run/mpfree-sd"
MPF = "/run/mpfree-sd/MPFree"
PRIV = "/run/mpfree-sd-write"
RW = "rw,nosuid,nodev,noexec,noatime,dirsync,uid=0,gid=960,fmask=0117,dmask=0007,iocharset=utf8,errors=remount-ro"
ASK = "0a1b2c3d4e5f60718293a4b5c6d7e8f9"
PROC = "/proc/4242"


class WriteKernel(FakeSdKernel):
    """The fake kernel, with a card whose lock switch can be on (`locked`: ro stays 1 whatever force_ro says)."""

    def __init__(self, root, locked=False, **kw):
        self.locked = locked
        kw.setdefault("card", "exfat")
        super().__init__(root, **kw)

    def insert(self):
        super().insert()
        if self.locked:
            for rel in ("ro", "mmcblk1p1/ro"):
                path = os.path.join(self.root, CARD_DIR, rel)
                if os.path.lexists(path):
                    os.unlink(path)
                    write(path, "1\n")

    def written(self, sectors):
        """The card's disk has written this many sectors (stat field 7)."""
        write(os.path.join(self.root, CARD_DIR, "stat"), STAT.format(w=sectors // 8, ws=sectors, d=0))


# fsck.exfat -V | -n DEVICE | -y DEVICE: logs whether the card's disk was read-only, answers FAKE_FSCK (the
# first -n) and FAKE_FSCK_AFTER (the later ones), or FAKE_FSCK_SEQ (one code per -n, in order: "4,0,0");
# FAKE_FSCK_WRITES makes it bump the card's write counter. -y (the repair) clears the in-use mark and the
# disk-error bit (byte 106, bits 1 and 2) unless FAKE_REPAIR_KEEPS, and answers FAKE_REPAIR (default 1, "mended").
FAKE_FSCK = """#!{python}
import os, sys
root = os.environ["FAKE_ROOT"]
card = root + "/{card_dir}"
if sys.argv[1:] == ["-V"]:
    print("exfatprogs version : 1.2.2")
    sys.exit(16)
assert sys.argv[1] in ("-n", "-y") and len(sys.argv) == 3, sys.argv
ro = open(card + "/ro").read().strip()
if sys.argv[1] == "-y":
    with open(os.environ["FAKE_LOG"], "a") as log:
        log.write(f"fsck.exfat: -y {{sys.argv[2][len(root):]}} ro={{ro}}\\n")
    assert ro == "0", "the repair runs on the writable disk"
    if not os.environ.get("FAKE_REPAIR_KEEPS"):
        with open(sys.argv[2], "r+b") as fh:
            fh.seek(106)
            b = fh.read(1)[0]
            fh.seek(106)
            fh.write(bytes([b & ~6]))
    print("mended: the volume's dirty flag cleared")
    sys.exit(int(os.environ.get("FAKE_REPAIR", "1")))
runs = os.path.join(root, "fsck-runs")
n = int(open(runs).read()) if os.path.exists(runs) else 0
open(runs, "w").write(str(n + 1))
with open(os.environ["FAKE_LOG"], "a") as log:
    log.write(f"fsck.exfat: -n {{sys.argv[2][len(root):]}} ro={{ro}}\\n")
if os.environ.get("FAKE_FSCK_WRITES"):
    open(card + "/mmcblk1p1/stat", "w").write({stat!r})
print(f"{{sys.argv[2][len(root):]}}: clean. directories 7, files 40")
seq = [int(c) for c in os.environ.get("FAKE_FSCK_SEQ", "").split(",") if c]
if seq:
    sys.exit(seq[n] if n < len(seq) else 0)
sys.exit(int(os.environ.get("FAKE_FSCK_AFTER" if n else "FAKE_FSCK", "0")))
"""

# mount -t TYPE -o OPTS DEVICE DIR | --bind FROM DIR | -o remount,bind,OPTS DIR, as /proc/mounts shows them.
# A mount point becomes a link to what it shows; FAKE_MOUNT_FAIL names a kind (-t, bind or remount) to refuse.
FAKE_MOUNT = """#!{python}
import json, os, sys
root = os.environ["FAKE_ROOT"]
args = sys.argv[1:]
with open(os.environ["FAKE_LOG"], "a") as log:
    log.write("mount: " + " ".join(a.replace(root, "") for a in args) + "\\n")
table = root + "/proc/mounts"
lines = open(table).read().splitlines(True)
kind = "bind" if args[0] == "--bind" else "-t" if args[0] == "-t" else "remount"
if os.environ.get("FAKE_MOUNT_FAIL") == kind:
    sys.exit(32)
def show(where, target):
    assert os.path.isdir(where) and not os.path.islink(where), where
    assert not os.listdir(where), "mount point not empty: " + where
    os.rmdir(where)
    os.symlink(target, where)
def owner(path):   # the mount a path is on: its line's device and type
    best = None
    for line in lines:
        f = line.split()
        if (root + f[1] == path or path.startswith(root + f[1] + "/")) and (best is None or len(f[1]) > len(best[1])):
            best = f
    return best
if kind == "-t":
    assert args[2] == "-o" and len(args) == 6, args
    fs, opts, dev, where = args[1], args[3], args[4], args[5]
    show(where, root + "/card-fs")
    lines.append(f"{{dev[len(root):]}} {{where[len(root):]}} {{fs}} {{opts}} 0 0\\n")
elif kind == "bind":
    src, where = args[1], args[2]
    real = os.path.realpath(src)
    assert os.path.isdir(real), src
    f = owner(src)
    if os.path.realpath(where) != real:
        show(where, real)
    lines.append(f"{{f[0]}} {{where[len(root):]}} {{f[2]}} {{f[3]}} 0 0\\n")
else:
    opts, where = args[1], args[2]
    assert opts.startswith("remount,bind,") and len(args) == 3, args
    new = opts[len("remount,bind,"):]
    for i, line in enumerate(lines):
        f = line.split()
        if root + f[1] == where:
            lines[i] = f"{{f[0]}} {{f[1]}} {{f[2]}} {{new}} 0 0\\n"
open(table, "w").write("".join(lines))
"""
# umount DIR. While FAKE_BUSY (a file holding a count) is over 0, it is refused as busy. The last unmount of
# the card clears its in-use mark (byte 106, bit 1), as a clean unmount does, unless FAKE_KEEP_MARK.
FAKE_UMOUNT = """#!{python}
import os, sys
root = os.environ["FAKE_ROOT"]
with open(os.environ["FAKE_LOG"], "a") as log:
    log.write("umount: " + " ".join(a.replace(root, "") for a in sys.argv[1:]) + "\\n")
assert sys.argv[1] != "-l", "never lazily"
where = sys.argv[-1]
busy = os.environ.get("FAKE_BUSY")
if busy and os.path.exists(busy):
    left = int(open(busy).read())
    if left > 0:
        open(busy, "w").write(str(left - 1))
        print("umount: target is busy", file=sys.stderr)
        sys.exit(32)
table = root + "/proc/mounts"
lines = open(table).read().splitlines(True)
gone = [line for line in lines if root + line.split()[1] == where]
if not gone:
    print("umount: not mounted", file=sys.stderr)
    sys.exit(32)
kept = [line for line in lines if line not in gone]
open(table, "w").write("".join(kept))
if os.path.islink(where):
    os.unlink(where)
    os.mkdir(where)
dev = gone[0].split()[0]
if not any(line.split()[0] == dev for line in kept) and not os.environ.get("FAKE_KEEP_MARK"):
    path = root + dev
    if os.path.exists(path):
        with open(path, "r+b") as fh:
            fh.seek(106)
            b = fh.read(1)[0]
            fh.seek(106)
            fh.write(bytes([b & ~2]))
"""


class WriteWindowTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.busybox, cls.dash = need_busybox()

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="mpfree-sd-write-test-")
        self.root = os.path.join(self.tmp, "root")
        self.bin = os.path.join(self.tmp, "bin")
        self.log = os.path.join(self.tmp, "log")
        self.fsck = True
        self.kernel = None
        self.proc = None
        self.quiet = None
        self.letgo = None

    def make_bin(self):
        if os.path.exists(self.bin):
            return
        bb = self.busybox
        fakes = {
            "sleep": f'#!/bin/sh\necho "sleep: $*" >> "$FAKE_LOG"\nexec {bb} usleep 50000\n',
            "sync": '#!/bin/sh\necho "sync" >> "$FAKE_LOG"\n',
            "udevadm": '#!/bin/sh\necho "udevadm: $*" >> "$FAKE_LOG"\n',
            "systemctl": '#!/bin/sh\necho "systemctl: $*" >> "$FAKE_LOG"\n',
            "poweroff": '#!/bin/sh\necho "poweroff: $*" >> "$FAKE_LOG"\n',
            "dmesg": '#!/bin/sh\ncat "$FAKE_ROOT/kernel-log"\n',
            "dd": "#!/bin/sh\n" + DD_REFUSES + f'echo "dd: $*" >> "$FAKE_LOG"\nexec {bb} dd "$@"\n',
            "mount": FAKE_MOUNT.format(python=sys.executable),
            "umount": FAKE_UMOUNT.format(python=sys.executable),
        }
        extras = ["poweroff"]
        if self.fsck:
            fakes["fsck.exfat"] = FAKE_FSCK.format(python=sys.executable, card_dir=CARD_DIR,
                                                   stat=STAT.format(w=1, ws=8, d=0))
            extras.append("fsck.exfat")
        program_dir(self.bin, bb, needs(WINDOW), fakes, extras=extras)
        r = self.root
        write(os.path.join(r, "proc/mounts"), "/dev/sda1 / ext4 ro,relatime,noload 0 0\n"
                                              "/dev/sda4 /run/mpfree-data ext4 rw,nosuid,nodev,noexec,relatime 0 0\n")
        write(os.path.join(r, "proc/uptime"), "41.20 80.00\n")
        write(os.path.join(r, "proc/filesystems"), "nodev\tsysfs\nnodev\tproc\n\text4\n\tvfat\n\texfat\n")
        write(os.path.join(r, "dev/kmsg"), "")
        write(os.path.join(r, "kernel-log"), "[    2.10] mmc0: new HS200 MMC card at address 0001\n")
        write(os.path.join(r, "run/mpfree-internal-off.txt"),
              "mpfree-internal-off: done: no eMMC or SD card block device is left\n")
        os.makedirs(os.path.join(r, "card-fs/Akai"))   # the card's own files: Akai's folder is there already

    def tearDown(self):
        if self.proc and self.proc.poll() is None:
            self.proc.kill()
            self.proc.wait()
        if self.kernel:
            self.kernel.close()
        shutil.rmtree(self.tmp, ignore_errors=True)

    def script(self):
        with open(WINDOW) as fh:
            text = fh.read()
        self.assertTrue(text.startswith("R=''\n"))
        text = text.replace("R=''", f"R='{self.root}'", 1)
        for name, value in (("QUIET", self.quiet), ("LETGO", self.letgo)):
            if value is not None:
                default = {"QUIET": 30, "LETGO": 10}[name]
                self.assertEqual(text.count(f"\n{name}={default}\n"), 1)
                text = text.replace(f"\n{name}={default}\n", f"\n{name}={value}\n", 1)
        return text

    def start(self, kernel=None, request=ASK + "\n", **env):
        """The window, asked with `request` (the word; a second line `repair` asks for a repair)."""
        self.make_bin()
        self.kernel = kernel or WriteKernel(self.root)
        write(os.path.join(self.root, "run/mpfree-app/sd-card-write.request"), request)
        self.proc = subprocess.Popen([self.dash, "-c", self.script(), "setup-sd-card-write.sh"],
                                     env={"PATH": self.bin, "FAKE_LOG": self.log, "FAKE_ROOT": self.root, **env},
                                     stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)

    def finish(self, seconds=30):
        out, _ = self.proc.communicate(timeout=seconds)
        return self.proc.returncode, out

    def run_window(self, kernel=None, **env):
        self.start(kernel, **env)
        return self.finish()

    def status(self):
        with open(os.path.join(self.root, "run/mpfree-setup/sd-card-write-status")) as fh:
            st = dict(line.split("=", 1) for line in fh.read().splitlines())
        self.assertEqual(st.pop("ask", None), ASK, st)
        return st

    def wait_state(self, state, seconds=20):
        end = time.monotonic() + seconds
        path = os.path.join(self.root, "run/mpfree-setup/sd-card-write-status")
        while time.monotonic() < end:
            if os.path.exists(path) and self.status().get("state") == state:
                return self.status()
            if self.proc.poll() is not None:
                break
            time.sleep(0.02)
        out = self.proc.stdout.read() if self.proc.poll() is not None else "still running"
        self.fail(f"no state={state}: {out}\n{self.notes()}")

    def close_card(self):
        write(os.path.join(self.root, "run/mpfree-app/sd-card-write.close"), "")
        return self.finish()

    def notes(self):
        return read_text(os.path.join(self.root, "run/mpfree-setup/sd-card-write-log.txt"))

    def calls(self, prefix):
        if not os.path.exists(self.log):
            return []
        with open(self.log) as fh:
            return [line[len(prefix):] for line in fh.read().splitlines() if line.startswith(prefix)]

    def mounts(self):
        return {line.split()[1]: line.split() for line in read_text(os.path.join(self.root, "proc/mounts")).splitlines()}

    def card_byte(self, offset=106):
        with open(os.path.join(self.root, "dev/mmcblk1p1"), "rb") as fh:
            fh.seek(offset)
            return fh.read(1)[0]

    def mark(self):
        """The kernel marks the card in use at its first write."""
        path = os.path.join(self.root, "dev/mmcblk1p1")
        with open(path, "r+b") as fh:
            fh.seek(106)
            fh.write(bytes([self.card_byte() | 2]))

    def assert_closed(self):
        self.assertEqual(self.kernel.bound, [SD], "only the SD card controller is ever bound, the eMMC's never")
        self.assertEqual(self.kernel.unbound, [SD])
        self.assertEqual([d for d in os.listdir(os.path.join(self.root, "sys/block")) if d.startswith("mmcblk")], [])
        self.assertFalse(os.path.exists(os.path.join(self.root, RULE)))
        for m in (MNT, MPF, PRIV):
            self.assertNotIn(m, self.mounts())
            self.assertFalse(os.path.lexists(self.root + m), m + " is removed")
        self.assertEqual(self.calls("systemctl: ") + self.calls("poweroff: "), [])

    def assert_never_writable(self):
        self.assertEqual(self.calls("mount: "), [])
        self.assertNotIn("making mmcblk1 writable", self.notes())
        self.assertFalse(os.path.exists(os.path.join(self.root, "card-fs/MPFree")))

    # The ordinary way.

    def test_opens_an_exfat_card_for_writing_inside_mpfree_only_and_closes_it_cleanly(self):
        self.start()
        st = self.wait_state("open")
        self.assertFalse(os.path.exists(os.path.join(self.root, "run/mpfree-app/sd-card-write.request")))
        self.assertEqual(st, {"state": "open", "message": "The SD card is open: MPFree writes inside its MPFree "
                                                          "folder only.",
                              "card": "mmcblk1", "fs": "exfat", "sectors": "4096", "mount": MNT, "write": MPF,
                              "fsck": "exfatprogs version : 1.2.2"})
        self.assertEqual(self.kernel.rule, RO_RULE, "read-only as it appears, as the read-only window")
        # The check ran on the read-only disk; only then was the disk made writable.
        self.assertEqual(self.calls("fsck.exfat: "), ["-n /dev/mmcblk1p1 ro=1"])
        self.assertEqual(read_text(os.path.join(self.root, CARD_DIR, "force_ro")), "0\n")
        # The card is mounted once, privately, then shown read-only whole and writable in MPFree/ only.
        self.assertEqual(self.calls("mount: "), [
            f"-t exfat -o {RW} /dev/mmcblk1p1 {PRIV}",
            f"--bind {PRIV} {MNT}",
            f"-o remount,bind,ro,nosuid,nodev,noexec,noatime {MNT}",
            f"--bind {PRIV}/MPFree {MPF}",
            f"-o remount,bind,rw,nosuid,nodev,noexec,noatime {MPF}",
        ])
        m = self.mounts()
        self.assertEqual(m[MNT][:4], ["/dev/mmcblk1p1", MNT, "exfat", "ro,nosuid,nodev,noexec,noatime"])
        self.assertEqual(m[MPF][:4], ["/dev/mmcblk1p1", MPF, "exfat", "rw,nosuid,nodev,noexec,noatime"])
        self.assertNotIn(PRIV, m, "the private mount is gone: nothing can write outside MPFree/")
        self.assertFalse(os.path.lexists(self.root + PRIV))
        self.assertTrue(os.path.isdir(os.path.join(self.root, "card-fs/MPFree")))
        self.assertIn("made the MPFree folder on the card", self.notes())
        # Saving: the kernel marks the card and writes 5 MiB.
        self.mark()
        self.kernel.written(5 * 2048)
        time.sleep(0.3)
        self.assertIsNone(self.proc.poll())
        code, out = self.close_card()
        self.assertEqual(code, 0, out + self.notes())
        self.assertEqual(self.status(), {"state": "closed", "message": "Closed cleanly: 5 MiB written; the card "
                                                                       "checks clean.", "written": "5"})
        # Unmounted in order after a sync, read-only again, the mark found clear, checked again read-only.
        self.assertEqual(self.calls("umount: "), [PRIV, MPF, MNT])
        self.assertEqual(self.calls("fsck.exfat: "), ["-n /dev/mmcblk1p1 ro=1"] * 2)
        log = read_text(self.log).splitlines()
        self.assertLess(log.index("sync"), log.index(f"umount: {MPF}"))
        self.assert_closed()
        self.assertIn("closed cleanly after writing 5 MiB", self.notes())

    def test_an_mpfree_folder_already_there_is_used(self):
        self.make_bin()
        os.makedirs(os.path.join(self.root, "card-fs/MPFree/kits"))
        self.start()
        self.wait_state("open")
        self.assertNotIn("made the MPFree folder", self.notes())
        self.assertTrue(os.path.isdir(os.path.join(self.root, "card-fs/MPFree/kits")))
        code, out = self.close_card()
        self.assertEqual(code, 0, out + self.notes())
        self.assertEqual(self.status()["written"], "0")

    # Cards it won't write to: each closes the window, and the card was never made writable.

    def test_refuses_cards_it_does_not_write(self):
        cases = (
            ("fat32", 0, 1, dict(), "the card is not exFAT: MPFree saves only to exFAT for now"),
            ("ntfs", 0, 1, dict(), "the card is not exFAT"),
            ("exfat", 0x02, 1, dict(), "the card wasn't safely removed last time (its exFAT is marked in use): "
                                       "check it on a computer first"),
            ("exfat", 0x04, 1, dict(), "the card's exFAT says a disk error was seen"),
            ("exfat", 0x06, 1, dict(), "wasn't safely removed"),
            ("exfat", 0, 2, dict(), "the card has 2 partitions"),
            ("exfat", 0, 1, dict(FAKE_FSCK="4"), "the card's check found problems (fsck.exfat -n, see the notes)"),
        )
        for card, flags, parts, env, words in cases:
            with self.subTest(card=card, flags=flags, parts=parts, env=env):
                self.tearDown()
                self.setUp()
                code, out = self.run_window(WriteKernel(self.root, card=card, flags=flags, parts=parts), **env)
                self.assertEqual(code, 3, out + self.notes())
                st = self.status()
                self.assertEqual(st["state"], "refused")
                self.assertIn(words, st["message"])
                self.assert_never_writable()
                self.assert_closed()
                if card != "exfat" or flags or parts != 1:
                    self.assertEqual(self.calls("fsck.exfat: "), [], "refused before the check")

    def test_a_locked_card_is_refused(self):
        code, out = self.run_window(WriteKernel(self.root, locked=True))
        self.assertEqual(code, 3, out + self.notes())
        self.assertEqual(self.status(), {"state": "refused", "message": "the card stays read-only: its lock switch "
                                                                        "is on (slide it up, away from LOCK)"})
        self.assertEqual(self.calls("mount: "), [])
        self.assertFalse(os.path.exists(os.path.join(self.root, "card-fs/MPFree")))
        self.assert_closed()

    # A repair asked for (the SD card round's --repair-card): in the MPC, then everything checked again.

    def test_a_marked_card_is_repaired_when_asked_then_opened(self):
        for flags, words in ((0x02, "the card wasn't safely removed last time (its exFAT is marked in use)"),
                             (0x06, "the card wasn't safely removed last time (its exFAT is marked in use); the "
                                    "card's exFAT says a disk error was seen")):
            with self.subTest(flags=flags):
                self.tearDown()
                self.setUp()
                self.start(WriteKernel(self.root, flags=flags), request=ASK + "\nrepair\n")
                st = self.wait_state("open")
                self.assertEqual(st["repaired"], words)
                self.assertEqual(self.card_byte() & 6, 0, "the mark and the bit are clear")
                # Checked read-only, repaired on the writable disk, checked read-only again, then opened.
                self.assertEqual(self.calls("fsck.exfat: "), ["-n /dev/mmcblk1p1 ro=1", "-y /dev/mmcblk1p1 ro=0",
                                                              "-n /dev/mmcblk1p1 ro=1"])
                self.assertIn(f"repairing the card, as asked: {words}", self.notes())
                self.assertIn("fsck.exfat -y: exit code 1", self.notes())
                self.assertIn("repaired: the mark is clear and the card checks clean", self.notes())
                self.assertIn("--- fsck.exfat -y /dev/mmcblk1p1 (the repair)", self.notes())
                self.assertEqual(self.calls("mount: ")[0], f"-t exfat -o {RW} /dev/mmcblk1p1 {PRIV}")
                code, out = self.close_card()
                self.assertEqual(code, 0, out + self.notes())
                self.assertEqual(self.status(), {"state": "closed", "message": "Closed cleanly: 0 MiB written; the "
                                                                               "card checks clean.",
                                                 "written": "0", "repaired": words})
                self.assert_closed()

    def test_a_card_whose_check_finds_problems_is_repaired_when_asked(self):
        self.start(request=ASK + "\nrepair\n", FAKE_FSCK_SEQ="4,0,0")
        st = self.wait_state("open")
        self.assertEqual(st["repaired"], "its check found problems (fsck.exfat -n)")
        self.assertEqual(self.calls("fsck.exfat: "), ["-n /dev/mmcblk1p1 ro=1", "-y /dev/mmcblk1p1 ro=0",
                                                      "-n /dev/mmcblk1p1 ro=1"])
        code, out = self.close_card()
        self.assertEqual(code, 0, out + self.notes())
        self.assert_closed()

    def test_a_clean_card_is_not_repaired_even_when_asked(self):
        self.start(request=ASK + "\nrepair\n")
        st = self.wait_state("open")
        self.assertNotIn("repaired", st)
        self.assertEqual(self.calls("fsck.exfat: "), ["-n /dev/mmcblk1p1 ro=1"])
        self.assertNotIn("repairing", self.notes())
        code, out = self.close_card()
        self.assertEqual(code, 0, out + self.notes())

    def test_a_repair_that_does_not_leave_it_clean_is_refused_and_the_card_stays_read_only(self):
        for env, words in ((dict(FAKE_REPAIR_KEEPS="1", FAKE_REPAIR="4"),
                            "the repair ran (fsck.exfat -y exit code 4, see the notes), but still: the card wasn't "
                            "safely removed last time (its exFAT is marked in use); it stays read-only"),
                           (dict(FAKE_FSCK_SEQ="0,4"),
                            "the repair ran (fsck.exfat -y exit code 1), but the card's check after it found "
                            "problems (fsck.exfat -n, see the notes); it stays read-only")):
            with self.subTest(env=env):
                self.tearDown()
                self.setUp()
                code, out = self.run_window(WriteKernel(self.root, flags=0x02), request=ASK + "\nrepair\n", **env)
                self.assertEqual(code, 3, out + self.notes())
                st = self.status()
                self.assertEqual(st, {"state": "refused", "message": words,
                                      "repaired": "the card wasn't safely removed last time (its exFAT is marked "
                                                  "in use)"})
                self.assertEqual(self.calls("mount: "), [], "never mounted")
                self.assertFalse(os.path.exists(os.path.join(self.root, "card-fs/MPFree")))
                self.assertEqual(self.calls("fsck.exfat: ")[1], "-y /dev/mmcblk1p1 ro=0")
                self.assert_closed()

    def test_a_locked_card_is_not_repaired(self):
        code, out = self.run_window(WriteKernel(self.root, flags=0x02, locked=True), request=ASK + "\nrepair\n")
        self.assertEqual(code, 3, out + self.notes())
        self.assertEqual(self.status()["message"], "the card stays read-only: its lock switch is on (slide it up, "
                                                   "away from LOCK)")
        self.assertEqual(self.calls("fsck.exfat: "), ["-n /dev/mmcblk1p1 ro=1"], "no repair on a locked card")
        self.assert_closed()

    def test_only_the_word_repair_on_the_second_line_asks_for_one(self):
        for request in (ASK + "repair\n", ASK + "\nREPAIR\n", ASK + "\nrepair now\n", ASK + " repair\n"):
            with self.subTest(request=request):
                self.tearDown()
                self.setUp()
                code, out = self.run_window(WriteKernel(self.root, flags=0x02), request=request)
                self.assertEqual(code, 3, out + self.notes())
                self.assertIn("check it on a computer first, or ask for a repair in the MPC",
                              self.status()["message"])
                self.assertEqual(self.calls("fsck.exfat: "), [])

    def test_a_system_without_fsck_exfat_is_refused_before_anything(self):
        self.fsck = False
        code, out = self.run_window()
        self.assertEqual(code, 3, out + self.notes())
        self.assertEqual(self.status()["state"], "refused")
        self.assertIn("no fsck.exfat", self.status()["message"])
        self.assertEqual(self.kernel.bound, [])

    def test_an_empty_slot(self):
        code, out = self.run_window(WriteKernel(self.root, card=None))
        self.assertEqual(code, 0, out + self.notes())
        self.assertEqual(self.status(), {"state": "none", "message": "There is no card in the SD slot."})
        self.assertEqual(self.calls("mount: "), [])

    def test_checks_before_the_window_opens(self):
        def device_there():
            write(os.path.join(self.root, "sys/block/mmcblk0"), "")

        def emmc_bound():
            self.kernel.link_driver(EMMC)

        def read_window_open():
            os.makedirs(self.root + MNT)
            with open(os.path.join(self.root, "proc/mounts"), "a") as fh:
                fh.write(f"/dev/mmcblk1p1 {MNT} exfat ro 0 0\n")

        def no_switch_off():
            os.unlink(os.path.join(self.root, "run/mpfree-internal-off.txt"))

        for setup, words in ((device_there, "an eMMC or SD card block device is already there"),
                             (emmc_bound, "the eMMC controller ff0f0000.dwmmc is switched on"),
                             (read_window_open, f"something is mounted at {MNT} already"),
                             (no_switch_off, "the startup switch-off of the internal storage did not finish")):
            with self.subTest(setup=setup.__name__):
                self.tearDown()
                self.setUp()
                self.make_bin()
                self.kernel = WriteKernel(self.root)
                setup()
                code, out = self.run_window(self.kernel)
                self.assertEqual(code, 1, out + self.notes())
                self.assertEqual(self.status()["state"], "failed")
                self.assertIn(words, self.status()["message"])
                self.assertEqual(self.kernel.bound, [])
                self.assertEqual(self.calls("fsck.exfat: ") + self.calls("mount: "), [])

    def test_writes_during_the_read_only_check_fail_it(self):
        code, out = self.run_window(FAKE_FSCK_WRITES="1")
        self.assertEqual(code, 1, out + self.notes())
        self.assertIn("counters moved during the read-only check", self.status()["message"])
        self.assertEqual(self.calls("mount: "), [])
        self.assert_closed()

    def test_a_mount_that_fails_closes_the_window(self):
        for kind in ("-t", "bind", "remount"):
            with self.subTest(kind=kind):
                self.tearDown()
                self.setUp()
                code, out = self.run_window(FAKE_MOUNT_FAIL=kind)
                self.assertEqual(code, 1, out + self.notes())
                self.assertEqual(self.status()["state"], "failed")
                self.assert_closed()

    def test_a_file_called_mpfree_is_left_alone(self):
        self.make_bin()
        write(os.path.join(self.root, "card-fs/MPFree"), "not a folder")
        code, out = self.run_window()
        self.assertEqual(code, 1, out + self.notes())
        self.assertIn("a file called MPFree", self.status()["message"])
        self.assertEqual(read_text(os.path.join(self.root, "card-fs/MPFree")), "not a folder")
        self.assert_closed()

    # While open.

    def test_closes_when_the_writer_stops_answering(self):
        self.quiet = 4
        self.start()
        self.wait_state("open")
        alive = os.path.join(self.root, "run/mpfree-app/sd-card-write.alive")
        for n in range(1, 25):
            write(alive, f"{n}\n")
            time.sleep(0.05)
        self.assertIsNone(self.proc.poll(), "open while the counter moves" + self.notes())
        code, out = self.finish()
        self.assertEqual(code, 0, out + self.notes())
        self.assertEqual(self.status()["message"], "Closed: the writer stopped. 0 MiB written; the card checks clean.")
        self.assert_closed()

    def test_an_error_on_the_card_fails_and_closes_it(self):
        self.start()
        self.wait_state("open")
        # errors=remount-ro: the kernel found an error and made the filesystem read-only.
        table = os.path.join(self.root, "proc/mounts")
        write(table, read_text(table).replace(f"{MPF} exfat rw,", f"{MPF} exfat ro,"))
        code, out = self.finish()
        self.assertEqual(code, 1, out + self.notes())
        self.assertIn("the card's filesystem stopped writing", self.status()["message"])
        self.assert_closed()

    def test_an_emmc_appearing_fails_and_closes_it(self):
        self.start()
        self.wait_state("open")
        self.kernel.disk(EMMC_DIR, "mmcblk0", {"ro": "1\n", "force_ro": "1\n", "size": "2048\n",
                                               "stat": STAT.format(w=0, ws=0, d=0)})
        code, out = self.finish()
        # It fails and closes; the eMMC disk doesn't go with the SD controller, so the MPC turns off.
        self.assertEqual(code, 2, out + self.notes())
        self.assertIn("STOP: a block device that is not the SD card's appeared: mmcblk0", self.notes())
        self.assertEqual(self.calls("systemctl: "), ["poweroff"])
        self.assertNotIn(MPF, self.mounts(), "unmounted first")

    def test_the_card_taken_out_while_open(self):
        self.start()
        self.wait_state("open")
        self.kernel.take_out()
        code, out = self.finish()
        self.assertEqual(code, 0, out + self.notes())
        self.assertEqual(self.status()["message"], "The card was taken out while open for writing: check it on a "
                                                   "computer before the next save.")
        self.assertNotIn(MPF, self.mounts())
        self.assertNotIn(MNT, self.mounts())

    def test_a_stop_from_the_system_closes_it(self):
        self.start()
        self.wait_state("open")
        self.proc.send_signal(signal.SIGTERM)
        code, out = self.finish()
        self.assertEqual(code, 0, out + self.notes())
        self.assertEqual(self.status(), {"state": "closed", "message": "Closed: the system stopped it."})
        self.assert_closed()

    # The close.

    def test_still_marked_or_unclean_after_the_close_is_said(self):
        for env, words in ((dict(FAKE_KEEP_MARK="1"), "it is still marked in use after the unmount"),
                           (dict(FAKE_FSCK_AFTER="4"), "its check after writing found problems")):
            with self.subTest(env=env):
                self.tearDown()
                self.setUp()
                self.start(**env)
                self.wait_state("open")
                self.mark()
                self.kernel.written(2048)
                code, out = self.close_card()
                self.assertEqual(code, 1, out + self.notes())
                st = self.status()
                self.assertEqual(st["state"], "failed")
                self.assertTrue(st["message"].startswith(f"Closed after writing 1 MiB, but {words}"), st)
                self.assertTrue(st["message"].endswith(": check it on a computer."), st)
                self.assert_closed()

    def test_a_file_still_held_is_let_go_or_its_holder_stopped(self):
        busy = os.path.join(self.tmp, "busy")
        self.letgo = 2
        # Held a little: the plain unmount is tried again until it works.
        self.start(FAKE_BUSY=busy)
        self.wait_state("open")
        write(busy, "2")
        code, out = self.close_card()
        self.assertEqual(code, 0, out + self.notes())
        self.assertEqual(self.calls("umount: "), [PRIV] + [MPF] * 3 + [MNT])
        self.assertNotIn("stopping process", self.notes())
        self.assert_closed()
        # Held past LETGO: the process with a file inside the card is stopped, then it unmounts.
        self.tearDown()
        self.setUp()
        self.letgo = 2
        self.make_bin()
        os.makedirs(self.root + PROC + "/fd")
        write(self.root + PROC + "/comm", "mpfree-disktest\n")
        os.symlink(MPF + "/mpfree-disktest/seq.bin", self.root + PROC + "/fd/3")
        os.makedirs(self.root + "/proc/77/fd")
        os.symlink("/run/mpfree-data/x", self.root + "/proc/77/fd/3")
        self.start(FAKE_BUSY=busy)
        self.wait_state("open")
        write(busy, "4")
        code, out = self.close_card()
        self.assertEqual(code, 0, out + self.notes())
        self.assertIn("stopping process 4242 (mpfree-disktest): it still held a file on the card", self.notes())
        self.assertNotIn("process 77", self.notes())
        self.assert_closed()

    def test_shuts_down_cleanly_if_the_card_never_unmounts(self):
        busy = os.path.join(self.tmp, "busy")
        self.letgo = 1
        self.start(FAKE_BUSY=busy)
        self.wait_state("open")
        write(busy, "1000")
        code, out = self.close_card()
        self.assertEqual(code, 2, out + self.notes())
        self.assertEqual(self.calls("systemctl: "), ["poweroff"])
        self.assertEqual(self.status()["message"], "The SD card could not be closed, so the MPC shuts down to close "
                                                   "it cleanly.")
        self.assertIn(SD, self.kernel.bound)
        self.assertEqual(self.kernel.unbound, [], "never switched off under a mounted card")

    def test_powers_off_if_the_controller_does_not_let_go(self):
        self.start(WriteKernel(self.root, ignore_unbind=True))
        self.wait_state("open")
        code, out = self.close_card()
        self.assertEqual(code, 2, out)
        self.assertEqual(self.calls("systemctl: "), ["poweroff"])
        self.assertNotIn(MPF, self.mounts(), "unmounted before it turns the MPC off")


class StaticTests(unittest.TestCase):
    def code(self):
        with open(WINDOW) as fh:
            text = fh.read()
        return "\n".join(line for line in text.splitlines()
                         if not line.lstrip().startswith("#") and not line.startswith("NEEDS="))

    def test_parses(self):
        dash = shutil.which("dash")
        if not dash:
            self.skipTest("dash not installed")
        res = subprocess.run([dash, "-n", WINDOW], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        self.assertEqual(res.returncode, 0, res.stdout)

    def test_what_it_can_change(self):
        code = self.code()
        # The card's device is only read: one sector, before, after a repair and after the close; nothing
        # written to a device (the repair's writes are fsck.exfat's).
        self.assertEqual([m.strip("(\n ") for m in re.findall(r"(?:^|[\s;|&(])dd\s[^\n|]*", code)],
                         ['dd if="$SECTOR" bs=1 skip="$1" count=1 2>/dev/null'] +
                         ['dd if="$R/dev/$dev" bs=512 count=1 2>/dev/null > "$SECTOR"'] * 3)
        self.assertEqual(re.findall(r"\bof=\S+", code), [])
        self.assertEqual(re.findall(r'>\s*"?\$R/dev/[^\s;]+', code), ['> "$R/dev/kmsg"'])
        # fsck.exfat with -n (and -V for its version), and -y once: the repair, only when asked for and only
        # after the read-only check; no other tool that changes a filesystem.
        self.assertEqual(sorted(re.findall(r"(?m)(?:^\s*|\$\()fsck\.exfat (-\w)", code)), ["-V", "-n", "-y"])
        self.assertLess(code.index('fsck_card "before writing"'), code.index("fsck.exfat -y"))
        self.assertLess(code.index("fsck.exfat -y"), code.index('fsck_card "after the repair"'))
        self.assertEqual(re.findall(r'(?m)^repair=\S+|\[ "\$how" = repair \] && repair=\S+', code),
                         ["repair=''", '[ "$how" = repair ] && repair=yes'])
        # What needs a repair is refused unless one was asked for.
        self.assertIn('need=$(marks)\nif [ -n "$need" ]; then\n  [ -n "$repair" ] || refuse "$need: ', code)
        self.assertIn('elif [ -n "$repair" ]; then\n  need="${need:+$need; }its check found problems', code)
        for word in ("mkfs", "mke2fs", "mkfs.vfat", "mkfs.exfat", "fsck.vfat", "blkdiscard", "hdparm", "sfdisk",
                     "sgdisk", "parted", "wipefs", "tune2fs", "e2fsck", "debugfs", "truncate", "fallocate",
                     "fatlabel", "exfatlabel", "dosfslabel", "blockdev", "chown", "chmod", "rm -r", "umount -l"):
            self.assertNotRegex(code, rf"(^|[\s;|&(]){re.escape(word)}(\s|$)", word)
        # One read-write mount, at the private place, then the two binds; the whole card read-only.
        mounts = [m.strip() for m in re.findall(r"(?:^|[\s;|&(])mount\s[^\n]*", code)]
        self.assertEqual(mounts, [
            'mount -t exfat -o "rw,nosuid,nodev,noexec,noatime,dirsync,uid=0,gid=$APP_GID,fmask=0117,dmask=0007,'
            'iocharset=utf8,errors=remount-ro" \\',
            'mount --bind "$R$PRIV" "$R$MNT" 2>> "$S/sd-card-write-log.txt" &&',
            'mount -o remount,bind,ro,nosuid,nodev,noexec,noatime "$R$MNT" 2>> "$S/sd-card-write-log.txt" ||',
            'mount --bind "$R$PRIV/MPFree" "$R$MPF" 2>> "$S/sd-card-write-log.txt" &&',
            'mount -o remount,bind,rw,nosuid,nodev,noexec,noatime "$R$MPF" 2>> "$S/sd-card-write-log.txt" ||',
        ])
        self.assertIn('"$R/dev/$dev" "$R$PRIV"', code)
        self.assertIn("MPF=/run/mpfree-sd/MPFree\n", code)
        # The only directory it makes on the card is MPFree.
        self.assertEqual(re.findall(r'mkdir "\$R\$PRIV/[^"]*"', code), ['mkdir "$R$PRIV/MPFree"'])
        # The kernel: force_ro 0 only on the card's own disk, once, after the check; the SD controller only.
        self.assertEqual(re.findall(r'echo (\S+) > "\$R/sys/block/\$\w+/force_ro"', code), ["1", "0"])
        self.assertLess(code.index('fsck_card "before writing"'), code.index('echo 0 > "$R/sys/block/$card/force_ro"'))
        self.assertEqual(re.findall(r"ATTR\{[^}]*\}=\S*", code), ['ATTR{force_ro}="1"\''])
        self.assertIn("SD=ff0c0000\nEMMC=ff0f0000\n", code)
        self.assertEqual(re.findall(r"(?m)^\s*devdir=\S+", code), ["devdir=''", "  devdir=$d"])
        self.assertEqual(re.findall(r'>\s*"?\$R/proc/', code), [])

    def test_fsck_is_not_a_program_the_creator_needs(self):
        # A system without fsck.exfat still makes a stick; the window refuses when asked instead.
        self.assertNotIn("fsck.exfat", needs(WINDOW))
        self.assertNotIn("kill", needs(WINDOW), "kill is the shell's own")


if __name__ == "__main__":
    unittest.main(verbosity=2)
