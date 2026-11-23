#!/usr/bin/env python3
"""Tests for setup-sd-card.sh, the SD card's read-only window on the stick's system (step S1 of the SD card
plan, docs/dev/hardware/sd-card-and-internal-drive.md). No device.

It runs under dash with BusyBox applets against a fake kernel, as setup-backup.sh's tests do
(test_stick_setup.py): a fake sysfs whose drivers_probe and unbind files are FIFOs served by a thread, as the
SD card controller and udev would answer them, the card's ro files following its force_ro file as the MMC
driver's do, /sys/block entries that are links into the controller's folder as the kernel's are, and /dev
files holding the card's first sectors. mount, umount and dmesg are stand-ins: the mount is only written to
the fake /proc/mounts. It is rehearsed through every way it can stop, with nothing on PATH but the programs
its NEEDS line names. Set MPFREE_REQUIRE_BUSYBOX=1 (CI does) to fail instead of skip without BusyBox or dash.
"""
import os
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from test_stick_setup import (DD_REFUSES, EMMC, SD, STAT, STICK, blob, need_busybox, needs,  # noqa: E402
                              program_dir, read_text, write)

WINDOW = os.path.join(STICK, "setup-sd-card.sh")
MNT = "/run/mpfree-sd"
RULE = "run/udev/rules.d/00-mpfree-sd-ro.rules"
RO_RULE = 'SUBSYSTEM=="block", KERNEL=="mmcblk*", ENV{DEVTYPE}=="disk", ACTION=="add", ATTR{force_ro}="1"\n'
OPTIONS = "ro,nosuid,nodev,noexec,noatime,uid=0,gid=960,fmask=0137,dmask=0027"
CARD_SECTORS = 4096
PART_START = 64
# Where the kernel puts the card: under the SD controller's folder, /sys/block/mmcblk1 a link to it.
CARD_DIR = f"sys/devices/platform/{SD}/mmc_host/mmc1/mmc1:0001/block/mmcblk1"
EMMC_DIR = f"sys/devices/platform/{EMMC}/mmc_host/mmc0/mmc0:0001/block/mmcblk0"
REFUSED = "[  812.40] Trying to write to read-only block-device mmcblk1p1\n"
# The pad player's word for an opening, as it writes it in the request; every status gives it back.
ASK = "5f0c1d2e3a4b59687766554433221100"


def boot_sector(kind, flags=0):
    """The first sector of a filesystem, as FAT and exFAT write it (the names at 3, 54 and 82), the rest noise,
    but for the byte that holds the in-use mark (FAT32 65, FAT16 37, exFAT's VolumeFlags 106-107): `flags`."""
    s = bytearray(blob(kind, 512))
    if kind == "fat32":
        s[3:11], s[82:90] = b"MSDOS5.0", b"FAT32   "
        s[65] = flags
    elif kind == "fat16":
        s[3:11], s[54:62] = b"MSDOS5.0", b"FAT16   "
        s[37] = flags
    elif kind == "exfat":
        s[3:11] = b"EXFAT   "
        s[106:108] = flags.to_bytes(2, "little")
    elif kind == "ntfs":
        s[3:11] = b"NTFS    "
    elif kind == "ext4":   # ext4 keeps nothing in its first sector; its superblock is at 1024
        s = bytearray(512)
    s[510:512] = b"\x55\xaa" if kind != "ext4" else b"\0\0"
    return bytes(s)


class FakeSdKernel:
    """The SD card controller as the kernel and udev answer setup-sd-card.sh. Writing the controller's name to
    /sys/bus/platform/drivers_probe binds it, and the card appears: /sys/block/mmcblk1 (a link into the
    controller's folder) with its partitions, and /dev nodes holding its first sectors. Its ro files follow its
    force_ro file (links to it), which starts at 1 only if the window's udev rule was there first. Writing the
    name to the driver's unbind file takes them away again. Options: `card` (fat32, fat16, exfat, ntfs, ext4
    or None for an empty slot), `parts` (0 for a card without a table), `emmc_too` (an eMMC disk appears as
    well, under its own controller), `boot_area` (an eMMC boot area appears under the SD controller),
    `writable` (a card that stays writable), `ignore_rule`, `ignore_unbind`."""

    def __init__(self, root, card="fat32", parts=1, emmc_too=False, boot_area=False, writable=False,
                 ignore_rule=False, ignore_unbind=False, flags=0):
        self.root, self.card, self.parts, self.flags = root, card, parts, flags
        self.emmc_too, self.boot_area, self.writable = emmc_too, boot_area, writable
        self.ignore_rule, self.ignore_unbind = ignore_rule, ignore_unbind
        self.bound, self.unbound, self.rule = [], [], None
        self.drv = os.path.join(root, "sys/bus/platform/drivers/dwmmc_rockchip")
        os.makedirs(self.drv)
        for name in (EMMC, SD, "ff0d0000.dwmmc"):
            os.makedirs(os.path.join(root, "sys/bus/platform/devices", name))
        self.link_driver("ff0d0000.dwmmc")   # the SDIO controller (Wi-Fi) stays bound
        os.makedirs(os.path.join(root, "sys/block"))
        os.makedirs(os.path.join(root, "dev/disk/by-partlabel"))
        os.symlink("../../sda1", os.path.join(root, "dev/disk/by-partlabel/mpfree-test-root"))
        self.probe = os.path.join(root, "sys/bus/platform/drivers_probe")
        os.mkfifo(self.probe)
        os.mkfifo(os.path.join(self.drv, "unbind"))
        self.stop = False
        self.threads = [threading.Thread(target=self.serve, args=(self.probe, self.bind), daemon=True),
                        threading.Thread(target=self.serve, args=(os.path.join(self.drv, "unbind"), self.unbind),
                                         daemon=True)]
        for t in self.threads:
            t.start()

    def link_driver(self, name):
        dev = os.path.join(self.root, "sys/bus/platform/devices", name)
        os.symlink(os.path.relpath(self.drv, dev), os.path.join(dev, "driver"))

    def serve(self, fifo, action):
        with os.fdopen(os.open(fifo, os.O_RDWR), "r") as fh:
            for line in fh:
                if self.stop:
                    return
                for name in line.split():
                    action(name)

    def disk(self, folder, name, files, parts=()):
        """A disk's folder, made whole aside and renamed into place, then its /sys/block link."""
        final = os.path.join(self.root, folder)
        stage = os.path.join(os.path.dirname(final), "." + name)
        for rel, value in files.items():
            path = os.path.join(stage, rel)
            if isinstance(value, tuple):
                os.makedirs(os.path.dirname(path), exist_ok=True)
                os.symlink(value[1], path)
            else:
                write(path, value)
        for pname, pfiles in parts:
            for rel, value in pfiles.items():
                path = os.path.join(stage, pname, rel)
                if isinstance(value, tuple):
                    os.makedirs(os.path.dirname(path), exist_ok=True)
                    os.symlink(value[1], path)
                else:
                    write(path, value)
        os.rename(stage, final)
        link = os.path.join(self.root, "sys/block", name)
        os.symlink(os.path.relpath(final, os.path.dirname(link)), link)

    def bind(self, name):
        self.bound.append(name)
        if name != SD:
            return
        self.rule = read_text(os.path.join(self.root, RULE)) or None
        self.link_driver(SD)
        if self.card:
            self.insert()

    def insert(self):
        force_ro = "1\n" if self.rule and not self.ignore_rule else "0\n"
        disk_ro = "0\n" if self.writable else ("link", "force_ro")
        part_ro = "0\n" if self.writable else ("link", "../force_ro")
        stat = STAT.format(w=0, ws=0, d=0)
        fs = boot_sector(self.card, self.flags)
        parts = []
        if self.parts == 0:
            write(os.path.join(self.root, "dev/mmcblk1"), fs + bytes(1024))
        else:
            write(os.path.join(self.root, "dev/mmcblk1"), blob("mbr", 512) + bytes(1024))
            size = (CARD_SECTORS - PART_START) // self.parts
            for n in range(1, self.parts + 1):
                write(os.path.join(self.root, f"dev/mmcblk1p{n}"), fs + bytes(1024))
                parts.append((f"mmcblk1p{n}", {"start": f"{PART_START + (n - 1) * size}\n", "size": f"{size}\n",
                                               "ro": part_ro, "stat": stat}))
        if self.boot_area:
            self.disk(f"{CARD_DIR}boot0", "mmcblk1boot0", {"ro": "1\n", "force_ro": "1\n", "size": "16\n",
                                                            "stat": stat})
        if self.emmc_too:
            self.disk(EMMC_DIR, "mmcblk0", {"ro": ("link", "force_ro"), "force_ro": "1\n", "size": "2048\n",
                                            "stat": stat})
        self.disk(CARD_DIR, "mmcblk1", {"ro": disk_ro, "force_ro": force_ro, "size": f"{CARD_SECTORS}\n",
                                        "stat": stat, "device/name": "SD32G\n"}, parts)

    def take_out(self):
        """The card pulled from the slot while the controller stays bound."""
        for name in os.listdir(os.path.join(self.root, "sys/block")):
            link = os.path.join(self.root, "sys/block", name)
            if SD in os.readlink(link):
                os.unlink(link)
        shutil.rmtree(os.path.join(self.root, f"sys/devices/platform/{SD}"), ignore_errors=True)
        for dev in os.listdir(os.path.join(self.root, "dev")):
            if dev.startswith("mmcblk1"):
                os.unlink(os.path.join(self.root, "dev", dev))

    def unbind(self, name):
        self.unbound.append(name)
        if self.ignore_unbind:
            return
        os.unlink(os.path.join(self.root, "sys/bus/platform/devices", name, "driver"))
        if name == SD:
            self.take_out()

    def bump_writes(self):
        write(os.path.join(self.root, CARD_DIR, "mmcblk1p1/stat"), STAT.format(w=1, ws=8, d=0))

    def close(self):
        self.stop = True
        for fifo in (self.probe, os.path.join(self.drv, "unbind")):
            try:
                fd = os.open(fifo, os.O_WRONLY | os.O_NONBLOCK)
            except OSError:
                continue
            os.write(fd, b"\n")
            os.close(fd)
        for t in self.threads:
            t.join(5)


# mount -t TYPE -o OPTIONS DEVICE DIR, as far as /proc/mounts shows it; FAKE_MOUNT_FAIL makes it fail.
FAKE_MOUNT = """#!{python}
import os, sys
root = os.environ["FAKE_ROOT"]
with open(os.environ["FAKE_LOG"], "a") as log:
    log.write("mount: " + " ".join(a.replace(root, "") for a in sys.argv[1:]) + "\\n")
if os.environ.get("FAKE_MOUNT_FAIL"):
    sys.exit(32)
args = sys.argv[1:]
assert args[0] == "-t" and args[2] == "-o" and len(args) == 6, args
fs, opts, dev, where = args[1], args[3], args[4], args[5]
assert os.path.isdir(where), where
with open(root + "/proc/mounts", "a") as fh:
    fh.write(f"{{dev[len(root):]}} {{where[len(root):]}} {{fs}} {{opts}},errors=remount-ro 0 0\\n")
"""
# umount [-l] DIR; while FAKE_BUSY (a file holding a count) is over 0, a plain umount is refused as busy.
FAKE_UMOUNT = """#!{python}
import os, sys
root = os.environ["FAKE_ROOT"]
with open(os.environ["FAKE_LOG"], "a") as log:
    log.write("umount: " + " ".join(a.replace(root, "") for a in sys.argv[1:]) + "\\n")
lazy = sys.argv[1] == "-l"
where = sys.argv[-1][len(root):]
busy = os.environ.get("FAKE_BUSY")
if busy and not lazy:
    left = int(open(busy).read())
    if left > 0:
        open(busy, "w").write(str(left - 1))
        print("umount: target is busy", file=sys.stderr)
        sys.exit(32)
lines = open(root + "/proc/mounts").read().splitlines(True)
kept = [line for line in lines if line.split()[1] != where]
if len(kept) == len(lines):
    print("umount: not mounted", file=sys.stderr)
    sys.exit(32)
open(root + "/proc/mounts", "w").write("".join(kept))
"""


class WindowTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.busybox, cls.dash = need_busybox()

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="mpfree-sd-card-test-")
        self.root = os.path.join(self.tmp, "root")
        self.bin = os.path.join(self.tmp, "bin")
        self.log = os.path.join(self.tmp, "log")
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
        program_dir(self.bin, bb, needs(WINDOW), fakes, extras=("poweroff",))
        r = self.root
        write(os.path.join(r, "proc/mounts"), "/dev/sda1 / ext4 ro,relatime,noload 0 0\n"
                                              "/dev/sda4 /run/mpfree-data ext4 rw,nosuid,nodev,noexec,relatime 0 0\n")
        write(os.path.join(r, "proc/uptime"), "41.20 80.00\n")
        write(os.path.join(r, "proc/filesystems"), "nodev\tsysfs\nnodev\tproc\n\text4\n\tvfat\n\texfat\n")
        write(os.path.join(r, "dev/kmsg"), "")
        write(os.path.join(r, "kernel-log"), "[    2.10] mmc0: new HS200 MMC card at address 0001\n")
        write(os.path.join(r, "run/mpfree-internal-off.txt"),
              "mpfree-internal-off: switched off ff0f0000.dwmmc\nmpfree-internal-off: switched off ff0c0000.dwmmc\n"
              "mpfree-internal-off: done: no eMMC or SD card block device is left\n")
        self.kernel = None
        self.proc = None
        self.quiet = None   # QUIET for this run, when a test needs it shorter

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
        if self.quiet is not None:
            self.assertEqual(text.count("\nQUIET=120\n"), 1)
            text = text.replace("\nQUIET=120\n", f"\nQUIET={self.quiet}\n", 1)
        return text

    def start(self, request=ASK + "\n", **env):
        path = os.path.join(self.root, "run/mpfree-app/sd-card.request")
        write(path, request)
        self.proc = subprocess.Popen([self.dash, "-c", self.script(), "setup-sd-card.sh"],
                                     env={"PATH": self.bin, "FAKE_LOG": self.log, "FAKE_ROOT": self.root, **env},
                                     stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        return self.proc

    def finish(self, seconds=30):
        out, _ = self.proc.communicate(timeout=seconds)
        return self.proc.returncode, out

    def run_window(self, **env):
        """A run that ends by itself (a check, an empty slot or a surprise)."""
        self.start(**env)
        return self.finish()

    def wait_state(self, state, seconds=20):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            if os.path.exists(self.status_path()) and self.status().get("state") == state:
                return self.status()
            if self.proc.poll() is not None:
                break
            time.sleep(0.02)
        out = self.proc.stdout.read() if self.proc.poll() is not None else "still running"
        self.fail(f"no state={state}: {out}\n{self.notes()}")

    def wait_state_ask(self, state, ask, seconds=20):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            if os.path.exists(self.status_path()) and self.status(ask=ask).get("state") == state:
                return self.status(ask=ask)
            time.sleep(0.02)
        self.fail(f"no state={state}\n{self.notes()}")

    def close_card(self):
        write(os.path.join(self.root, "run/mpfree-app/sd-card.close"), "")
        return self.finish()

    def status_path(self):
        return os.path.join(self.root, "run/mpfree-setup/sd-card-status")

    def status(self, ask=ASK):
        """The status, without its ask line, which must give back the request's word."""
        with open(self.status_path()) as fh:
            st = dict(line.split("=", 1) for line in fh.read().splitlines())
        self.assertEqual(st.pop("ask", None), ask, st)
        return st

    def notes(self):
        return read_text(os.path.join(self.root, "run/mpfree-setup/sd-card-log.txt"))

    def calls(self, prefix):
        if not os.path.exists(self.log):
            return []
        with open(self.log) as fh:
            return [line[len(prefix):] for line in fh.read().splitlines() if line.startswith(prefix)]

    def mounts(self):
        return [line.split()[1] for line in read_text(os.path.join(self.root, "proc/mounts")).splitlines()]

    def assert_never_opened(self):
        self.assertEqual(self.kernel.bound, [])
        self.assertEqual(self.calls("mount: "), [])

    def assert_closed(self, mounted=True):
        """The window is shut: unmounted, the controller unbound, no card device, no rule, nothing else bound."""
        self.assertEqual(self.kernel.bound, [SD], "only the SD card controller is ever bound, the eMMC's never")
        self.assertEqual(self.kernel.unbound, [SD])
        self.assertFalse(os.path.lexists(os.path.join(self.root, "sys/bus/platform/devices", SD, "driver")))
        self.assertTrue(os.path.lexists(os.path.join(self.root, "sys/bus/platform/devices/ff0d0000.dwmmc/driver")))
        self.assertEqual([d for d in os.listdir(os.path.join(self.root, "sys/block")) if d.startswith("mmcblk")], [])
        self.assertFalse(os.path.exists(os.path.join(self.root, RULE)))
        self.assertNotIn(MNT, self.mounts())
        self.assertFalse(os.path.exists(self.root + MNT), "the empty mount point is removed")
        self.assertEqual(sorted(os.listdir(os.path.join(self.root, "dev/disk/by-partlabel"))), ["mpfree-test-root"])
        self.assertEqual(self.calls("systemctl: ") + self.calls("poweroff: "), [])
        if mounted:
            self.assertTrue(self.calls("umount: ")[-1].endswith(MNT), self.calls("umount: "))

    def assert_read_only(self, dev="/dev/mmcblk1p1"):
        """The card was read only as one sector of its filesystem, and mounted read-only; the in-use mark was
        read from the sector's copy in RAM."""
        reads = self.calls("dd: ")
        self.assertEqual([r for r in reads if "/dev/" in r], [f"if={self.root}{dev} bs=512 count=1"])
        sector = f"if={self.root}/run/mpfree-setup/sd-card-sector bs=1 skip="
        self.assertEqual(len([r for r in reads if r.startswith(sector) and r.endswith(" count=1")]), 1, reads)
        self.assertEqual(len(reads), 2, reads)
        for call in self.calls("mount: "):
            self.assertRegex(call, rf"^-t (vfat|exfat) -o {re.escape(OPTIONS)} {re.escape(dev)} {re.escape(MNT)}$")

    # The ordinary ways.

    def test_opens_a_fat32_card_read_only_and_closes_it(self):
        self.kernel = FakeSdKernel(self.root)
        self.start()
        st = self.wait_state("open")
        self.assertFalse(os.path.exists(os.path.join(self.root, "run/mpfree-app/sd-card.request")),
                         "the request is taken away, so the path unit stays quiet")
        self.assertEqual(st, {"state": "open", "message": "The SD card is open, read-only.", "card": "mmcblk1",
                              "fs": "vfat", "sectors": str(CARD_SECTORS), "mount": MNT, "dirty": "no"})
        self.assertEqual(self.kernel.rule, RO_RULE, "the rule is there before the bind")
        self.assertEqual(read_text(os.path.join(self.root, CARD_DIR, "force_ro")), "1\n")
        self.assertIn(f"/dev/mmcblk1p1 {MNT} vfat {OPTIONS},", read_text(os.path.join(self.root, "proc/mounts")))
        self.assertIn("window open: mmcblk1, 4096 sectors, FAT32 on mmcblk1p1, read-only at /run/mpfree-sd",
                      self.notes())
        # It stays open, checking once a second, until it is asked to close.
        time.sleep(0.5)
        self.assertIsNone(self.proc.poll())
        code, out = self.close_card()
        self.assertEqual(code, 0, out + self.notes())
        self.assertEqual(self.status(), {"state": "closed", "message": "Closed. Nothing was written to the card."})
        self.assert_closed()
        self.assert_read_only()
        self.assertFalse(os.path.exists(os.path.join(self.root, "run/mpfree-app/sd-card.close")))
        self.assertIn("window closed: no SD card block device is left", self.notes())

    def test_exfat_and_a_card_without_a_table(self):
        for card, parts, fs, kind, dev in (("exfat", 1, "exfat", "exFAT", "mmcblk1p1"),
                                           ("fat16", 0, "vfat", "FAT16", "mmcblk1")):
            with self.subTest(card=card, parts=parts):
                self.tearDown()
                self.setUp()
                self.kernel = FakeSdKernel(self.root, card=card, parts=parts)
                self.start()
                self.assertEqual(self.wait_state("open")["fs"], fs)
                self.assertIn(f"{kind} on {dev}, read-only", self.notes())
                code, out = self.close_card()
                self.assertEqual(code, 0, out + self.notes())
                self.assert_closed()
                self.assert_read_only(f"/dev/{dev}")

    def test_a_card_not_safely_removed_last_time_is_still_opened_and_said(self):
        # (card, flags, dirty): only the in-use bit counts, and exFAT's disk-error bit.
        for card, flags, dirty in (("fat32", 0x01, "yes"), ("fat32", 0x02, "no"), ("fat16", 0x01, "yes"),
                                   ("fat16", 0x00, "no"), ("exfat", 0x0002, "yes"), ("exfat", 0x0001, "no"),
                                   ("exfat", 0x0004, "yes"), ("exfat", 0x0100, "no")):
            with self.subTest(card=card, flags=flags):
                self.tearDown()
                self.setUp()
                self.kernel = FakeSdKernel(self.root, card=card, flags=flags)
                self.start()
                st = self.wait_state("open")
                self.assertEqual(st["dirty"], dirty)
                if dirty == "yes":
                    self.assertEqual(st["message"], "The SD card is open, read-only. It wasn't safely removed "
                                                    "last time: check it on a computer.")
                    self.assertIn("the card was not unmounted cleanly last time", self.notes())
                else:
                    self.assertEqual(st["message"], "The SD card is open, read-only.")
                    self.assertNotIn("not unmounted cleanly", self.notes())
                self.assertEqual("disk error" in self.notes(), flags == 0x0004)
                code, out = self.close_card()
                self.assertEqual(code, 0, out + self.notes())
                self.assert_closed()
                self.assert_read_only()
                self.assertFalse(os.path.exists(os.path.join(self.root, "run/mpfree-setup/sd-card-sector")))

    def test_an_empty_slot(self):
        self.kernel = FakeSdKernel(self.root, card=None)
        code, out = self.run_window()
        self.assertEqual(code, 0, out + self.notes())
        self.assertEqual(self.status(), {"state": "none", "message": "There is no card in the SD slot."})
        self.assert_closed(mounted=False)
        self.assertEqual(self.calls("mount: ") + self.calls("dd: "), [])
        self.assertIn("no card answered within 10 s", self.notes())

    def test_cards_it_does_not_read_are_left_alone(self):
        cases = (
            ({"card": "ntfs"}, None, "the card is NTFS; MPFree reads FAT and exFAT"),
            ({"card": "ext4"}, None, "the card's filesystem is not FAT or exFAT; MPFree reads those"),
            ({"card": "fat32", "parts": 2}, None,
             "the card has 2 partitions; MPFree reads a card with one, as a camera, a computer or Akai's system formats it"),
            ({"card": "exfat"}, "nodev\tproc\n\tvfat\n",
             "the card is exFAT, and this kernel has no exfat filesystem loaded (the stick must load it before the switch)"),
        )
        for opts, filesystems, why in cases:
            with self.subTest(why=why):
                self.tearDown()
                self.setUp()
                if filesystems:
                    write(os.path.join(self.root, "proc/filesystems"), filesystems)
                self.kernel = FakeSdKernel(self.root, **opts)
                code, out = self.run_window()
                self.assertEqual(code, 1, out)
                self.assertEqual(self.status(), {"state": "failed", "message": why})
                self.assert_closed(mounted=False)
                self.assertEqual(self.calls("mount: "), [])
                self.assertEqual(len(self.calls("dd: ")), 0 if opts.get("parts") == 2 else 1)

    # Before the window opens.

    def test_checks_before_the_window_opens(self):
        def no_switch_off():
            write(os.path.join(self.root, "run/mpfree-internal-off.txt"), "mpfree-internal-off: started\n")

        def a_device_there():
            write(os.path.join(self.root, "sys/block/mmcblk0/size"), "2048\n")

        def sd_bound():
            self.kernel.link_driver(SD)

        def emmc_bound():
            self.kernel.link_driver(EMMC)

        def backup_running():
            write(os.path.join(self.root, "run/mpfree-setup/status"), "state=running\nstep=copying\n")

        def content_open():
            write(os.path.join(self.root, "run/mpfree-setup/status"), "state=open\nstep=reading\n")

        def mounted_there():
            with open(os.path.join(self.root, "proc/mounts"), "a") as fh:
                fh.write(f"/dev/sdb1 {MNT} vfat ro 0 0\n")

        def no_controller():
            shutil.rmtree(os.path.join(self.root, "sys/bus/platform/devices", SD))

        cases = (
            (no_switch_off, "the startup switch-off of the internal storage did not finish"),
            (a_device_there, "an eMMC or SD card block device is already there (mmcblk0 )"),
            (sd_bound, f"the SD card controller {SD} is switched on already"),
            (emmc_bound, f"the eMMC controller {EMMC} is switched on"),
            (backup_running, "the first-start backup is running"),
            (content_open, "the MPC's internal drive is open for Akai's kits and samples"),
            (mounted_there, f"something is mounted at {MNT} already"),
            (no_controller, "no SD card controller at ff0c0000"),
        )
        for setup, why in cases:
            with self.subTest(why=why):
                self.tearDown()
                self.setUp()
                self.kernel = FakeSdKernel(self.root)
                setup()
                code, out = self.run_window()
                self.assertEqual(code, 1, out)
                self.assertEqual(self.status(), {"state": "failed", "message": why})
                self.assert_never_opened()
                self.assertFalse(os.path.exists(os.path.join(self.root, RULE)))

    def test_stops_first_when_a_program_is_missing(self):
        self.kernel = FakeSdKernel(self.root)
        os.unlink(os.path.join(self.bin, "dmesg"))
        code, out = self.run_window()
        self.assertEqual(code, 1, out)
        self.assertEqual(self.status()["message"], "this system lacks programs the SD card needs: dmesg")
        self.assert_never_opened()

    # Surprises once it is open.

    def test_anything_but_the_card_closes_the_window(self):
        self.kernel = FakeSdKernel(self.root, boot_area=True)
        code, out = self.run_window()
        self.assertEqual(code, 1, out)
        self.assertEqual(self.status()["message"], "an eMMC area appeared under the SD controller: mmcblk1boot0")
        self.assert_closed(mounted=False)
        self.assertEqual(self.calls("mount: ") + self.calls("dd: "), [])

    def test_an_emmc_disk_turns_the_mpc_off(self):
        # Only the SD card controller is unbound, so the eMMC disk stays: MPFree never runs with it reachable.
        self.kernel = FakeSdKernel(self.root, emmc_too=True)
        code, out = self.run_window()
        self.assertEqual(code, 2, out)
        self.assertEqual(self.calls("systemctl: "), ["poweroff"])
        self.assertIn("STOP: a block device that is not the SD card's appeared: mmcblk0", self.notes())
        self.assertEqual(self.calls("mount: ") + self.calls("dd: "), [])

    def test_a_card_that_stays_writable_is_not_mounted(self):
        self.kernel = FakeSdKernel(self.root, writable=True)
        code, out = self.run_window()
        self.assertEqual(code, 1, out)
        self.assertEqual(self.status()["message"], "mmcblk1 is not read-only")
        self.assert_closed(mounted=False)
        self.assertEqual(self.calls("mount: ") + self.calls("dd: "), [])

    def test_marks_the_card_read_only_itself_too(self):
        self.kernel = FakeSdKernel(self.root, ignore_rule=True)
        self.start()
        self.wait_state("open")
        self.assertEqual(read_text(os.path.join(self.root, CARD_DIR, "force_ro")), "1\n")
        code, out = self.close_card()
        self.assertEqual(code, 0, out + self.notes())
        self.assert_closed()

    def test_a_mount_that_fails_closes_the_window(self):
        self.kernel = FakeSdKernel(self.root)
        code, out = self.run_window(FAKE_MOUNT_FAIL="1")
        self.assertEqual(code, 1, out)
        self.assertEqual(self.status()["message"], "the card (FAT32) could not be mounted read-only")
        self.assert_closed(mounted=False)

    def test_writes_while_open_fail_it(self):
        self.kernel = FakeSdKernel(self.root)
        self.start()
        self.wait_state("open")
        self.kernel.bump_writes()
        code, out = self.finish()
        self.assertEqual(code, 1, out)
        self.assertRegex(self.status()["message"], r"^the card's write or discard counters moved while it was open")
        self.assert_closed()

    def test_a_write_the_kernel_warned_about_fails_it(self):
        # The kernel lets a write to a read-only disk through, warning once; the counters may not show it yet.
        self.kernel = FakeSdKernel(self.root)
        self.start()
        self.wait_state("open")
        with open(os.path.join(self.root, "kernel-log"), "a") as fh:
            fh.write(REFUSED)
        code, out = self.close_card()
        self.assertEqual(code, 1, out)
        self.assertEqual(self.status()["message"],
                         "something wrote to the read-only card while it was open (the kernel's log says so)")
        self.assert_closed()

    def test_other_disks_warnings_and_earlier_ones_do_not_count(self):
        with open(os.path.join(self.root, "kernel-log"), "a") as fh:
            fh.write("[    9.80] Trying to write to read-only block-device mmcblk0p8\n")   # before the window
        self.kernel = FakeSdKernel(self.root)
        self.start()
        self.wait_state("open")
        with open(os.path.join(self.root, "kernel-log"), "a") as fh:
            fh.write("[  812.40] Trying to write to read-only block-device sda1\n")   # the stick's own root
        code, out = self.close_card()
        self.assertEqual(code, 0, out + self.notes())
        self.assert_closed()

    def test_the_card_taken_out_while_open(self):
        self.kernel = FakeSdKernel(self.root)
        self.start()
        self.wait_state("open")
        self.kernel.take_out()
        code, out = self.finish()
        self.assertEqual(code, 0, out + self.notes())
        self.assertEqual(self.status(), {"state": "closed", "message": "The card was taken out while it was open. "
                                                                       "Nothing was written to it."})
        self.assert_closed()

    def test_a_stop_from_the_system_closes_it(self):
        self.kernel = FakeSdKernel(self.root)
        self.start()
        self.wait_state("open")
        self.proc.send_signal(signal.SIGTERM)
        code, out = self.finish()
        self.assertEqual(code, 0, out + self.notes())
        self.assertEqual(self.status(), {"state": "closed",
                                         "message": "Closed: the system stopped it. Nothing was written to the card."})
        self.assert_closed()

    def test_waits_for_the_pad_player_to_let_go(self):
        # Busy three times: the plain unmount is tried again until it works.
        busy = os.path.join(self.tmp, "busy")
        write(busy, "3")
        self.kernel = FakeSdKernel(self.root)
        self.start(FAKE_BUSY=busy)
        self.wait_state("open")
        code, out = self.close_card()
        self.assertEqual(code, 0, out + self.notes())
        self.assertEqual(self.calls("umount: "), [MNT] * 4)
        self.assert_closed()
        # Busy for good: after LETGO tries, a lazy unmount (the card is read-only, nothing is lost).
        self.tearDown()
        self.setUp()
        write(busy, "1000")
        self.kernel = FakeSdKernel(self.root)
        self.start(FAKE_BUSY=busy)
        self.wait_state("open")
        code, out = self.close_card()
        self.assertEqual(code, 0, out + self.notes())
        self.assertEqual(self.calls("umount: "), [MNT] * 10 + [f"-l {MNT}"])
        self.assertIn("unmounted lazily: MPFree still held a file on the card (read-only)", self.notes())
        self.assert_closed()

    def test_powers_off_if_the_controller_does_not_let_go(self):
        self.kernel = FakeSdKernel(self.root, ignore_unbind=True)
        self.start()
        self.wait_state("open")
        code, out = self.close_card()
        self.assertEqual(code, 2, out)
        self.assertEqual(self.calls("systemctl: "), ["poweroff"])
        self.assertEqual(self.status()["state"], "failed")
        self.assertNotIn(MNT, self.mounts(), "unmounted before it turns the MPC off")

    def test_closes_when_the_pad_player_stops_answering(self):
        alive = os.path.join(self.root, "run/mpfree-app/sd-card.alive")
        write(alive, "00000000000000000007\n")   # an earlier opening's: not this one's
        self.quiet = 5
        self.kernel = FakeSdKernel(self.root)
        self.start()
        self.wait_state("open")
        self.assertFalse(os.path.exists(alive), "an earlier opening's counter is taken away")
        # While the counter moves, the window stays open, far past QUIET checks.
        for n in range(1, 31):
            write(alive, f"{n:020}\n")
            time.sleep(0.05)
        self.assertIsNone(self.proc.poll(), "it stays open while the pad player answers" + self.notes())
        self.assertEqual(self.status()["state"], "open")
        # Then it stops: the window closes as if asked, and checks everything as it does.
        code, out = self.finish()
        self.assertEqual(code, 0, out + self.notes())
        self.assertEqual(self.status(), {"state": "closed",
                                         "message": "Closed: MPFree stopped reading the card. Nothing was written "
                                                    "to it."})
        self.assertIn("closed: MPFree stopped reading the card (its counter stood still for 5 s)", self.notes())
        self.assert_closed()
        self.assert_read_only()
        self.assertFalse(os.path.exists(alive), "the counter is taken away with the window")

    def test_a_pad_player_that_never_answers_still_gets_its_window_closed(self):
        self.quiet = 3
        self.kernel = FakeSdKernel(self.root)
        self.start()
        self.wait_state("open")
        code, out = self.finish()
        self.assertEqual(code, 0, out + self.notes())
        self.assertEqual(self.status()["state"], "closed")
        self.assert_closed()

    def test_every_status_gives_back_the_requests_word(self):
        # Only its hex digits, at most 32 of them: nothing else from the file reaches the status.
        for request, ask in ((ASK + "\n", ASK), ("", ""), ("$(reboot) 'x\n\nab;cd\n", "ebabcd"),
                             ("0123456789abcdef" * 4 + "\n", "0123456789abcdef" * 2)):
            with self.subTest(request=request):
                self.tearDown()
                self.setUp()
                self.kernel = FakeSdKernel(self.root)
                self.start(request=request)
                self.assertEqual(self.wait_state_ask("open", ask)["state"], "open")
                code, out = self.close_card()
                self.assertEqual(code, 0, out + self.notes())
                self.assertEqual(self.status(ask=ask)["state"], "closed")
        # One that stops at the checks says it too.
        self.tearDown()
        self.setUp()
        os.remove(os.path.join(self.root, "run/mpfree-internal-off.txt"))
        code, out = self.run_window()
        self.assertEqual(code, 1, out)
        self.assertEqual(self.status()["state"], "failed")

    def test_a_close_left_from_before_is_not_for_this_opening(self):
        self.kernel = FakeSdKernel(self.root)
        write(os.path.join(self.root, "run/mpfree-app/sd-card.close"), "")
        self.start()
        self.wait_state("open")
        time.sleep(0.3)
        self.assertIsNone(self.proc.poll(), "it stays open")
        code, out = self.close_card()
        self.assertEqual(code, 0, out + self.notes())


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

    def test_never_writes_the_card(self):
        code = self.code()
        # The card is read once, one sector, into a pipe; nothing is written to a device but the kernel log.
        self.assertEqual([m.strip("(\n ") for m in re.findall(r"(?:^|[\s;|&(])dd\s[^\n|]*", code)],
                         ['dd if="$R/dev/$dev" bs=512 count=1 2>/dev/null > "$SECTOR"',
                          'dd if="$SECTOR" bs=1 skip="$1" count=1 2>/dev/null'])
        self.assertIn('SECTOR="$S/sd-card-sector"\n', code)
        self.assertEqual(re.findall(r"\bof=\S+", code), [])
        self.assertEqual(re.findall(r'>\s*"?\$R/dev/[^\s;]+', code), ['> "$R/dev/kmsg"'])
        # One mount, read-only first; no other way to change a filesystem.
        self.assertEqual([m.strip() for m in re.findall(r"(?:^|[\s;|&(])mount\s[^\n]*", code)],
                         ['mount -t "$fs" -o "ro,nosuid,nodev,noexec,noatime,uid=0,gid=$APP_GID,fmask=0137,dmask=0027" '
                          '"$R/dev/$dev" "$R$MNT" \\'])
        for word in ("fsck", "mkfs", "mke2fs", "mkfs.vfat", "mkfs.exfat", "fsck.vfat", "fsck.exfat", "blkdiscard",
                     "hdparm", "sfdisk", "sgdisk", "parted", "wipefs", "tune2fs", "e2fsck", "debugfs", "truncate",
                     "fallocate", "fatlabel", "exfatlabel", "dosfslabel", "blockdev", "chown", "chmod"):
            self.assertNotRegex(code, rf"(^|[\s;|&(]){re.escape(word)}(\s|$)", word)
        # The kernel: only read-only switched on, and the SD controller bound and unbound.
        self.assertEqual(re.findall(r'echo (\S+) > "\$R/sys/block/\$b/force_ro"', code), ["1"])
        self.assertEqual(re.findall(r"ATTR\{[^}]*\}=\S*", code), ['ATTR{force_ro}="1"\''])
        self.assertEqual(re.findall(r'>\s*"?\$R/sys/[^\s;]+', code), ['> "$R/sys/block/$b/force_ro"',
                                                                     '> "$R/sys/bus/platform/drivers_probe"'])
        self.assertIn('echo "$name" > "$devdir/driver/unbind"', code)
        self.assertEqual(re.findall(r'>\s*"?\$R/proc/', code), [])
        # The only controller it names is the SD card's to bind; the eMMC's only to check it has no driver.
        self.assertIn("SD=ff0c0000\nEMMC=ff0f0000\n", code)
        self.assertEqual(re.findall(r'echo "\$name" > "\$R/sys/bus/platform/drivers_probe"', code),
                         ['echo "$name" > "$R/sys/bus/platform/drivers_probe"'])
        self.assertIn("devdir=$d\n", code)
        self.assertEqual(re.findall(r"(?m)^\s*devdir=\S+", code), ["devdir=''", "  devdir=$d"])
        # Deletes: its own files in /run, stale /dev/disk links, the empty mount point.
        self.assertEqual(sorted(set(re.findall(r"\brm -f \"[^\"]+\"(?: \"[^\"]+\")*", code))),
                         sorted({'rm -f "$ASK/sd-card.request" "$ASK/sd-card.close" "$ALIVE"', 'rm -f "$l"',
                                 'rm -f "$RULE"', 'rm -f "$ASK/sd-card.close"', 'rm -f "$S/sd-card-counters-now.txt"',
                                 'rm -f "$ALIVE" "$S/sd-card-sector"'}))
        self.assertIn('ALIVE="$ASK/sd-card.alive"\n', code)
        self.assertNotIn("rm -r", code)
        self.assertIn('rmdir "$R$MNT"', code)
        self.assertIn("MNT=/run/mpfree-sd\n", code)


if __name__ == "__main__":
    unittest.main(verbosity=2)
