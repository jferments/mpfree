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


def data_fs(size, fill=b"\x11", label=b"MPFREE-DATA"):
    """Bytes that pass for partition 4's ext4: the superblock's magic and label, the rest `fill`."""
    data = bytearray(fill * size)
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


class DataPartitionTests(WriterTests):
    """Partition 4 (saved settings) carried from the stick being erased into the new image."""
    NEW = data_fs(1 << 20)                      # the image's empty one, 1 MiB here (64 MiB on a real stick)

    def setUp(self):
        super().setUp()
        self.folder = os.path.join(self.work, "kexec-stick-20260101T000000Z-abc")
        self.image = gpt_disk(8192, stick_layout(6144, self.NEW))
        write(os.path.join(self.folder, "stick.img"), self.image)
        write(os.path.join(self.folder, "SHA256SUMS"), f"{hashlib.sha256(self.image).hexdigest()}  stick.img\n")

    def old_stick(self, data, first=10240, extra=()):
        dev = self.disk("sdb", "8:16", 16 * MIB >> 9)
        write(dev, gpt_disk(16 * MIB >> 9, stick_layout(first, data, extra), fill=b"\xee"))
        return dev

    def expected(self, data):
        return self.image[:6144 * 512] + data + self.image[6144 * 512 + len(data):]

    def test_the_old_sticks_settings_are_carried_over_and_copied_first(self):
        old = data_fs(1 << 20, fill=b"\x22")
        dev = self.old_stick(old)
        res = self.writer()
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("Partition 4 (your saved settings and mapped buttons) is kept from this stick", res.stdout)
        self.assertIn(f"partition 4: carrying over {len(old)} bytes", res.stdout)
        self.assertIn("they match stick.img with the saved partition 4", res.stdout)
        self.assertEqual(self.contents(dev)[:len(self.image)], self.expected(old))
        [copy] = glob.glob(os.path.join(self.folder, "data-from-old-stick-*.img"))
        self.assertEqual(self.contents(copy), old)
        self.assertEqual(os.stat(copy).st_mode & 0o777, 0o600)

    def test_a_stick_without_one_starts_empty(self):
        dev = self.disk("sdb", "8:16", 16 * MIB >> 9)
        res = self.writer()
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("the stick has no MPFree data partition, so the saved settings start empty", res.stdout)
        self.assertIn("they match stick.img (sha256", res.stdout)
        self.assertEqual(self.contents(dev)[:len(self.image)], self.image)
        self.assertEqual(glob.glob(os.path.join(self.folder, "data-from-old-stick-*")), [])

    def test_fresh_data_starts_empty(self):
        dev = self.old_stick(data_fs(1 << 20, fill=b"\x22"))
        res = self.writer("WRITE\n", "--fresh-data")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("Partition 4 starts empty (--fresh-data)", res.stdout)
        self.assertEqual(self.contents(dev)[:len(self.image)], self.image)

    def test_data_from_a_saved_copy(self):
        dev = self.disk("sdb", "8:16", 16 * MIB >> 9)
        saved = os.path.join(self.root, "saved.img")
        write(saved, data_fs(1 << 20, fill=b"\x33"))
        res = self.writer("WRITE\n", "--data-from", saved)
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertEqual(self.contents(dev)[:len(self.image)], self.expected(self.contents(saved)))
        write(saved, data_fs(1 << 19))
        res = self.writer("WRITE\n", "--data-from", saved)
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("is not a 1048576-byte MPFREE-DATA partition copy", res.stdout)

    def test_a_partition_4_that_is_not_ours_stops_before_writing(self):
        for data, problem in ((data_fs(1 << 19, fill=b"\x22"), "is 1024 sectors, not this build's 2048"),
                              (data_fs(1 << 20, label=b"SOMETHING"), "is not an MPFREE-DATA ext4")):
            with self.subTest(problem):
                dev = self.old_stick(data)
                before = self.contents(dev)
                res = self.writer()
                self.assertEqual(res.returncode, 1, res.stdout)
                self.assertIn(problem, res.stdout)
                self.assertIn("Run with --fresh-data", res.stdout)
                self.assertEqual(self.contents(dev), before)
                shutil.rmtree(os.path.join(self.root, "sys"))
                os.unlink(dev)
                os.makedirs(os.path.join(self.root, "sys/block"))
                self.disk("nvme0n1", "259:0", 512 << 30 >> 9, removable="0", usb=False)

    def test_an_image_without_a_data_partition_has_nothing_to_keep(self):
        write(os.path.join(self.folder, "stick.img"), IMAGE)
        write(os.path.join(self.folder, "SHA256SUMS"), f"{hashlib.sha256(IMAGE).hexdigest()}  stick.img\n")
        self.disk("sdb", "8:16", 16 * MIB >> 9)
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
        self.assertEqual(code.count("os.write("), 1)
        self.assertEqual(code.count("os.O_RDWR"), 1)
        self.assertIn("os.O_EXCL", code)
        # The only other file it writes: the copy of the old partition 4, new, private, in the build folder.
        self.assertEqual(code.count("os.O_WRONLY"), 1)
        self.assertIn("os.O_WRONLY | os.O_CREAT | os.O_EXCL", code)
        self.assertIn("os.path.join(os.path.dirname(image), time.strftime(\"data-from-old-stick-", code)
        self.assertNotIn("ssh", code.replace("sha", ""))
        self.assertNotIn("/dev/mmc", code)


if __name__ == "__main__":
    unittest.main(verbosity=2)
