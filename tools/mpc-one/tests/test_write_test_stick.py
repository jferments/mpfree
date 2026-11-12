#!/usr/bin/env python3
"""Tests for write-test-stick.py against a fake /sys, /dev and /proc. No real device is opened.

MPFREE_TEST_ROOT makes the tool look for /sys, /dev, /proc and /run under a
temporary folder and skip sudo, so the "devices" here are plain files.
"""
import glob
import hashlib
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import unittest
import uuid
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.join(os.path.dirname(HERE), "write-test-stick.py")
IMAGE = bytes(range(256)) * 4096 * 3          # 3 MiB, not a multiple of the 4 MiB chunk
MIB = 1 << 20
SBIN_PATH = "/usr/sbin:/usr/bin:/sbin:/bin"


def data_fs(size, fill=b"\x11", label=b"MPFREE-DATA"):
    """Bytes that pass for partition 4's ext4: the superblock's magic, label and size (1 KiB blocks), the
    rest `fill`."""
    data = bytearray(fill * size)
    struct.pack_into("<I", data, 1024 + 0x04, size // 1024)
    struct.pack_into("<I", data, 1024 + 0x18, 0)
    struct.pack_into("<III", data, 1024 + 0x5C, 0, 0, 0)     # no features, so no superblock checksum
    data[1024 + 0x38:1024 + 0x3A] = b"\x53\xef"
    data[1024 + 0x78:1024 + 0x88] = label.ljust(16, b"\0")
    return bytes(data)


def gpt_disk(sectors, parts, fill=b"\0"):
    """A disk of `sectors` sectors with a GPT holding parts [(first sector, sectors, name, contents)]."""
    disk = bytearray(fill * (sectors * 512))
    entries = bytearray(128 * 128)
    for i, (first, size, name, data) in enumerate(parts):
        entry = bytearray(128)
        entry[0:16] = uuid.UUID("0FC63DAF-8483-4772-8E79-3D69D8477DE4").bytes_le
        entry[16:32] = uuid.uuid4().bytes_le
        struct.pack_into("<QQ", entry, 32, first, first + size - 1)
        label = name.encode("utf-16-le")
        entry[56:56 + len(label)] = label
        entries[i * 128:(i + 1) * 128] = entry
        disk[first * 512:first * 512 + len(data)] = data
    head = bytearray(92)
    head[0:8] = b"EFI PART"
    struct.pack_into("<II", head, 8, 0x10000, 92)
    struct.pack_into("<QQQQ", head, 24, 1, sectors - 1, 34, sectors - 34)
    head[56:72] = uuid.uuid4().bytes_le
    struct.pack_into("<QIII", head, 72, 2, 128, 128, zlib.crc32(entries))
    struct.pack_into("<I", head, 16, zlib.crc32(head))
    mbr = bytearray(512)
    mbr[446 + 4] = 0xEE
    struct.pack_into("<II", mbr, 446 + 8, 1, min(sectors - 1, 0xFFFFFFFF))
    mbr[510:512] = b"\x55\xaa"
    disk[0:512] = mbr
    disk[512:604] = head
    disk[1024:1024 + len(entries)] = entries
    return bytes(disk)


def stick_layout(data_first, data, extra=()):
    """Partitions as offline-kexec-stick.py --app makes them, with partition 4 at `data_first`."""
    return [(2048, 2048, "mpfree-test-root", b"\x01" * 4096), (4096, 1024, "mpfree-test-log", b""),
            (5120, 1024, "mpfree-test-kits", b"\x03" * 4096), (data_first, len(data) // 512, "mpfree-test-data", data),
            *extra]


def write(path, data=""):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb" if isinstance(data, bytes) else "w") as fh:
        fh.write(data)


class WriterTests(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.mkdtemp(prefix="mpfree-stick-writer-test-")
        self.work = os.path.join(self.root, "work")
        folder = os.path.join(self.work, "kexec-stick-20260101T000000Z-abc")
        write(os.path.join(folder, "stick.img"), IMAGE)
        write(os.path.join(folder, "SHA256SUMS"), f"{hashlib.sha256(IMAGE).hexdigest()}  stick.img\n")
        write(os.path.join(self.root, "proc/self/mountinfo"),
              "22 1 259:2 / / rw,relatime - ext4 /dev/nvme0n1p2 rw\n")
        write(os.path.join(self.root, "proc/swaps"), "Filename\tType\tSize\tUsed\tPriority\n")
        os.makedirs(os.path.join(self.root, "sys/block"))
        self.disk("nvme0n1", "259:0", 512 << 30 >> 9, removable="0", usb=False)

    def tearDown(self):
        shutil.rmtree(self.root, ignore_errors=True)

    def disk(self, name, dev, sectors, removable="1", usb=True, serial="FAKE123", parts=1):
        bus = "usb2/2-1/2-1:1.0/host6" if usb else "0000:00:1d.0/nvme"
        real = os.path.join(self.root, "sys/devices/pci0000:00", bus, "block", name)
        major, minor = dev.split(":")
        write(os.path.join(real, "size"), f"{sectors}\n")
        write(os.path.join(real, "dev"), dev + "\n")
        write(os.path.join(real, "removable"), removable + "\n")
        write(os.path.join(real, "ro"), "0\n")
        write(os.path.join(real, "device/vendor"), "FakeVend\n")
        write(os.path.join(real, "device/model"), "Fake Stick\n")
        os.makedirs(os.path.join(real, "holders"))
        os.symlink(real, os.path.join(self.root, "sys/block", name))
        write(os.path.join(self.root, f"run/udev/data/b{dev}"), f"E:ID_SERIAL_SHORT={serial}\n")
        os.makedirs(os.path.join(self.root, "sys/class/block"), exist_ok=True)
        os.symlink(real, os.path.join(self.root, "sys/class/block", name))
        write(os.path.join(self.root, "dev", name), b"\xee" * (sectors * 512 if usb else 16))
        for n in range(1, parts + 1):
            part = os.path.join(real, f"{name}{n}")
            write(os.path.join(part, "partition"), f"{n}\n")
            write(os.path.join(part, "dev"), f"{major}:{int(minor) + n}\n")
            os.makedirs(os.path.join(part, "holders"))
            os.symlink(part, os.path.join(self.root, "sys/class/block", f"{name}{n}"))
        return os.path.join(self.root, "dev", name)

    def writer(self, answer="WRITE\n", *args):
        env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "MPFREE_TEST_ROOT": self.root,
               "MPFREE_WORK_DIR": self.work, "HOME": self.root}
        return subprocess.run([sys.executable, "-I", TOOL, *args], input=answer, env=env, text=True,
                              stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=60)

    def contents(self, dev):
        with open(dev, "rb") as fh:
            return fh.read()

    def test_writes_the_one_stick_and_reads_it_back(self):
        dev = self.disk("sdb", "8:16", 16 * MIB >> 9)
        res = self.writer()
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("/dev/sdb: FakeVend Fake Stick, serial FAKE123, 16777216 bytes", res.stdout)
        self.assertIn("CAN be written", res.stdout)
        self.assertIn("they match stick.img", res.stdout)
        self.assertNotIn("nvme0n1", res.stdout, "internal disks are not even listed")
        data = self.contents(dev)
        self.assertEqual(data[:len(IMAGE)], IMAGE)
        self.assertEqual(data[len(IMAGE):], b"\xee" * (16 * MIB - len(IMAGE)), "nothing past the image")

    def test_anything_but_write_stops(self):
        dev = self.disk("sdb", "8:16", 16 * MIB >> 9)
        for answer in ("write\n", "\n", "yes\n", ""):
            res = self.writer(answer)
            self.assertEqual(res.returncode, 1, res.stdout)
            self.assertEqual(self.contents(dev), b"\xee" * 16 * MIB)

    def refused(self, reason):
        res = self.writer()
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn(reason, res.stdout)
        self.assertIn("STOP: no USB stick to write", res.stdout)
        return res

    def test_refusals(self):
        dev = self.disk("sdb", "8:16", 16 * MIB >> 9)
        with self.subTest("mounted partition"):
            with open(os.path.join(self.root, "proc/self/mountinfo"), "a") as fh:
                fh.write("40 22 8:17 / /run/media/user/STICK rw - vfat /dev/sdb1 rw\n")
            self.refused("/dev/sdb1 is mounted or used as swap")
        write(os.path.join(self.root, "proc/self/mountinfo"), "22 1 259:2 / / rw - ext4 /dev/nvme0n1p2 rw\n")
        with self.subTest("swap"):
            write(os.path.join(self.root, "proc/swaps"), "Filename Type\n/dev/sdb partition 1 0 -2\n")
            self.refused("/dev/sdb is mounted or used as swap")
        write(os.path.join(self.root, "proc/swaps"), "Filename Type\n")
        with self.subTest("holder"):
            holders = os.path.join(self.root, "sys/class/block/sdb1/holders")
            os.symlink("../../dm-0", os.path.join(holders, "dm-0"))
            self.refused("/dev/sdb1 is in use by LVM, RAID or dm-crypt")
            os.unlink(os.path.join(holders, "dm-0"))
        with self.subTest("not removable"):
            write(os.path.join(self.root, "sys/block/sdb/removable"), "0\n")
            self.refused("not marked removable")
            write(os.path.join(self.root, "sys/block/sdb/removable"), "1\n")
        with self.subTest("too small"):
            write(os.path.join(self.root, "sys/block/sdb/size"), "1024\n")
            self.refused("smaller than stick.img")
        with self.subTest("too large"):
            write(os.path.join(self.root, "sys/block/sdb/size"), f"{(300 << 30) >> 9}\n")
            self.refused("larger than 256 GiB")
        self.assertEqual(self.contents(dev), b"\xee" * 16 * MIB)

    def test_two_sticks_stop(self):
        a = self.disk("sdb", "8:16", 16 * MIB >> 9)
        b = self.disk("sdc", "8:32", 16 * MIB >> 9, serial="FAKE456")
        res = self.writer()
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("more than one USB stick could be written", res.stdout)
        self.assertEqual(self.contents(a), b"\xee" * 16 * MIB)
        self.assertEqual(self.contents(b), b"\xee" * 16 * MIB)

    def test_damaged_image_stops(self):
        dev = self.disk("sdb", "8:16", 16 * MIB >> 9)
        write(os.path.join(self.work, "kexec-stick-20260101T000000Z-abc", "stick.img"), IMAGE[:-1] + b"X")
        res = self.writer()
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("does not match its SHA256SUMS line", res.stdout)
        self.assertEqual(self.contents(dev), b"\xee" * 16 * MIB)

    def test_privileged_step_checks_again(self):
        dev = self.disk("sdb", "8:16", 16 * MIB >> 9)
        image = os.path.join(self.work, "kexec-stick-20260101T000000Z-abc", "stick.img")
        digest = hashlib.sha256(IMAGE).hexdigest()
        with open(os.path.join(self.root, "proc/self/mountinfo"), "a") as fh:
            fh.write("40 22 8:17 / /mnt rw - vfat /dev/sdb1 rw\n")
        res = self.writer("", "--root-write", "sdb", str(len(IMAGE)), image, digest)
        self.assertIn("usage: --root-write NAME SIZE IMAGE SHA256 DATA", res.stdout)
        res = self.writer("", "--root-write", "sdb", str(len(IMAGE)), image, digest, "none")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("/dev/sdb no longer qualifies", res.stdout)
        write(os.path.join(self.root, "proc/self/mountinfo"), "22 1 259:2 / / rw - ext4 /dev/nvme0n1p2 rw\n")
        for name in ("nvme0n1", "../sdb", "sdb1"):
            res = self.writer("", "--root-write", name, str(len(IMAGE)), image, digest, "none")
            self.assertEqual(res.returncode, 1, res.stdout)
        res = self.writer("", "--root-write", "sdb", str(len(IMAGE)), image, "0" * 64, "none")
        self.assertIn("stick.img changed since it was checked", res.stdout)
        res = self.writer("", "--root-write", "sdb", str(len(IMAGE)), image, digest, "keep")
        self.assertIn("the data partition mode does not match stick.img", res.stdout)
        self.assertEqual(self.contents(dev), b"\xee" * 16 * MIB)


def backup_gpt_ok(disk):
    """True when the disk's last sector holds a valid backup GPT header whose entries check out."""
    sectors = len(disk) // 512
    head = disk[-512:]
    if head[:8] != b"EFI PART":
        return False
    size, crc = struct.unpack_from("<II", head, 12)
    if zlib.crc32(head[:16] + bytes(4) + head[20:size]) != crc:
        return False
    mine, other = struct.unpack_from("<QQ", head, 24)
    table_lba, count, entry_size, table_crc = struct.unpack_from("<QIII", head, 72)
    table = disk[table_lba * 512:table_lba * 512 + count * entry_size]
    return mine == sectors - 1 and other == 1 and zlib.crc32(table) == table_crc


def partx(path):
    """[(number, first sector, sectors, name)] as util-linux reads the disk's GPT, or None without partx."""
    if not shutil.which("partx", path=SBIN_PATH):
        if os.environ.get("MPFREE_REQUIRE_OFFLINE_TOOLS") == "1":
            raise AssertionError("partx is required (MPFREE_REQUIRE_OFFLINE_TOOLS=1)")
        return None
    out = subprocess.run(["partx", "--show", "--noheadings", "-o", "NR,START,SECTORS,NAME", path],
                         stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
                         env={**os.environ, "PATH": SBIN_PATH})
    assert out.returncode == 0, out.stdout
    return [(int(a), int(b), int(c), d) for a, b, c, d in (line.split() for line in out.stdout.splitlines())]


def real_ext4(path, size, label="MPFREE-DATA"):
    """A real ext4 made by mke2fs (with metadata_csum, so its superblock has a checksum), or None."""
    if not shutil.which("mke2fs", path=SBIN_PATH):
        if os.environ.get("MPFREE_REQUIRE_OFFLINE_TOOLS") == "1":
            raise AssertionError("mke2fs is required (MPFREE_REQUIRE_OFFLINE_TOOLS=1)")
        return None
    res = subprocess.run(["mke2fs", "-q", "-F", "-t", "ext4", "-O", "^has_journal,metadata_csum", "-L", label,
                          path, f"{size // 1024}k"], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
                         env={**os.environ, "PATH": SBIN_PATH})
    assert res.returncode == 0, res.stdout
    with open(path, "rb") as fh:
        return fh.read()


class DataPartitionTests(WriterTests):
    """Partition 4 (saved settings): left in place, moved from the old layout with a copy first, or empty;
    and what a write cut off at each step leaves."""
    NEW = data_fs(1 << 20)                      # the image's empty one, 1 MiB here (64 MiB in a real stick.img)
    START = 8192                                # where the image's partition 4 starts (1 GiB in a real one)
    OLD_START = 6144                            # where the old layout put it: straight after partition 3
    STICK = 16 * MIB >> 9
    END = 30719                                 # the last 1 MiB boundary before the stick's backup table
    MOVE_WRITES = 9                             # 6 for partition 4, 1 for partitions 1 to 3, 2 for the tables
    KEEP_WRITES = 3

    def setUp(self):
        super().setUp()
        self.folder = os.path.join(self.work, "kexec-stick-20260101T000000Z-abc")
        self.image = gpt_disk(12288, stick_layout(self.START, self.NEW))
        write(os.path.join(self.folder, "stick.img"), self.image)
        write(os.path.join(self.folder, "SHA256SUMS"), f"{hashlib.sha256(self.image).hexdigest()}  stick.img\n")
        self.cut = None

    def writer(self, answer="WRITE\n", *args):
        env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "MPFREE_TEST_ROOT": self.root,
               "MPFREE_WORK_DIR": self.work, "HOME": self.root}
        if self.cut is not None:
            env["MPFREE_TEST_CUT_AFTER"] = str(self.cut)
        return subprocess.run([sys.executable, "-I", TOOL, *args], input=answer, env=env, text=True,
                              stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=60)

    def old_stick(self, data, first=None, sectors=None):
        """A stick written by an earlier round: the old layout (partition 4 straight after partition 3) by
        default, or this layout with first=START."""
        first = self.OLD_START if first is None else first
        dev = os.path.join(self.root, "dev/sdb")
        if not os.path.exists(dev):
            self.disk("sdb", "8:16", self.STICK)
        size = (sectors or len(data) // 512) * 512
        layout = stick_layout(first, data.ljust(size, b"\0"))
        write(dev, gpt_disk(self.STICK, layout, fill=b"\xee"))
        return dev

    def copies(self):
        return sorted(glob.glob(os.path.join(self.folder, "data-from-old-stick-*.img")))

    def assert_written(self, dev, data):
        """The stick holds the image's partitions 1 to 3, `data` at the start of partition 4, and a partition
        table (both copies) with partition 4 running to the end of the stick."""
        disk = self.contents(dev)
        # From the end of the main table to the end of partition 3, the gap before partition 1 included.
        self.assertEqual(disk[34 * 512:6144 * 512], self.image[34 * 512:6144 * 512])
        self.assertEqual(disk[self.START * 512:self.START * 512 + len(data)], data)
        want = [(1, 2048, 2048, "mpfree-test-root"), (2, 4096, 1024, "mpfree-test-log"),
                (3, 5120, 1024, "mpfree-test-kits"), (4, self.START, self.END - self.START + 1, "mpfree-test-data")]
        self.assertEqual(self.tool().gpt_partitions(os.open(dev, os.O_RDONLY)), want)
        self.assertTrue(backup_gpt_ok(disk), "the backup partition table checks out")
        found = partx(dev)
        if found is not None:
            self.assertEqual(found, want, "util-linux reads the same table")

    def tool(self):
        import importlib.util
        spec = importlib.util.spec_from_file_location("write_test_stick", TOOL)
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        return mod

    def test_left_in_place_when_it_is_already_where_it_goes(self):
        saved = data_fs(2 << 20, fill=b"\x22")
        dev = self.old_stick(saved, first=self.START, sectors=self.END - self.START + 1)
        before = self.contents(dev)[self.START * 512:(self.END + 1) * 512]
        res = self.writer()
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("Partition 4 (saved settings and files) fills the rest of the stick.", res.stdout)
        self.assertIn("already where it goes, so it stays in place (2097152 bytes of filesystem); it is not written",
                      res.stdout)
        self.assertIn("they match stick.img's partitions 1 to 3, partition 4 and a partition table", res.stdout)
        self.assert_written(dev, saved)
        self.assertEqual(self.contents(dev)[self.START * 512:(self.END + 1) * 512], before, "not one byte changed")
        self.assertEqual(self.copies(), [])

    def test_left_in_place_survives_a_cut_off_write_anywhere(self):
        saved = data_fs(2 << 20, fill=b"\x22")
        for cut in range(self.KEEP_WRITES):
            with self.subTest(cut=cut):
                dev = self.old_stick(saved, first=self.START, sectors=2048 * 4)
                before = self.contents(dev)[self.START * 512:(self.END + 1) * 512]
                self.cut = cut
                res = self.writer()
                self.assertEqual(res.returncode, 1, res.stdout)
                self.assertIn("the write was cut off (test)", res.stdout)
                self.assertIn("are still on it: run this again", res.stdout)
                self.assertEqual(self.contents(dev)[self.START * 512:(self.END + 1) * 512], before)
                self.cut = None
                res = self.writer()
                self.assertEqual(res.returncode, 0, res.stdout)
                self.assertIn("so it stays in place", res.stdout)
                self.assert_written(dev, saved)

    def test_the_old_layouts_partition_is_copied_first_then_moved(self):
        old = data_fs(1 << 20, fill=b"\x22")
        dev = self.old_stick(old)
        res = self.writer()
        self.assertEqual(res.returncode, 0, res.stdout)
        [copy] = self.copies()
        self.assertIn(f"partition 4: moving {len(old)} bytes (sha256 {hashlib.sha256(old).hexdigest()}) to where "
                      f"it now starts; a copy is in {copy}", res.stdout)
        self.assertIn(f"partition 4: {(self.END - self.START + 1) * 512} bytes, from 4 MiB to the end", res.stdout)
        self.assert_written(dev, old)
        self.assertEqual(self.contents(copy), old)
        self.assertEqual(os.stat(copy).st_mode & 0o777, 0o600)
        # The next round finds it in place.
        res = self.writer()
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("so it stays in place", res.stdout)
        self.assert_written(dev, old)
        self.assertEqual(len(self.copies()), 1)

    def test_moving_survives_a_cut_off_write_anywhere(self):
        """After a cut at any write, running again ends with the old settings in place, and the copy saved
        before the first write holds them too."""
        old = data_fs(1 << 20, fill=b"\x22")
        for cut in range(self.MOVE_WRITES):
            with self.subTest(cut=cut):
                for path in self.copies():
                    os.unlink(path)
                dev = self.old_stick(old)
                table = self.contents(dev)[:34 * 512]
                self.cut = cut
                res = self.writer()
                self.assertEqual(res.returncode, 1, res.stdout)
                self.assertIn("the write was cut off (test)", res.stdout)
                if cut < self.MOVE_WRITES - 1:
                    self.assertEqual(self.contents(dev)[:34 * 512], table, "the main table is written last")
                [copy] = self.copies()
                self.assertIn(f"Your saved settings are in {copy}: write again with --data-from", res.stdout)
                self.assertEqual(self.contents(copy), old)
                self.cut = None
                res = self.writer()
                self.assertEqual(res.returncode, 0, res.stdout)
                # Cut off before partition 4's superblock was whole: still moved from where it was;
                # after: already in place.
                self.assertIn("moving" if cut < 6 else "so it stays in place", res.stdout)
                self.assert_written(dev, old)

    def test_two_different_data_partitions_stop_after_a_copy(self):
        """A stick written with this layout, then by an older tool (old layout, newer settings): the one at
        START is stale. Neither is chosen silently: the table's one is copied, and nothing is written."""
        stale, newer = data_fs(1 << 20, fill=b"\x22"), data_fs(1 << 20, fill=b"\x33")
        struct.pack_into("<I", newer := bytearray(newer), 1024 + 0x30, 1794355200)   # 2026-11-11 00:00 UTC
        newer = bytes(newer)
        dev = self.old_stick(newer)
        disk = bytearray(self.contents(dev))
        disk[self.START * 512:self.START * 512 + len(stale)] = stale
        write(dev, bytes(disk))
        res = self.writer()
        self.assertEqual(res.returncode, 1, res.stdout)
        [copy] = self.copies()
        self.assertIn("the stick has two different MPFree data partitions", res.stdout)
        self.assertIn("at 3 MiB (last written 2026-11-11 00:00 UTC), which the MPC used last", res.stdout)
        self.assertIn(f"A copy of the second is in {copy}: run with --data-from that file", res.stdout)
        self.assertEqual(self.contents(copy), newer)
        self.assertEqual(self.contents(dev), bytes(disk), "nothing written on the stick")
        res = self.writer("WRITE\n", "--data-from", copy)
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assert_written(dev, newer)

    def test_a_stick_without_one_starts_empty(self):
        dev = self.disk("sdb", "8:16", self.STICK)
        res = self.writer()
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("the stick has no MPFree data partition, so the saved settings start empty", res.stdout)
        self.assert_written(dev, self.NEW)
        self.assertEqual(self.copies(), [])

    def test_fresh_data_starts_empty(self):
        dev = self.old_stick(data_fs(2 << 20, fill=b"\x22"), first=self.START)
        res = self.writer("WRITE\n", "--fresh-data")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("Partition 4 starts empty (--fresh-data)", res.stdout)
        self.assert_written(dev, self.NEW)
        self.assertEqual(self.copies(), [])

    def test_data_from_a_saved_copy(self):
        dev = self.disk("sdb", "8:16", self.STICK)
        saved = os.path.join(self.root, "saved.img")
        write(saved, data_fs(2 << 20, fill=b"\x33"))
        res = self.writer("WRITE\n", "--data-from", saved)
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn(f"Partition 4 comes from {saved} (--data-from).", res.stdout)
        self.assert_written(dev, self.contents(saved))
        # A copy whose filesystem is smaller than the partition it came from goes back too.
        write(saved, data_fs(1 << 20, fill=b"\x44") + bytes(1 << 20))
        res = self.writer("WRITE\n", "--data-from", saved)
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assert_written(dev, self.contents(saved))
        for bad in (data_fs(2 << 20)[:1 << 20], data_fs(1 << 20, label=b"SOMETHING")):
            with self.subTest(len(bad)):
                write(saved, bad)
                res = self.writer("WRITE\n", "--data-from", saved)
                self.assertEqual(res.returncode, 1, res.stdout)
                self.assertIn("is not a whole MPFREE-DATA partition copy of at most", res.stdout)

    def test_a_partition_4_that_is_not_ours_stops_before_writing(self):
        cases = ((data_fs(1 << 20, label=b"SOMETHING"), self.OLD_START, "is not an MPFREE-DATA ext4"),
                 (data_fs(1 << 20, label=b"SOMETHING"), self.START, "is not an MPFREE-DATA ext4"),
                 (data_fs(12 << 20, fill=b"\x22"), self.OLD_START, "bigger than the room for it"))
        for data, first, problem in cases:
            with self.subTest(problem, first=first):
                dev = self.old_stick(data, first=first)
                before = self.contents(dev)
                res = self.writer()
                self.assertEqual(res.returncode, 1, res.stdout)
                self.assertIn(problem, res.stdout)
                self.assertIn("Run with --fresh-data", res.stdout)
                self.assertEqual(self.contents(dev), before)
                self.assertEqual(self.copies(), [])

    def test_a_stick_too_small_for_the_layout_is_not_written(self):
        dev = self.disk("sdb", "8:16", (6 * MIB >> 9) - 2048)
        res = self.writer()
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("smaller than stick.img needs", res.stdout)
        self.assertEqual(self.contents(dev), b"\xee" * ((6 * MIB) - MIB))

    def test_a_real_ext4_with_a_checksum(self):
        made = real_ext4(os.path.join(self.root, "real.img"), 2 << 20)
        if made is None:
            self.skipTest("needs mke2fs")
        tool = self.tool()
        self.assertTrue(struct.unpack_from("<I", made, 1024 + 0x64)[0] & 0x400, "metadata_csum")
        self.assertTrue(tool.is_data_head(made[:4096]))
        self.assertEqual(tool.fs_bytes(made[:4096]), len(made))
        # Moved from the old layout, it is still a clean filesystem.
        dev = self.old_stick(made)
        res = self.writer()
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assert_written(dev, made)
        moved = os.path.join(self.root, "moved.img")
        write(moved, self.contents(dev)[self.START * 512:self.START * 512 + len(made)])
        fsck = subprocess.run(["e2fsck", "-fn", moved], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
                              env={**os.environ, "PATH": SBIN_PATH})
        self.assertEqual(fsck.returncode, 0, fsck.stdout)
        # A superblock half written (one of its sectors old) is not taken for MPFree's.
        for offset in (1024 + 0x100, 1024 + 0x300):
            torn = bytearray(made[:4096])
            torn[offset] ^= 0xFF
            self.assertFalse(tool.is_data_head(bytes(torn)), offset)
        torn = bytearray(made)
        torn[1024 + 0x300] ^= 0xFF
        dev = self.old_stick(bytes(torn), first=self.START)
        before = self.contents(dev)
        res = self.writer()
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("is not an MPFREE-DATA ext4", res.stdout)
        self.assertEqual(self.contents(dev), before)

    def test_an_image_without_a_data_partition_has_nothing_to_keep(self):
        write(os.path.join(self.folder, "stick.img"), IMAGE)
        write(os.path.join(self.folder, "SHA256SUMS"), f"{hashlib.sha256(IMAGE).hexdigest()}  stick.img\n")
        self.disk("sdb", "8:16", self.STICK)
        res = self.writer("WRITE\n", "--fresh-data")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("has no data partition (built without --app)", res.stdout)

    # The plain-image tests above do not apply to a GPT image.
    test_writes_the_one_stick_and_reads_it_back = None
    test_anything_but_write_stops = None
    test_refusals = None
    test_two_sticks_stop = None
    test_damaged_image_stops = None
    test_privileged_step_checks_again = None


class StaticTests(unittest.TestCase):
    def test_no_device_name_from_the_user_and_one_write_path(self):
        with open(TOOL) as fh:
            code = fh.read()
        # One place writes the stick (write_at), opened once, exclusively.
        self.assertEqual(code.count("os.write("), 1)
        self.assertEqual(code.count("os.O_RDWR"), 1)
        self.assertIn("os.O_RDWR | os.O_EXCL", code)
        self.assertEqual(code.count("write_at("), 3, "its definition and its two callers in root_write")
        # The only other file it writes: the copy of the old partition 4, new, private, in the build folder.
        self.assertEqual(code.count("os.O_WRONLY"), 1)
        self.assertIn("os.O_WRONLY | os.O_CREAT | os.O_EXCL", code)
        self.assertIn('time.strftime("data-from-old-stick-', code)
        self.assertIn("path = os.path.join(os.path.dirname(image), stamp", code)
        # The cut-off hook for the tests only works under the fake root.
        self.assertIn('CUT_AFTER = int(os.environ.get("MPFREE_TEST_CUT_AFTER", "-1")) if TEST_ROOT else -1', code)
        self.assertNotIn("ssh", code.replace("sha", ""))
        self.assertNotIn("/dev/mmc", code)


if __name__ == "__main__":
    unittest.main(verbosity=2)
