#!/usr/bin/env python3
"""Tests for format-spare-stick.py against a fake /sys, /dev, /proc and /run. No real device is opened.

MPFREE_TEST_ROOT makes the tool look for /sys, /dev, /proc and /run under a temporary folder and skip sudo,
so the "disks" here are plain files. Stand-ins for the kernel's partition re-read (blockdev), udev (udevadm)
and the desktop's mounts (udisksctl) come first on PATH; wipefs, mkfs.vfat, mkfs.exfat, mkfs.ext4 and blkid
are the real ones, run on the stand-in partition's file (tests that need one are skipped without it).
"""
import os
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.join(os.path.dirname(HERE), "format-spare-stick.py")
MIB = 1 << 20
SYSTEM_PATH = "/usr/sbin:/usr/bin:/sbin:/bin"


def have(*names):
    return all(shutil.which(n, path=SYSTEM_PATH) for n in names)


def write(path, data=""):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb" if isinstance(data, bytes) else "w") as fh:
        fh.write(data)


# The kernel re-reading a disk's partition table (blockdev --rereadpt, or partx -u adding it): the old
# partitions go, the MBR's first one appears as a file of its size with its sysfs entry. Logs each call.
# FAKE_REREAD_BUSY makes the re-read refused as busy; FAKE_PARTX_FAIL makes partx do nothing.
FAKE_BLOCKDEV = r'''#!{python}
import os, struct, sys, glob, shutil
root = os.environ["MPFREE_TEST_ROOT"]
me = os.path.basename(sys.argv[0])
open(root + "/calls.log", "a").write(me + " " + " ".join(a.replace(root, "") for a in sys.argv[1:]) + "\n")
assert sys.argv[1] == ("--rereadpt" if me == "blockdev" else "-u"), sys.argv
if me == "blockdev" and os.environ.get("FAKE_REREAD_BUSY"):
    print("blockdev: ioctl error on BLKRRPART: Device or resource busy"); sys.exit(1)
if me == "partx" and os.environ.get("FAKE_PARTX_FAIL"):
    sys.exit(0)
disk = sys.argv[2]
name = os.path.basename(disk)
real = os.path.realpath(f"{{root}}/sys/block/{{name}}")
for old in glob.glob(f"{{real}}/{{name}}[0-9]*"):
    shutil.rmtree(old)
    for link in (f"{{root}}/sys/class/block/{{os.path.basename(old)}}", f"{{root}}/dev/{{os.path.basename(old)}}"):
        if os.path.lexists(link):
            os.unlink(link)
with open(disk, "rb") as fh:
    sector = fh.read(512)
assert sector[510:512] == b"\x55\xaa"
kind = sector[450]
start, count = struct.unpack_from("<II", sector, 454)
if kind:
    major, minor = open(real + "/dev").read().strip().split(":")
    part = f"{{real}}/{{name}}1"
    os.makedirs(part + "/holders")
    open(part + "/partition", "w").write("1\n")
    open(part + "/dev", "w").write(f"{{major}}:{{int(minor) + 1}}\n")
    open(part + "/start", "w").write(f"{{start}}\n")
    open(part + "/size", "w").write(f"{{count}}\n")
    os.symlink(part, f"{{root}}/sys/class/block/{{name}}1")
    with open(f"{{root}}/dev/{{name}}1", "wb") as fh:
        fh.truncate(count * 512)
'''

# udev settling: each stand-in partition probed with the real blkid, its record and by-label link made.
FAKE_UDEVADM = r'''#!{python}
import os, subprocess, sys, glob
root = os.environ["MPFREE_TEST_ROOT"]
open(root + "/calls.log", "a").write("udevadm " + " ".join(sys.argv[1:]) + "\n")
labels = root + "/dev/disk/by-label"
os.makedirs(labels, exist_ok=True)
for link in os.listdir(labels):
    if not os.path.exists(os.path.join(labels, link)):
        os.unlink(os.path.join(labels, link))
for dev in sorted(glob.glob(root + "/dev/sd*[0-9]")):
    node = os.path.basename(dev)
    if os.path.exists(f"{{root}}/run/udev/keep-{{node}}"):
        continue
    res = subprocess.run(["blkid", "-p", "-o", "export", dev], stdout=subprocess.PIPE, text=True)
    props = dict(l.split("=", 1) for l in res.stdout.splitlines() if "=" in l)
    number = open(f"{{root}}/sys/class/block/{{node}}/dev").read().strip()
    with open(f"{{root}}/run/udev/data/b{{number}}", "w") as fh:
        for key, ours in (("TYPE", "ID_FS_TYPE"), ("LABEL", "ID_FS_LABEL")):
            if key in props:
                fh.write(f"E:{{ours}}={{props[key]}}\n")
    for link in os.listdir(labels):
        if os.path.realpath(os.path.join(labels, link)) == os.path.realpath(dev):
            os.unlink(os.path.join(labels, link))
    if props.get("LABEL"):
        os.symlink("../../" + node, os.path.join(labels, props["LABEL"]))
'''

# The desktop's mounts: udisksctl mount and unmount, as the user, editing the stand-in mount table.
FAKE_UDISKSCTL = r'''#!{python}
import os, sys
root = os.environ["MPFREE_TEST_ROOT"]
open(root + "/calls.log", "a").write("udisksctl " + " ".join(sys.argv[1:]) + "\n")
verb, dev = sys.argv[1], sys.argv[-1]
node = os.path.basename(dev)
table = root + "/proc/self/mountinfo"
lines = open(table).read().splitlines(True)
if os.environ.get("FAKE_UDISKS_FAIL"):
    print("Error: target is busy"); sys.exit(1)
if verb == "unmount":
    kept = [l for l in lines if l.split(" - ")[1].split()[1] != dev]
    open(table, "w").writelines(kept)
    print(f"Unmounted {{dev}}.")
    sys.exit(0)
number = open(f"{{root}}/sys/class/block/{{node}}/dev").read().strip()
props = dict(l[2:].split("=", 1) for l in open(f"{{root}}/run/udev/data/b{{number}}").read().splitlines())
where = f"/run/media/{{os.environ['USER']}}/{{props['ID_FS_LABEL']}}"
os.makedirs(root + where, exist_ok=True)
with open(table, "a") as fh:
    fh.write(f"90 1 {{number}} / {{where}} rw,nosuid,nodev - {{props['ID_FS_TYPE']}} {{dev}} rw\n")
print(f"Mounted {{dev}} at {{where}}")
'''


class FormatTests(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.mkdtemp(prefix="mpfree-format-test-")
        write(os.path.join(self.root, "proc/self/mountinfo"),
              "22 1 259:2 / / rw,relatime - ext4 /dev/nvme0n1p2 rw\n")
        write(os.path.join(self.root, "proc/swaps"), "Filename\tType\tSize\tUsed\tPriority\n")
        os.makedirs(os.path.join(self.root, "sys/block"))
        os.makedirs(os.path.join(self.root, "sys/class/block"))
        os.makedirs(os.path.join(self.root, "run/udev/data"))
        write(os.path.join(self.root, "sys/bus/usb/devices/2-1.3/speed"), "480\n")
        self.bin = os.path.join(self.root, "bin")
        for name, text in (("blockdev", FAKE_BLOCKDEV), ("partx", FAKE_BLOCKDEV), ("udevadm", FAKE_UDEVADM),
                           ("udisksctl", FAKE_UDISKSCTL)):
            write(os.path.join(self.bin, name), text.format(python=sys.executable))
            os.chmod(os.path.join(self.bin, name), 0o755)
        self.disk("nvme0n1", "259:0", 512 << 30 >> 9, removable="0", usb=False)

    def tearDown(self):
        shutil.rmtree(self.root, ignore_errors=True)

    def disk(self, name, dev, sectors, removable="1", usb=True, serial="FAKE123", ro="0", data=True):
        bus = "0000:00:14.0/usb2/2-1/2-1.3/2-1.3:1.0/host6/target6:0:0/6:0:0:0" if usb else "0000:00:1d.0/nvme"
        real = os.path.join(self.root, "sys/devices/pci0000:00", bus, "block", name)
        write(os.path.join(real, "size"), f"{sectors}\n")
        write(os.path.join(real, "dev"), dev + "\n")
        write(os.path.join(real, "removable"), removable + "\n")
        write(os.path.join(real, "ro"), ro + "\n")
        write(os.path.join(real, "device/vendor"), "FakeVend\n")
        write(os.path.join(real, "device/model"), "Fake Stick\n")
        os.makedirs(os.path.join(real, "holders"))
        os.symlink(real, os.path.join(self.root, "sys/block", name))
        os.symlink(real, os.path.join(self.root, "sys/class/block", name))
        write(os.path.join(self.root, f"run/udev/data/b{dev}"),
              f"E:ID_SERIAL_SHORT={serial}\nE:ID_PART_TABLE_TYPE=dos\n")
        path = os.path.join(self.root, "dev", name)
        write(path, b"")
        if data:
            with open(path, "r+b") as fh:
                fh.truncate(sectors * 512)
                fh.seek(0)
                fh.write(b"\xee" * (3 * MIB))
                fh.seek(sectors * 512 - MIB)
                fh.write(b"\xee" * MIB)
        if usb:
            os.makedirs(os.path.join(self.root, "dev/disk/by-path"), exist_ok=True)
            os.symlink(f"../../{name}", os.path.join(self.root, "dev/disk/by-path",
                                                     f"pci-0000:00:14.0-usb-0:1.3:1.0-scsi-0:0:0:{name[-1]}"))
        return path

    def part(self, disk, n, fs="", label="", partname="", mounted=None):
        """A partition of `disk` with udev's record of it, optionally mounted at `mounted`."""
        real = os.path.realpath(os.path.join(self.root, "sys/block", disk))
        with open(os.path.join(real, "dev")) as fh:
            major, minor = fh.read().strip().split(":")
        dev = f"{major}:{int(minor) + n}"
        part = os.path.join(real, f"{disk}{n}")
        write(os.path.join(part, "partition"), f"{n}\n")
        write(os.path.join(part, "dev"), dev + "\n")
        write(os.path.join(part, "size"), "2048\n")
        write(os.path.join(part, "uevent"), f"PARTNAME={partname}\n" if partname else "")
        os.makedirs(os.path.join(part, "holders"))
        os.symlink(part, os.path.join(self.root, "sys/class/block", f"{disk}{n}"))
        write(os.path.join(self.root, "dev", f"{disk}{n}"), b"\xee" * 4096)
        props = "".join(f"E:{k}={v}\n" for k, v in (("ID_FS_TYPE", fs), ("ID_FS_LABEL", label),
                                                      ("ID_PART_ENTRY_NAME", partname)) if v)
        write(os.path.join(self.root, f"run/udev/data/b{dev}"), props)
        if mounted:
            with open(os.path.join(self.root, "proc/self/mountinfo"), "a") as fh:
                fh.write(f"60 1 {dev} / {mounted.replace(' ', chr(92) + '040')} rw - {fs or 'vfat'} /dev/{disk}{n} rw\n")
        return dev

    def tool(self, *args, answer="ERASE\n", extra=None):
        env = {"PATH": self.bin + ":" + SYSTEM_PATH, "MPFREE_TEST_ROOT": self.root, "HOME": self.root, "USER": "u"}
        env.update(extra or {})
        return subprocess.run([sys.executable, "-I", TOOL, *args], input=answer, env=env, text=True,
                              stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=120)

    def calls(self):
        log = os.path.join(self.root, "calls.log")
        if not os.path.exists(log):
            return []
        with open(log) as fh:
            return fh.read().splitlines()

    def contents(self, path):
        with open(path, "rb") as fh:
            return fh.read()

    def blkid(self, path):
        res = subprocess.run(["blkid", "-p", "-o", "export", path], stdout=subprocess.PIPE, text=True,
                             env={"PATH": SYSTEM_PATH})
        return dict(line.split("=", 1) for line in res.stdout.splitlines() if "=" in line)

    def check_mbr(self, path, kind, sectors):
        with open(path, "rb") as fh:
            head = fh.read(2 * MIB)
            fh.seek(sectors * 512 - MIB)
            tail = fh.read(MIB)
        self.assertEqual(head[510:512], b"\x55\xaa")
        self.assertEqual(head[450], kind)
        self.assertEqual(struct.unpack_from("<II", head, 454), (2048, (sectors // 2048) * 2048 - 2048))
        self.assertEqual(head[462:510], bytes(48), "one partition only")
        self.assertNotEqual(struct.unpack_from("<I", head, 440)[0], 0, "a disk signature")
        self.assertEqual(head[512:], bytes(2 * MIB - 512), "the old table and the new partition's first MiB cleared")
        self.assertEqual(tail, bytes(MIB), "the last MiB (a GPT's backup) cleared")

    # The ordinary way, each filesystem.

    @unittest.skipUnless(have("wipefs", "blkid", "mkfs.vfat", "mkfs.exfat", "mkfs.ext4"), "needs the mkfs tools")
    def test_formats_the_one_stick_and_mounts_it(self):
        sectors = 96 * MIB >> 9
        for flag, label, kind, fs, name in (("--fat32", "WP-FAT", 0x0C, "vfat", "FAT32"),
                                            ("--exfat", "WP-EXFAT", 0x07, "exfat", "exFAT"),
                                            ("--ext4", "WP-TEST", 0x83, "ext4", "ext4")):
            with self.subTest(flag=flag):
                self.tearDown()
                self.setUp()
                disk = self.disk("sdb", "8:16", sectors)
                self.part("sdb", 1, fs="vfat", label="OLD STICK", mounted="/run/media/u/OLD STICK")
                res = self.tool(flag, label)
                out = res.stdout
                self.assertEqual(res.returncode, 0, out)
                # What it shows before it asks.
                self.assertIn("/dev/sdb: FakeVend Fake Stick, serial FAKE123, 0.1 GB (100663296 bytes)", out)
                self.assertIn("CAN be erased", out)
                self.assertIn("plugged in at: USB port 2-1.3 (480 Mbit/s), "
                              "/dev/disk/by-path/pci-0000:00:14.0-usb-0:1.3:1.0-scsi-0:0:0:b", out)
                self.assertIn("partition table: MBR", out)
                self.assertIn("/dev/sdb1: vfat, label OLD STICK, 0.0 GB (1048576 bytes), mounted at "
                              "/run/media/u/OLD STICK", out)
                self.assertIn(f"It becomes one {name} partition labelled {label}", out)
                self.assertIn("Your desktop has it mounted: it is unmounted first", out)
                self.assertNotIn("nvme0n1", out, "internal disks are not even listed")
                self.assertLess(out.index("Type ERASE"), out.index("unmounting /dev/sdb1"))
                # What it did, in order: unmount, settle, the re-read, settle, ..., the mount.
                calls = self.calls()
                self.assertEqual(calls[0], "udisksctl unmount --no-user-interaction -b /dev/sdb1")
                self.assertEqual(calls[1:4], ["udevadm settle --timeout=10", "blockdev --rereadpt /dev/sdb",
                                              "udevadm settle --timeout=10"])
                self.assertNotIn("partx -u /dev/sdb", calls, "not needed when the kernel reads the table")
                self.assertEqual(calls[-1], "udisksctl mount --no-user-interaction -b /dev/sdb1")
                self.check_mbr(disk, kind, sectors)
                found = self.blkid(os.path.join(self.root, "dev/sdb1"))
                self.assertEqual((found.get("TYPE"), found.get("LABEL")), (fs, label))
                self.assertIn(f"READY: /dev/sdb1, {name}, labelled {label}, mounted at /run/media/u/{label}", out)
                if os.environ.get("SHOW_RUN"):
                    print(out)
                with open(os.path.join(self.root, "proc/self/mountinfo")) as fh:
                    mounts = fh.read()
                self.assertNotIn("OLD", mounts)
                self.assertIn(f"/run/media/u/{label} rw,nosuid,nodev - {fs} /dev/sdb1", mounts)
                if fs == "vfat":
                    self.assertEqual(found.get("VERSION"), "FAT32")
                if fs == "ext4" and have("debugfs"):
                    stat = subprocess.run(["debugfs", "-R", "stat /", os.path.join(self.root, "dev/sdb1")],
                                          stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True).stdout
                    self.assertRegex(stat, rf"User:\s+{os.getuid()}\s+Group:\s+{os.getgid()}",
                                     "its top folder is the user's")

    @unittest.skipUnless(have("wipefs", "blkid", "mkfs.vfat"), "needs the mkfs tools")
    def test_a_blank_stick_and_one_the_desktop_mounts_first(self):
        disk = self.disk("sdc", "8:32", 64 * MIB >> 9, data=False)
        res = self.tool("--fat32", "WP-FAT")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("no partitions and no filesystem", res.stdout)
        self.assertNotIn("unmounting", res.stdout)
        self.check_mbr(disk, 0x0C, 64 * MIB >> 9)
        # The desktop mounts the new filesystem before the tool does: it is found there, not mounted twice.
        self.tearDown()
        self.setUp()
        self.disk("sdc", "8:32", 64 * MIB >> 9, data=False)
        with open(os.path.join(self.bin, "udevadm"), "a") as fh:  # the desktop's automount, as udev settles after the filesystem is made
            fh.write("if os.path.exists(root + '/dev/disk/by-label/WP-FAT') and ' /dev/sdc1 ' not in "
                     "open(root + '/proc/self/mountinfo').read():\n"
                     "    open(root + '/proc/self/mountinfo', 'a').write("
                     "'91 1 8:33 / /run/media/u/WP-FAT rw - vfat /dev/sdc1 rw\\n')\n")
        res = self.tool("--fat32", "WP-FAT")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("READY: /dev/sdc1, FAT32, labelled WP-FAT, mounted at /run/media/u/WP-FAT", res.stdout)
        self.assertNotIn("udisksctl mount --no-user-interaction -b /dev/sdc1", self.calls())

    @unittest.skipUnless(have("wipefs", "blkid", "mkfs.vfat"), "needs the mkfs tools")
    def test_a_busy_re_read_is_tried_again_then_partx_adds_the_partition(self):
        disk = self.disk("sdb", "8:16", 64 * MIB >> 9)
        res = self.tool("--fat32", "WP-FAT", extra={"FAKE_REREAD_BUSY": "1"})
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertEqual(res.stdout.count("the kernel's re-read of /dev/sdb was refused (blockdev: ioctl error on "
                                          "BLKRRPART: Device or resource busy); trying again"), 3)
        self.assertIn("/dev/sdb1 not there yet: adding it from the new table (partx -u)", res.stdout)
        self.assertIn("partx -u /dev/sdb", self.calls())
        self.assertIn("READY: /dev/sdb1, FAT32, labelled WP-FAT, mounted at /run/media/u/WP-FAT", res.stdout)
        self.check_mbr(disk, 0x0C, 64 * MIB >> 9)
        # Neither works: it says so, and nothing is made on a partition that isn't there.
        self.tearDown()
        self.setUp()
        self.disk("sdb", "8:16", 64 * MIB >> 9)
        res = self.tool("--fat32", "WP-FAT", extra={"FAKE_REREAD_BUSY": "1", "FAKE_PARTX_FAIL": "1"})
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("STOP: /dev/sdb1 did not appear after the new partition table", res.stdout)
        self.assertIn("The erase did not finish", res.stdout)
        self.assertNotIn("making FAT32", res.stdout)

    # Nothing is changed without ERASE, or when the stick can't be told apart.

    def test_anything_but_erase_stops_and_changes_nothing(self):
        disk = self.disk("sdb", "8:16", 64 * MIB >> 9)
        self.part("sdb", 1, fs="vfat", label="OLD", mounted="/run/media/u/OLD")
        before = self.contents(disk)
        for answer in ("", "erase\n", "yes\n", "ERASE NOW\n"):
            with self.subTest(answer=answer):
                res = self.tool("--fat32", "WP-FAT", answer=answer)
                self.assertEqual(res.returncode, 1, res.stdout)
                self.assertIn("Stopped. Nothing was changed.", res.stdout)
                self.assertEqual(self.contents(disk), before)
                self.assertEqual(self.calls(), [], "not even the unmount")

    def test_refusals(self):
        cases = [
            ("not marked removable", dict(removable="0"), None),
            ("larger than 256 GiB", dict(sectors=(300 << 30) >> 9, data=False), None),
            ("smaller than 1 GB", dict(sectors=8 * MIB >> 9), None),
            ("read-only", dict(ro="1"), None),
            ("the MPFree stick", {}, dict(partname="mpfree-test-root")),
            ("the MPFree stick", {}, dict(fs="ext4", label="MPFREE-DATA")),
            ("/dev/sdb1 is in use at /home/u/backup", {}, dict(fs="ext4", mounted="/home/u/backup")),
            ("/dev/sdb1 is in use at /run/media/someone/X", {}, dict(fs="vfat", mounted="/run/media/someone/X")),
        ]
        for reason, disk_args, part_args in cases:
            with self.subTest(reason=reason):
                self.tearDown()
                self.setUp()
                sectors = disk_args.pop("sectors", 64 * MIB >> 9)
                disk = self.disk("sdb", "8:16", sectors, **disk_args)
                if part_args is not None:
                    self.part("sdb", 1, **part_args)
                before = self.contents(disk)
                res = self.tool("--fat32", "WP-FAT")
                self.assertEqual(res.returncode, 1, res.stdout)
                self.assertIn("not erased: ", res.stdout)
                self.assertIn(reason, res.stdout)
                self.assertIn("STOP: no spare USB stick to erase", res.stdout)
                self.assertNotIn("Type ERASE", res.stdout)
                self.assertEqual(self.contents(disk), before)
                self.assertEqual(self.calls(), [])

    def test_swap_and_stacked_disks_are_refused(self):
        disk = self.disk("sdb", "8:16", 64 * MIB >> 9)
        self.part("sdb", 1)
        with open(os.path.join(self.root, "proc/swaps"), "a") as fh:
            fh.write("/dev/sdb1\tpartition\t1024\t0\t-2\n")
        res = self.tool("--fat32", "WP-FAT")
        self.assertIn("/dev/sdb1 is in use at [swap] (swap)", res.stdout)
        self.assertEqual(res.returncode, 1)
        self.tearDown()
        self.setUp()
        disk = self.disk("sdb", "8:16", 64 * MIB >> 9)
        self.part("sdb", 1)
        os.makedirs(os.path.join(os.path.realpath(os.path.join(self.root, "sys/class/block/sdb1")), "holders/dm-0"))
        res = self.tool("--fat32", "WP-FAT")
        self.assertIn("/dev/sdb1 is in use by LVM, RAID or dm-crypt", res.stdout)
        self.assertEqual(res.returncode, 1)
        self.assertEqual(self.contents(disk)[:MIB], b"\xee" * MIB)

    def test_none_or_two_sticks_stop(self):
        res = self.tool("--fat32", "WP-FAT")
        self.assertIn("USB and removable disks seen:\n  (none)", res.stdout)
        self.assertIn("STOP: no spare USB stick to erase", res.stdout)
        self.disk("sdb", "8:16", 64 * MIB >> 9)
        self.disk("sdc", "8:32", 64 * MIB >> 9, serial="OTHER")
        res = self.tool("--fat32", "WP-FAT")
        self.assertEqual(res.returncode, 1)
        self.assertIn("STOP: more than one USB stick could be erased", res.stdout)
        self.assertEqual(self.calls(), [])

    def test_the_label_on_another_disk_stops(self):
        disk = self.disk("sdb", "8:16", 64 * MIB >> 9)
        before = self.contents(disk)
        nvme = os.path.realpath(os.path.join(self.root, "sys/block/nvme0n1"))
        write(os.path.join(nvme, "nvme0n1p3/partition"), "3\n")
        write(os.path.join(nvme, "nvme0n1p3/dev"), "259:3\n")
        os.symlink(os.path.join(nvme, "nvme0n1p3"), os.path.join(self.root, "sys/class/block/nvme0n1p3"))
        write(os.path.join(self.root, "run/udev/data/b259:3"), "E:ID_FS_TYPE=vfat\nE:ID_FS_LABEL=WP-FAT\n")
        res = self.tool("--fat32", "WP-FAT")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("STOP: /dev/nvme0n1p3 already has a filesystem labelled WP-FAT", res.stdout)
        self.assertNotIn("Type ERASE", res.stdout)
        self.assertEqual(self.contents(disk), before)
        # The stick's own old filesystem with that label is fine: it is the one erased.
        os.unlink(os.path.join(self.root, "run/udev/data/b259:3"))
        self.part("sdb", 1, fs="vfat", label="WP-FAT")
        res = self.tool("--fat32", "WP-FAT", answer="no\n")
        self.assertIn("Type ERASE", res.stdout)

    def test_an_unmount_that_fails_erases_nothing(self):
        disk = self.disk("sdb", "8:16", 64 * MIB >> 9)
        self.part("sdb", 1, fs="vfat", label="OLD", mounted="/run/media/u/OLD")
        before = self.contents(disk)
        res = self.tool("--fat32", "WP-FAT", extra={"FAKE_UDISKS_FAIL": "1"})
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("STOP: /dev/sdb1 could not be unmounted", res.stdout)
        self.assertEqual(self.contents(disk), before)
        self.assertNotIn("blockdev --rereadpt /dev/sdb", self.calls())

    def test_the_privileged_step_checks_again(self):
        disk = self.disk("sdb", "8:16", 64 * MIB >> 9)
        before = self.contents(disk)
        size = str(64 * MIB)
        for args, why in (
                (["sdb", "OTHER", size, "--fat32", "WP-FAT", "0", "0", "u"], "not the stick you confirmed"),
                (["sdb", "FAKE123", str(32 * MIB), "--fat32", "WP-FAT", "0", "0", "u"], "not the stick you confirmed"),
                (["sdb", "FAKE123", size, "--ntfs", "WP-FAT", "0", "0", "u"], "usage"),
                (["sdb", "FAKE123", size, "--fat32", "wp fat", "0", "0", "u"], "usage"),
                (["nvme0n1", "FAKE123", size, "--fat32", "WP-FAT", "0", "0", "u"], "not a disk this tool erases"),
                (["sdb", "FAKE123", size, "--fat32", "WP-FAT", "0", "0"], "usage")):
            with self.subTest(args=args):
                res = self.tool("--root-format", *args)
                self.assertEqual(res.returncode, 1, res.stdout)
                self.assertIn(why, res.stdout)
                self.assertEqual(self.contents(disk), before)
        # Mounted again between the question and the erase, even by the desktop.
        self.part("sdb", 1, fs="vfat", label="OLD", mounted="/run/media/u/OLD")
        res = self.tool("--root-format", "sdb", "FAKE123", size, "--fat32", "WP-FAT", "0", "0", "u")
        self.assertIn("no longer qualifies (something of it is mounted again)", res.stdout)
        self.assertEqual(self.contents(disk), before)
        self.assertEqual(self.calls(), [])

    def test_labels_and_usage(self):
        self.disk("sdb", "8:16", 64 * MIB >> 9)
        for label in ("wp-fat", "WP FAT", "-WP", "TWELVE-CHARS", ""):
            with self.subTest(label=label):
                res = self.tool("--fat32", label)
                self.assertEqual(res.returncode, 1, res.stdout)
                self.assertIn("the label is 1 to 11 capital letters", res.stdout)
        for args in ([], ["--fat32"], ["--ntfs", "X"], ["WP-FAT", "--fat32"]):
            with self.subTest(args=args):
                res = self.tool(*args)
                self.assertEqual(res.returncode, 2, res.stdout)
                self.assertIn("ERASES ONE SPARE USB STICK", res.stdout)
        self.assertEqual(self.calls(), [])

    def test_the_only_ways_it_changes_a_disk(self):
        """Static: one exclusive open for writing, every other change through a named program."""
        with open(TOOL) as fh:
            code = fh.read()
        self.assertEqual(code.count("os.open("), 1)
        self.assertIn('fd = os.open(dev, os.O_RDWR | os.O_EXCL | getattr(os, "O_CLOEXEC", 0))', code)
        self.assertEqual(len(re.findall(r"^\s+write_at\(fd, ", code, re.M)), 3)
        self.assertNotRegex(code, r"\bdd\b|of=|shell=True|os\.system|mkfs\.ntfs|parted|sgdisk|sfdisk")
        self.assertEqual(sorted(set(re.findall(r'program\("([a-z.]+)"\)', code))), ["udisksctl"])
        self.assertIn('mkfs_name = {"vfat": "mkfs.vfat", "exfat": "mkfs.exfat", "ext4": "mkfs.ext4"}[fs]', code)
        self.assertIn('tools = {t: program(t) for t in ("blockdev", "wipefs", "blkid", mkfs_name)}', code)
        self.assertIn('ROOT_PATH = os.environ.get("PATH", "") if TEST_ROOT else "/usr/sbin:/usr/bin:/sbin:/bin"', code)
        # The erase runs only after the typed word, as the last step before sudo.
        main = code[code.index("def main("):]
        self.assertLess(main.index("!= CONFIRM"), main.index('udisks("unmount"'))
        self.assertLess(main.index('udisks("unmount"'), main.index('["sudo"]'))


if __name__ == "__main__":
    unittest.main(verbosity=2)
