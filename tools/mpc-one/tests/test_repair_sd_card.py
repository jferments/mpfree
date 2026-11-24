#!/usr/bin/env python3
"""Tests for repair-sd-card.py against a fake /sys, /dev, /proc and /run. No real device is opened.

MPFREE_TEST_ROOT makes the tool look for /sys, /dev, /proc and /run under a temporary folder and skip sudo,
so the "disks" here are plain files. A stand-in for the desktop's mounts (udisksctl) comes first on PATH;
mkfs.exfat and fsck.exfat are exfatprogs' real ones, run on the stand-in partition's file (the tests that need
them are skipped without them).
"""
import hashlib
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.join(os.path.dirname(HERE), "repair-sd-card.py")
MIB = 1 << 20
SYSTEM_PATH = "/usr/sbin:/usr/bin:/sbin:/bin"
FLAGS = 106


def have(*names):
    return all(shutil.which(n, path=SYSTEM_PATH) for n in names)


def exfatprogs():
    if not have("mkfs.exfat", "fsck.exfat"):
        return False
    res = subprocess.run(["fsck.exfat", "-V"], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
                         env={"PATH": SYSTEM_PATH})
    return "exfatprogs version" in res.stdout


EXFAT = exfatprogs()


def write(path, data=""):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb" if isinstance(data, bytes) else "w") as fh:
        fh.write(data)


# The desktop's mounts: udisksctl mount and unmount, as the user, editing the stand-in mount table.
# FAKE_UDISKS_FAIL makes the unmount refused as busy; FAKE_MOUNT_FAIL makes the mount refused.
FAKE_UDISKSCTL = r'''#!{python}
import os, sys
root = os.environ["MPFREE_TEST_ROOT"]
open(root + "/calls.log", "a").write("udisksctl " + " ".join(sys.argv[1:]) + "\n")
verb, dev = sys.argv[1], sys.argv[-1]
node = os.path.basename(dev)
table = root + "/proc/self/mountinfo"
lines = open(table).read().splitlines(True)
if os.environ.get("FAKE_UDISKS_FAIL"):
    print("Error unmounting " + dev + ": target is busy"); sys.exit(1)
if verb == "mount" and os.environ.get("FAKE_MOUNT_FAIL"):
    print("Error mounting " + dev + ": not authorized"); sys.exit(1)
if verb == "unmount":
    kept = [l for l in lines if l.split(" - ")[1].split()[1] != dev]
    open(table, "w").writelines(kept)
    print(f"Unmounted {{dev}}.")
    sys.exit(0)
number = open(f"{{root}}/sys/class/block/{{node}}/dev").read().strip()
where = f"/run/media/{{os.environ['USER']}}/CARD"
with open(table, "a") as fh:
    fh.write(f"90 1 {{number}} / {{where}} rw,nosuid,nodev - exfat {{dev}} rw\n")
print(f"Mounted {{dev}} at {{where}}")
'''

# exfat-utils' fsck (fuse-exfat's), which takes other options: refused.
FAKE_OLD_FSCK = r'''#!/bin/sh
echo "exfatfsck 1.3.0"
echo "$0 $*" >> "$MPFREE_TEST_ROOT/calls.log"
exit 0
'''


# The real fsck.exfat, after which a desktop mounts the card again (as udisks may when a repair lets go of it).
FAKE_REMOUNTING_FSCK = r'''#!{python}
import os, subprocess, sys
root = os.environ["MPFREE_TEST_ROOT"]
res = subprocess.run(["{fsck}"] + sys.argv[1:])
if "-y" in sys.argv:
    with open(root + "/proc/self/mountinfo", "a") as fh:
        fh.write("91 1 8:17 / /run/media/u/CARD rw - exfat /dev/sdb1 rw\n")
sys.exit(res.returncode)
'''


class RepairTests(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.mkdtemp(prefix="mpfree-repair-test-")
        write(os.path.join(self.root, "proc/self/mountinfo"),
              "22 1 259:2 / / rw,relatime - ext4 /dev/nvme0n1p2 rw\n")
        write(os.path.join(self.root, "proc/swaps"), "Filename\tType\tSize\tUsed\tPriority\n")
        os.makedirs(os.path.join(self.root, "sys/block"))
        os.makedirs(os.path.join(self.root, "sys/class/block"))
        os.makedirs(os.path.join(self.root, "run/udev/data"))
        write(os.path.join(self.root, "sys/bus/usb/devices/2-1.3/speed"), "480\n")
        self.bin = os.path.join(self.root, "bin")
        write(os.path.join(self.bin, "udisksctl"), FAKE_UDISKSCTL.format(python=sys.executable))
        os.chmod(os.path.join(self.bin, "udisksctl"), 0o755)
        self.disk("nvme0n1", "259:0", 512 << 30 >> 9, removable="0", usb=False)

    def tearDown(self):
        shutil.rmtree(self.root, ignore_errors=True)

    # The fake machine.

    def disk(self, name, dev, sectors, removable="1", usb=True, ro="0", mmc_type=None, fs=""):
        """A whole disk: on USB (a card reader), a built-in reader's mmcblk (mmc_type SD or MMC), or internal."""
        if mmc_type:
            bus = "platform/fe320000.mmc/mmc_host/mmc0/mmc0:aaaa"
        elif usb:
            bus = "pci0000:00/0000:00:14.0/usb2/2-1/2-1.3/2-1.3:1.0/host6/target6:0:0/6:0:0:0"
        else:
            bus = "pci0000:00/0000:00:1d.0/nvme/nvme0"
        real = os.path.join(self.root, "sys/devices", bus, "block", name)
        write(os.path.join(real, "size"), f"{sectors}\n")
        write(os.path.join(real, "dev"), dev + "\n")
        write(os.path.join(real, "removable"), removable + "\n")
        write(os.path.join(real, "ro"), ro + "\n")
        if mmc_type:
            write(os.path.join(real, "device/type"), mmc_type + "\n")
            write(os.path.join(real, "device/name"), "SD64G\n")
        else:
            write(os.path.join(real, "device/vendor"), "Generic\n")
            write(os.path.join(real, "device/model"), "STORAGE DEVICE\n")
        os.makedirs(os.path.join(real, "holders"))
        os.symlink(real, os.path.join(self.root, "sys/block", name))
        os.symlink(real, os.path.join(self.root, "sys/class/block", name))
        write(os.path.join(self.root, f"run/udev/data/b{dev}"), f"E:ID_FS_TYPE={fs}\n" if fs else "")
        path = os.path.join(self.root, "dev", name)
        write(path, b"")
        with open(path, "r+b") as fh:
            fh.truncate(sectors * 512)
        return path

    def part(self, disk, n, fs="exfat", label="", partname="", mounted=None, size=32 * MIB, image=True):
        """Partition n of `disk` with udev's record of it; a real exFAT filesystem in it if fs is exfat."""
        real = os.path.realpath(os.path.join(self.root, "sys/block", disk))
        with open(os.path.join(real, "dev")) as fh:
            major, minor = fh.read().strip().split(":")
        dev = f"{major}:{int(minor) + n}"
        node = f"{disk}p{n}" if disk.startswith("mmcblk") else f"{disk}{n}"
        part = os.path.join(real, node)
        write(os.path.join(part, "partition"), f"{n}\n")
        write(os.path.join(part, "dev"), dev + "\n")
        write(os.path.join(part, "size"), f"{size // 512}\n")
        write(os.path.join(part, "uevent"), f"PARTNAME={partname}\n" if partname else "")
        os.makedirs(os.path.join(part, "holders"))
        os.symlink(part, os.path.join(self.root, "sys/class/block", node))
        path = os.path.join(self.root, "dev", node)
        write(path, b"")
        with open(path, "r+b") as fh:
            fh.truncate(size)
        if fs == "exfat" and image:
            self.mkfs(path, label or "CARD")
        props = "".join(f"E:{k}={v}\n" for k, v in (("ID_FS_TYPE", fs), ("ID_FS_LABEL", label),
                                                      ("ID_PART_ENTRY_NAME", partname)) if v)
        write(os.path.join(self.root, f"run/udev/data/b{dev}"), props)
        if mounted:
            self.mount(dev, node, mounted, fs)
        return path

    def mount(self, dev, node, where, fs="exfat"):
        with open(os.path.join(self.root, "proc/self/mountinfo"), "a") as fh:
            fh.write(f"60 1 {dev} / {where.replace(' ', chr(92) + '040')} rw - {fs} /dev/{node} rw\n")

    def mkfs(self, path, label):
        subprocess.run(["mkfs.exfat", "-L", label, path], check=True, stdout=subprocess.DEVNULL,
                       env={"PATH": SYSTEM_PATH})

    def card(self, marked=False, corrupt=False, mounted="/run/media/u/CARD"):
        """The MPC's card in a USB reader: one exFAT partition, optionally marked or with a broken boot region."""
        self.disk("sdb", "8:16", 64 * MIB >> 9)
        path = self.part("sdb", 1, label="CARD", mounted=mounted)
        if marked:
            self.set_flags(path, self.flags(path) | 0x02)
        if corrupt:
            # A byte of the extended boot sector: the boot region's checksum no longer matches, and fsck.exfat
            # restores the region from its backup copy.
            with open(path, "r+b") as fh:
                fh.seek(512 + 100)
                fh.write(b"\x55")
        return path

    def flags(self, path):
        with open(path, "rb") as fh:
            fh.seek(FLAGS)
            return fh.read(1)[0]

    def set_flags(self, path, value):
        with open(path, "r+b") as fh:
            fh.seek(FLAGS)
            fh.write(bytes([value]))

    def digest(self, path):
        with open(path, "rb") as fh:
            return hashlib.sha256(fh.read()).hexdigest()

    def tool(self, *args, answer="REPAIR\n", extra=None, path=None):
        env = {"PATH": path or self.bin + ":" + SYSTEM_PATH, "MPFREE_TEST_ROOT": self.root, "HOME": self.root,
               "USER": "u"}
        env.update(extra or {})
        return subprocess.run([sys.executable, "-I", TOOL, *args], input=answer, env=env, text=True,
                              stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=120)

    def calls(self):
        log = os.path.join(self.root, "calls.log")
        if not os.path.exists(log):
            return []
        with open(log) as fh:
            return fh.read().splitlines()

    def mounted(self, node):
        with open(os.path.join(self.root, "proc/self/mountinfo")) as fh:
            return [line.split()[4] for line in fh if line.split(" - ")[1].split()[1] == f"/dev/{node}"]

    # Check only.

    @unittest.skipUnless(EXFAT, "needs exfatprogs")
    def test_check_only_on_a_clean_card_changes_nothing(self):
        path = self.card()
        before = self.digest(path)
        res = self.tool(answer="")
        out = res.stdout
        self.assertEqual(res.returncode, 0, out)
        self.assertIn("/dev/sdb: Generic STORAGE DEVICE, 0.1 GB (67108864 bytes)\n      the card", out)
        self.assertIn("plugged in at: USB port 2-1.3 (480 Mbit/s)", out)
        self.assertIn("/dev/sdb1: exfat, label CARD, 0.0 GB (33554432 bytes), mounted at /run/media/u/CARD", out)
        self.assertNotIn("nvme0n1", out, "internal disks are not even listed")
        self.assertIn("fsck.exfat: exfatprogs ", out)
        self.assertIn("the not-safely-removed mark: clear", out)
        self.assertIn("running: fsck.exfat -n /dev/sdb1", out)
        self.assertIn("CHECK: clean and unmarked", out)
        self.assertIn("RESULT: the card is clean and its mark is clear. Nothing was changed.", out)
        self.assertNotIn("Type REPAIR", out)
        self.assertIn("The card is left unmounted: take it out now and put it back in the MPC's slot.", out)
        self.assertEqual(self.digest(path), before)
        self.assertEqual(self.calls(), ["udisksctl unmount --no-user-interaction -b /dev/sdb1"])
        self.assertEqual(self.mounted("sdb1"), [], "a clean card is left unmounted, ready to take out")

    @unittest.skipUnless(EXFAT, "needs exfatprogs")
    def test_check_only_reports_the_mark_and_changes_nothing(self):
        path = self.card(marked=True)
        before = self.digest(path)
        res = self.tool(answer="REPAIR\n")
        out = res.stdout
        self.assertEqual(res.returncode, 3, out)
        self.assertIn("the not-safely-removed mark: SET (it wasn't safely removed last time)", out)
        self.assertIn("CHECK: clean, but marked", out)
        self.assertIn("RESULT: the card needs a repair (see above). Nothing was changed.", out)
        self.assertIn("run this again with --repair", out)
        self.assertNotIn("Type REPAIR", out, "check only never asks, even with REPAIR waiting")
        self.assertNotIn("fsck.exfat -y", out)
        self.assertEqual(self.digest(path), before)
        self.assertEqual(self.mounted("sdb1"), ["/run/media/u/CARD"], "mounted again, to copy files off")

    @unittest.skipUnless(EXFAT, "needs exfatprogs")
    def test_check_only_reports_a_broken_filesystem_and_the_disk_error_bit(self):
        path = self.card(corrupt=True)
        self.set_flags(path, self.flags(path) | 0x04)
        before = self.digest(path)
        res = self.tool()
        out = res.stdout
        self.assertEqual(res.returncode, 3, out)
        self.assertIn("the disk-error bit: SET", out)
        self.assertIn("checksum of boot region is not correct", out)
        self.assertIn("CHECK: fsck.exfat found problems (exit code 4)", out)
        self.assertIn("RESULT: the card needs a repair", out)
        self.assertEqual(self.digest(path), before)

    # The repair.

    @unittest.skipUnless(EXFAT, "needs exfatprogs")
    def test_repair_clears_the_mark_after_REPAIR(self):
        path = self.card(marked=True)
        res = self.tool("--repair")
        out = res.stdout
        self.assertEqual(res.returncode, 0, out)
        self.assertIn("CHECK: clean, but marked", out)
        self.assertIn("It can't\nbe undone", out)
        self.assertLess(out.index("Type REPAIR"), out.index("running: fsck.exfat -y /dev/sdb1"))
        self.assertIn("checking it again:", out)
        self.assertEqual(out.count("the not-safely-removed mark: clear"), 1, "after the repair")
        self.assertIn("READY: /dev/sdb1 is clean and unmarked. MPFree can save to it now.", out)
        self.assertEqual(self.flags(path) & 0x06, 0)
        check = subprocess.run(["fsck.exfat", "-n", path], stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                               text=True, env={"PATH": SYSTEM_PATH})
        self.assertEqual(check.returncode, 0, check.stdout)
        # The desktop's mount went before the check and doesn't come back: the card is ready to take out.
        self.assertEqual(self.calls(), ["udisksctl unmount --no-user-interaction -b /dev/sdb1"])
        self.assertEqual(self.mounted("sdb1"), [])
        self.assertIn("The card is left unmounted: take it out now", out)

    @unittest.skipUnless(EXFAT, "needs exfatprogs")
    def test_repair_with_yes_asks_nothing(self):
        """--yes: the SD card round's first command asked for the repair on its command line."""
        path = self.card(marked=True)
        res = self.tool("--repair", "--yes", answer="")
        out = res.stdout
        self.assertEqual(res.returncode, 0, out)
        self.assertNotIn("Type REPAIR", out)
        self.assertLess(out.index("Repairing /dev/sdb1: asked for on the command line (--yes)."),
                        out.index("running: fsck.exfat -y /dev/sdb1"))
        self.assertIn("READY: /dev/sdb1 is clean and unmarked.", out)
        self.assertEqual(self.flags(path) & 0x06, 0)

    def test_yes_goes_only_with_repair(self):
        for args in (["--yes"], ["--yes", "--repair"], ["--repair", "--yes", "--yes"]):
            with self.subTest(args=args):
                res = self.tool(*args)
                self.assertEqual(res.returncode, 2, res.stdout)
                self.assertEqual(self.calls(), [])

    @unittest.skipUnless(EXFAT, "needs exfatprogs")
    def test_repair_mends_a_broken_boot_region(self):
        path = self.card(corrupt=True)
        res = self.tool("--repair")
        out = res.stdout
        self.assertEqual(res.returncode, 0, out)
        self.assertIn("CHECK: fsck.exfat found problems (exit code 4)", out)
        self.assertIn("READY: /dev/sdb1", out)
        check = subprocess.run(["fsck.exfat", "-n", path], stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                               text=True, env={"PATH": SYSTEM_PATH})
        self.assertEqual(check.returncode, 0, check.stdout)

    @unittest.skipUnless(EXFAT, "needs exfatprogs")
    def test_anything_but_REPAIR_changes_nothing(self):
        for answer in ("repair\n", "\n", "yes\n", ""):
            with self.subTest(answer=answer):
                self.tearDown()
                self.setUp()
                path = self.card(marked=True)
                before = self.digest(path)
                res = self.tool("--repair", answer=answer)
                self.assertEqual(res.returncode, 1, res.stdout)
                self.assertIn("Stopped. Nothing was changed.", res.stdout)
                self.assertNotIn("fsck.exfat -y", res.stdout)
                self.assertEqual(self.digest(path), before)
                self.assertEqual(self.mounted("sdb1"), ["/run/media/u/CARD"])

    @unittest.skipUnless(EXFAT, "needs exfatprogs")
    def test_repair_on_a_clean_card_asks_nothing_and_changes_nothing(self):
        path = self.card()
        before = self.digest(path)
        res = self.tool("--repair")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("nothing to repair. Nothing was changed.", res.stdout)
        self.assertIn("The card is left unmounted", res.stdout)
        self.assertNotIn("Type REPAIR", res.stdout)
        self.assertEqual(self.digest(path), before)

    @unittest.skipUnless(EXFAT, "needs exfatprogs")
    def test_a_card_nobody_mounted_is_left_unmounted(self):
        path = self.card(marked=True, mounted=None)
        res = self.tool("--repair")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("READY", res.stdout)
        self.assertEqual(self.flags(path) & 0x02, 0)
        self.assertNotIn("unmounting", res.stdout)
        self.assertEqual(self.calls(), [], "not mounted at the end either")
        self.assertEqual(self.mounted("sdb1"), [])

    @unittest.skipUnless(EXFAT, "needs exfatprogs")
    def test_a_failed_mount_again_is_only_reported(self):
        path = self.card(marked=True)
        before = self.digest(path)
        res = self.tool(extra={"FAKE_MOUNT_FAIL": "1"})
        self.assertEqual(res.returncode, 3, res.stdout)
        self.assertIn("not authorized (open it from the file manager instead)", res.stdout)
        self.assertIn("RESULT: the card needs a repair", res.stdout)
        self.assertNotIn("STOP", res.stdout)
        self.assertEqual(self.digest(path), before)

    @unittest.skipUnless(EXFAT, "needs exfatprogs")
    def test_a_desktop_that_mounts_it_again_is_told_to_eject_it(self):
        path = self.card(marked=True)
        # The desktop mounts the card again once fsck.exfat -y lets go of it.
        real = shutil.which("fsck.exfat", path=SYSTEM_PATH)
        write(os.path.join(self.bin, "fsck.exfat"), FAKE_REMOUNTING_FSCK.format(python=sys.executable, fsck=real))
        os.chmod(os.path.join(self.bin, "fsck.exfat"), 0o755)
        res = self.tool("--repair")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("READY: /dev/sdb1", res.stdout)
        self.assertIn("Your desktop has mounted the card again: eject it in the file manager", res.stdout)
        self.assertNotIn("left unmounted", res.stdout)
        self.assertEqual(self.flags(path) & 0x02, 0)

    @unittest.skipUnless(EXFAT, "needs exfatprogs")
    def test_built_in_sd_reader_and_whole_disk_exfat(self):
        # An SD card in a laptop's own reader: mmcblkN whose type is SD.
        self.disk("mmcblk0", "179:0", 64 * MIB >> 9, removable="0", mmc_type="SD")
        path = self.part("mmcblk0", 1, mounted="/media/u/CARD")
        self.set_flags(path, 0x02)
        res = self.tool("--repair")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("/dev/mmcblk0: SD card SD64G", res.stdout)
        self.assertIn("plugged in at: a built-in card reader", res.stdout)
        self.assertIn("READY: /dev/mmcblk0p1", res.stdout)
        self.assertEqual(self.flags(path) & 0x02, 0)
        # exFAT on the whole card, no partition table.
        self.tearDown()
        self.setUp()
        disk = self.disk("sdc", "8:32", 32 * MIB >> 9, fs="exfat")
        self.mkfs(disk, "CARD")
        self.set_flags(disk, 0x02)
        res = self.tool("--repair")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("READY: /dev/sdc is clean", res.stdout)
        self.assertEqual(self.flags(disk) & 0x02, 0)

    # What it refuses. Each refusal changes nothing and runs neither fsck nor udisksctl.

    def refused(self, res, *words, code=1):
        self.assertEqual(res.returncode, code, res.stdout)
        self.assertIn("STOP: ", res.stdout)
        for word in words:
            self.assertIn(word, res.stdout)
        self.assertNotIn("running: fsck.exfat", res.stdout)
        self.assertEqual(self.calls(), [])

    def test_no_card(self):
        res = self.tool("--repair")
        self.refused(res, "USB, removable and SD card disks seen:\n  (none)", "no exFAT SD card found", code=5)

    @unittest.skipUnless(EXFAT, "needs exfatprogs")
    def test_refuses_anything_that_is_not_the_card(self):
        cases = {
            "another filesystem": (dict(), dict(fs="vfat", image=False), "its filesystem is vfat, not exFAT"),
            "no filesystem": (dict(), dict(fs="", image=False), "no filesystem"),
            "read-only": (dict(ro="1"), dict(), "read-only (the card's lock switch may be on)"),
            "not removable": (dict(removable="0"), dict(), "not marked removable"),
            "mounted elsewhere": (dict(), dict(mounted="/mnt/card"), "/dev/sdb1 is in use at /mnt/card"),
            "someone else's desktop": (dict(), dict(mounted="/run/media/other/CARD"), "in use at /run/media/other"),
            "the MPFree stick": (dict(), dict(label="MPFREE-DATA"), "the MPFree stick"),
            "an mpfree-test partition": (dict(), dict(partname="mpfree-test-boot"), "the MPFree stick"),
            "too small": (dict(sectors=8 * MIB >> 9), dict(size=4 * MIB, image=False), "smaller than 1 GB"),
        }
        for what, (disk, part, reason) in cases.items():
            with self.subTest(what=what):
                self.tearDown()
                self.setUp()
                self.disk("sdb", "8:16", disk.pop("sectors", 64 * MIB >> 9), **disk)
                self.part("sdb", 1, **part)
                res = self.tool("--repair")
                # No disk qualifies: exit code 5, as with no card at all.
                self.refused(res, "not the card: ", reason, "no exFAT SD card found", code=5)

    @unittest.skipUnless(EXFAT, "needs exfatprogs")
    def test_refuses_two_partitions_an_emmc_and_stacked_disks(self):
        self.disk("sdb", "8:16", 128 * MIB >> 9)
        self.part("sdb", 1)
        self.part("sdb", 2)
        self.disk("mmcblk1", "179:32", 64 * MIB >> 9, removable="0", mmc_type="MMC")
        self.part("mmcblk1", 1)
        self.disk("sdd", "8:48", 64 * MIB >> 9)
        self.part("sdd", 1)
        os.makedirs(os.path.join(self.root, "sys/class/block/sdd1/holders/dm-0"))
        res = self.tool("--repair")
        self.refused(res, "2 partitions (the MPC's card has one)",
                     "not an SD card (an eMMC or another MMC device)",
                     "/dev/sdd1 is in use by LVM, RAID or dm-crypt", code=5)

    @unittest.skipUnless(EXFAT, "needs exfatprogs")
    def test_refuses_two_cards(self):
        self.card()
        self.disk("sdc", "8:32", 64 * MIB >> 9)
        self.part("sdc", 1)
        res = self.tool("--repair")
        self.refused(res, "more than one disk could be the card")

    @unittest.skipUnless(EXFAT, "needs exfatprogs")
    def test_stops_if_the_desktop_keeps_the_card(self):
        path = self.card(marked=True)
        before = self.digest(path)
        res = self.tool("--repair", extra={"FAKE_UDISKS_FAIL": "1"})
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("could not be unmounted: close any window or program showing the card", res.stdout)
        self.assertNotIn("running: fsck.exfat", res.stdout)
        self.assertEqual(self.digest(path), before)

    @unittest.skipUnless(EXFAT, "needs exfatprogs")
    def test_refuses_another_fsck_exfat(self):
        path = self.card(marked=True)
        before = self.digest(path)
        write(os.path.join(self.bin, "fsck.exfat"), FAKE_OLD_FSCK)
        os.chmod(os.path.join(self.bin, "fsck.exfat"), 0o755)
        res = self.tool("--repair")
        self.assertIn("is not exfatprogs' fsck.exfat (install exfatprogs); nothing was changed", res.stdout)
        self.assertIn("The check did not finish", res.stdout)
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertEqual([c for c in self.calls() if "fsck.exfat" in c and "-V" not in c], [],
                         "only asked for its version")
        self.assertEqual(self.digest(path), before)
        self.assertEqual(self.mounted("sdb1"), ["/run/media/u/CARD"], "mounted again after the stop")

    def test_missing_exfatprogs(self):
        # A PATH with only the stand-ins and Python: no fsck.exfat at all.
        lone = os.path.join(self.root, "lone")
        os.makedirs(lone)
        os.symlink(sys.executable, os.path.join(lone, "python3"))
        self.disk("sdb", "8:16", 64 * MIB >> 9)
        path = self.part("sdb", 1, image=False)
        write(path, b"\0" * 4096)
        res = self.tool(path=self.bin + ":" + lone)
        self.assertIn("fsck.exfat is not installed (it comes with exfatprogs)", res.stdout)
        self.assertEqual(res.returncode, 1, res.stdout)

    @unittest.skipUnless(EXFAT, "needs exfatprogs")
    def test_the_privileged_step_checks_the_disk_again(self):
        self.card(marked=True, mounted=None)
        size = str(64 * MIB)
        res = self.tool("--root-repair", "sdb", str(64 * MIB + 512), "sdb1", "u")
        self.assertIn("its size changed); nothing was changed", res.stdout)
        res = self.tool("--root-repair", "sdb", size, "sdb2", "u")
        self.assertIn("/dev/sdb2 is not the card's filesystem; nothing was changed", res.stdout)
        res = self.tool("--root-repair", "nvme0n1", size, "nvme0n1p1", "u")
        self.assertIn("nvme0n1 is not a disk this tool checks", res.stdout)
        self.mount("8:17", "sdb1", "/run/media/u/CARD")
        res = self.tool("--root-repair", "sdb", size, "sdb1", "u")
        self.assertIn("no longer qualifies (something of it is mounted again); nothing was changed", res.stdout)
        res = self.tool("--root-check", "sdb", "x", "sdb1", "u")
        self.assertIn("usage: --root-check|--root-repair", res.stdout)
        self.assertNotIn("running: fsck.exfat", "".join([res.stdout]))
        self.assertEqual(self.calls(), [])

    @unittest.skipUnless(EXFAT, "needs exfatprogs")
    def test_a_partition_that_is_not_exfat_inside_is_not_touched(self):
        # udev says exfat but the first sector says otherwise (the card changed since udev looked).
        self.disk("sdb", "8:16", 64 * MIB >> 9)
        path = self.part("sdb", 1, image=False)
        write(path, b"\xee" * (32 * MIB))
        before = self.digest(path)
        res = self.tool("--repair")
        self.assertIn("first sector is not exFAT's; nothing was changed", res.stdout)
        self.assertNotIn("running: fsck.exfat", res.stdout)
        self.assertEqual(self.digest(path), before)

    def test_bad_arguments_print_the_help(self):
        res = self.tool("--fix")
        self.assertEqual(res.returncode, 2)
        self.assertIn("python3 tools/mpc-one/repair-sd-card.py --repair", res.stdout)
        self.assertEqual(self.calls(), [])


if __name__ == "__main__":
    unittest.main()
